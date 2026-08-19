//! Issuing and consuming the time-limited deletion authority.
//!
//! A proposal is the only thing that permits an unlink. It is private, expires, is single-use,
//! and is dropped the moment anything it depended on changes.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::CoreError;
use crate::filesystem::is_recognized_session_name;
use crate::ledger::{Ledger, PendingAdditionalDeletionItem, PendingDeletionItem};
use crate::rule::BackupRule;

use super::legacy::{
    target_additional_file_ids, target_contains_additional_file, target_contains_recording,
    target_recording_ids, target_relative_path, verify_complete_snapshot, verify_ledger_authority,
};
use super::rule_execute::execute_rule_retirement;
use super::rule_verify::{
    freeze_rule_context, rule_additional_candidates, rule_context_matches,
    rule_recording_candidates, validate_rule_context, validate_rule_snapshot_shape,
    verify_complete_rule_snapshot, verify_rule_ledger_authority,
};
use super::{
    CompleteDeletionSnapshot, CompleteRuleDeletionSnapshot, DeletionConfirmation,
    DeletionFaultPoint, DeletionFaults, DeletionOutcome, DeletionProposal, DeletionProposalStore,
    DeletionProposalSummary, DeletionReport, PROPOSAL_TTL, PrivateProposal, ProposalInvalidation,
    RetirementTarget, RuleDeletionConfirmation, RuleDeletionContext, TrashAdapter,
};

pub fn propose_rule_deletion(
    context: &RuleDeletionContext<'_>,
    snapshot: CompleteRuleDeletionSnapshot,
    expires_at: OffsetDateTime,
) -> Result<DeletionProposal, CoreError> {
    validate_rule_context(context)?;
    validate_rule_snapshot_shape(&snapshot)?;
    let session_count =
        u64::try_from(snapshot.sessions.len()).map_err(|_| CoreError::InvalidRequest)?;
    let file_count = u64::try_from(rule_recording_candidates(&snapshot).len())
        .map_err(|_| CoreError::InvalidRequest)?
        .checked_add(
            u64::try_from(rule_additional_candidates(&snapshot).len())
                .map_err(|_| CoreError::InvalidRequest)?,
        )
        .ok_or(CoreError::InvalidRequest)?;
    let byte_count =
        rule_recording_candidates(&snapshot)
            .into_iter()
            .try_fold(0_u64, |total, candidate| {
                total
                    .checked_add(candidate.source_size)
                    .ok_or(CoreError::InvalidRequest)
            })?;
    let byte_count = rule_additional_candidates(&snapshot).into_iter().try_fold(
        byte_count,
        |total, candidate| {
            total
                .checked_add(candidate.source_size)
                .ok_or(CoreError::InvalidRequest)
        },
    )?;
    Ok(DeletionProposal {
        proposal_id: Uuid::new_v4().to_string(),
        source_id: context.source_id.clone(),
        session_count,
        file_count,
        byte_count,
        expires_at,
        frozen: freeze_rule_context(context),
        snapshot,
    })
}

impl DeletionProposalStore {
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_rule(
        &mut self,
        context: RuleDeletionContext<'_>,
        current_rule: &BackupRule,
        snapshot: CompleteRuleDeletionSnapshot,
        now: OffsetDateTime,
        deletion_allowed: bool,
        ledger: &Ledger,
        faults: &dyn DeletionFaults,
    ) -> Result<DeletionProposal, CoreError> {
        self.invalidate(ProposalInvalidation::AnotherDeletionAttempt);
        if !deletion_allowed
            || !current_rule.enabled
            || current_rule.archived_at.is_some()
            || current_rule.id != *context.rule_id
            || current_rule.updated_at != context.rule_updated_at
        {
            return Err(CoreError::DeletionPreflightRefused);
        }
        let expires_at = now
            .checked_add(time::Duration::seconds(
                i64::try_from(PROPOSAL_TTL.as_secs()).map_err(|_| CoreError::InvalidRequest)?,
            ))
            .ok_or(CoreError::InvalidRequest)?;
        let proposal = propose_rule_deletion(&context, snapshot, expires_at)?;
        verify_rule_ledger_authority(ledger, &proposal)?;
        verify_complete_rule_snapshot(&proposal, current_rule, faults)?;
        self.rule_proposals
            .insert(proposal.proposal_id.clone(), proposal.clone());
        Ok(proposal)
    }

    pub fn confirm_rule(
        &mut self,
        proposal_id: &str,
        confirmation: RuleDeletionConfirmation<'_>,
        ledger: &mut Ledger,
        trash: &dyn TrashAdapter,
        faults: &dyn DeletionFaults,
    ) -> Result<DeletionReport, CoreError> {
        self.confirm_rule_observed(proposal_id, confirmation, ledger, trash, faults, &mut || {})
    }

    pub fn confirm_rule_observed(
        &mut self,
        proposal_id: &str,
        confirmation: RuleDeletionConfirmation<'_>,
        ledger: &mut Ledger,
        trash: &dyn TrashAdapter,
        faults: &dyn DeletionFaults,
        observer: &mut dyn FnMut(),
    ) -> Result<DeletionReport, CoreError> {
        let Some(proposal) = self.rule_proposals.remove(proposal_id) else {
            self.invalidate(ProposalInvalidation::AnotherDeletionAttempt);
            return Err(CoreError::ProposalInvalidated);
        };
        self.invalidate(ProposalInvalidation::AnotherDeletionAttempt);
        if confirmation.now >= proposal.expires_at {
            return Err(CoreError::ProposalExpired);
        }
        if !rule_context_matches(&proposal.frozen, &confirmation.current_context)
            || !confirmation.current_rule.enabled
            || confirmation.current_rule.archived_at.is_some()
            || confirmation.current_rule.id != proposal.frozen.rule_id
            || confirmation.current_rule.updated_at != proposal.frozen.rule_updated_at
        {
            return Err(CoreError::ProposalInvalidated);
        }
        let stored_rule = ledger
            .backup_rule(&proposal.frozen.rule_id)?
            .ok_or(CoreError::DeletionPreflightRefused)?;
        if stored_rule != *confirmation.current_rule {
            return Err(CoreError::DeletionPreflightRefused);
        }
        let stored_source = ledger
            .source(&proposal.frozen.source_id)
            .map_err(|_| CoreError::DeletionPreflightRefused)?;
        if stored_source != proposal.frozen.authority.source {
            return Err(CoreError::DeletionPreflightRefused);
        }
        verify_rule_ledger_authority(ledger, &proposal)?;
        let plan = verify_complete_rule_snapshot(&proposal, confirmation.current_rule, faults)?;
        observer();
        execute_rule_retirement(
            &proposal,
            &plan,
            confirmation.started_at,
            confirmation.finished_at,
            ledger,
            trash,
            faults,
        )
    }

    pub fn prepare(
        &mut self,
        snapshot: CompleteDeletionSnapshot,
        now: Duration,
        deletion_allowed: bool,
        faults: &dyn DeletionFaults,
    ) -> Result<DeletionProposalSummary, CoreError> {
        self.invalidate(ProposalInvalidation::AnotherDeletionAttempt);
        if !deletion_allowed
            || snapshot.candidates.is_empty()
            || snapshot.m4a_barrier_run_id.is_empty()
        {
            return Err(CoreError::DeletionPreflightRefused);
        }
        let candidate_ids: BTreeSet<&str> = snapshot
            .candidates
            .iter()
            .map(|candidate| candidate.recording_id.as_str())
            .collect();
        let candidate_paths: BTreeSet<PathBuf> = snapshot
            .candidates
            .iter()
            .map(|candidate| candidate.source_relative_path.clone())
            .collect();
        let additional_ids: BTreeSet<&str> = snapshot
            .additional_files
            .iter()
            .map(|candidate| candidate.additional_file_id.as_str())
            .collect();
        let additional_paths: BTreeSet<PathBuf> = snapshot
            .additional_files
            .iter()
            .map(|candidate| candidate.source_relative_path.clone())
            .collect();
        let all_paths = candidate_paths
            .union(&additional_paths)
            .cloned()
            .collect::<BTreeSet<_>>();
        if candidate_ids.len() != snapshot.candidates.len()
            || candidate_paths.len() != snapshot.candidates.len()
            || additional_ids.len() != snapshot.additional_files.len()
            || additional_paths.len() != snapshot.additional_files.len()
            || all_paths.len() != snapshot.candidates.len() + snapshot.additional_files.len()
            || all_paths != snapshot.current_source_paths
        {
            return Err(CoreError::DeletionPreflightRefused);
        }
        let _ = verify_complete_snapshot(
            &snapshot.context,
            &snapshot.candidates,
            &snapshot.additional_files,
            faults,
        )
        .map_err(|_| CoreError::DeletionPreflightRefused)?;
        let byte_count = snapshot
            .candidates
            .iter()
            .try_fold(0_u64, |total, candidate| {
                total
                    .checked_add(candidate.source_size)
                    .ok_or(CoreError::DeletionPreflightRefused)
            })?;
        let byte_count =
            snapshot
                .additional_files
                .iter()
                .try_fold(byte_count, |total, candidate| {
                    total
                        .checked_add(candidate.source_size)
                        .ok_or(CoreError::DeletionPreflightRefused)
                })?;
        let file_count = u64::try_from(snapshot.candidates.len() + snapshot.additional_files.len())
            .map_err(|_| CoreError::InvalidRequest)?;
        let session_count = u64::try_from(
            snapshot
                .candidates
                .iter()
                .filter_map(|candidate| candidate.source_relative_path.parent())
                .filter(|parent| {
                    parent.components().count() == 1 && is_recognized_session_name(parent)
                })
                .collect::<BTreeSet<_>>()
                .len(),
        )
        .map_err(|_| CoreError::InvalidRequest)?;
        let expires_at = now
            .checked_add(PROPOSAL_TTL)
            .ok_or(CoreError::InvalidRequest)?;
        let proposal_id = Uuid::new_v4().to_string();
        let transmitter = snapshot.context.transmitter;
        self.proposals.insert(
            proposal_id.clone(),
            PrivateProposal {
                context: snapshot.context,
                candidates: snapshot.candidates,
                additional_files: snapshot.additional_files,
                m4a_barrier_run_id: snapshot.m4a_barrier_run_id,
                expires_at,
            },
        );
        Ok(DeletionProposalSummary {
            proposal_id,
            transmitter,
            session_count,
            file_count,
            byte_count,
            expires_in_seconds: PROPOSAL_TTL.as_secs(),
        })
    }

    pub fn confirm(
        &mut self,
        proposal_id: &str,
        confirmation: DeletionConfirmation<'_>,
        ledger: &mut Ledger,
        trash: &dyn TrashAdapter,
        faults: &dyn DeletionFaults,
    ) -> Result<DeletionReport, CoreError> {
        self.confirm_observed(proposal_id, confirmation, ledger, trash, faults, &mut || {})
    }

    pub fn confirm_observed(
        &mut self,
        proposal_id: &str,
        confirmation: DeletionConfirmation<'_>,
        ledger: &mut Ledger,
        trash: &dyn TrashAdapter,
        faults: &dyn DeletionFaults,
        observer: &mut dyn FnMut(),
    ) -> Result<DeletionReport, CoreError> {
        let Some(proposal) = self.proposals.remove(proposal_id) else {
            self.invalidate(ProposalInvalidation::AnotherDeletionAttempt);
            return Err(CoreError::ProposalInvalidated);
        };
        self.invalidate(ProposalInvalidation::AnotherDeletionAttempt);
        if confirmation.now >= proposal.expires_at {
            return Err(CoreError::ProposalExpired);
        }
        if &proposal.context != confirmation.current_context {
            return Err(CoreError::ProposalInvalidated);
        }
        verify_ledger_authority(ledger, &proposal)
            .map_err(|_| CoreError::DeletionPreflightRefused)?;
        let retirement_plan = verify_complete_snapshot(
            &proposal.context,
            &proposal.candidates,
            &proposal.additional_files,
            faults,
        )
        .map_err(|_| CoreError::DeletionPreflightRefused)?;
        observer();

        let run_id = Uuid::new_v4().to_string();
        let pending_items: Vec<PendingDeletionItem> = proposal
            .candidates
            .iter()
            .map(|candidate| PendingDeletionItem {
                recording_id: candidate.recording_id.clone(),
                source_size: candidate.source_size,
            })
            .collect();
        let pending_additional_items: Vec<PendingAdditionalDeletionItem> = proposal
            .additional_files
            .iter()
            .map(|candidate| PendingAdditionalDeletionItem {
                additional_file_id: candidate.additional_file_id.clone(),
                source_size: candidate.source_size,
            })
            .collect();
        ledger.begin_deletion_run_with_additional(
            &run_id,
            &proposal.context.source_id,
            confirmation.started_at,
            &pending_items,
            &pending_additional_items,
        )?;

        let total_files =
            u64::try_from(proposal.candidates.len() + proposal.additional_files.len())
                .map_err(|_| CoreError::InvalidRequest)?;
        let mut deleted_files = 0_u64;
        let mut deleted_bytes = 0_u64;
        for (index, target) in retirement_plan.targets.iter().enumerate() {
            let relative_path = target_relative_path(target);
            let absolute_path = proposal.context.source_root.join(relative_path);
            let move_result = faults
                .check(DeletionFaultPoint::MoveToTrash(index))
                .and_then(|()| trash.move_to_trash(&absolute_path));
            if move_result.is_err() {
                let recording_ids = target_recording_ids(target);
                let additional_file_ids = target_additional_file_ids(target);
                ledger.record_deletion_target_failure_with_additional(
                    &run_id,
                    &recording_ids,
                    &additional_file_ids,
                    "trash_failed",
                )?;
                let outcome = if deleted_files == 0 {
                    DeletionOutcome::Refused
                } else {
                    DeletionOutcome::PartiallyDeleted
                };
                let outcome_name = if outcome == DeletionOutcome::Refused {
                    "refused"
                } else {
                    "partially_moved_to_trash"
                };
                ledger.finish_deletion_run(
                    &run_id,
                    confirmation.finished_at,
                    outcome_name,
                    Some("trash_failed"),
                )?;
                return Ok(DeletionReport {
                    run_id,
                    outcome,
                    deleted_files,
                    deleted_bytes,
                    remaining_files: total_files.saturating_sub(deleted_files),
                    first_failure_code: Some("trash_failed".to_owned()),
                });
            }
            let target_candidates = proposal
                .candidates
                .iter()
                .filter(|candidate| target_contains_recording(target, &candidate.recording_id))
                .collect::<Vec<_>>();
            let recording_ids = target_candidates
                .iter()
                .map(|candidate| candidate.recording_id.as_str())
                .collect::<Vec<_>>();
            let target_additional = proposal
                .additional_files
                .iter()
                .filter(|candidate| {
                    target_contains_additional_file(target, &candidate.additional_file_id)
                })
                .collect::<Vec<_>>();
            let additional_file_ids = target_additional
                .iter()
                .map(|candidate| candidate.additional_file_id.as_str())
                .collect::<Vec<_>>();
            ledger.record_deletion_target_success_with_additional(
                &run_id,
                &recording_ids,
                &additional_file_ids,
                confirmation.finished_at,
                match target {
                    RetirementTarget::Session {
                        relative_directory, ..
                    } => Some(relative_directory.as_path()),
                    RetirementTarget::RootFile { .. } => None,
                },
            )?;
            let target_files = u64::try_from(target_candidates.len() + target_additional.len())
                .map_err(|_| CoreError::InvalidRequest)?;
            let target_bytes = target_candidates
                .iter()
                .try_fold(0_u64, |total, candidate| {
                    total
                        .checked_add(candidate.source_size)
                        .ok_or(CoreError::InvalidRequest)
                })?;
            let target_bytes =
                target_additional
                    .iter()
                    .try_fold(target_bytes, |total, candidate| {
                        total
                            .checked_add(candidate.source_size)
                            .ok_or(CoreError::InvalidRequest)
                    })?;
            deleted_files = deleted_files
                .checked_add(target_files)
                .ok_or(CoreError::InvalidRequest)?;
            deleted_bytes = deleted_bytes
                .checked_add(target_bytes)
                .ok_or(CoreError::InvalidRequest)?;
        }
        ledger.finish_deletion_run(&run_id, confirmation.finished_at, "moved_to_trash", None)?;
        Ok(DeletionReport {
            run_id,
            outcome: DeletionOutcome::Deleted,
            deleted_files,
            deleted_bytes,
            remaining_files: 0,
            first_failure_code: None,
        })
    }

    pub fn invalidate(&mut self, _reason: ProposalInvalidation) {
        self.proposals.clear();
        self.rule_proposals.clear();
    }
}
