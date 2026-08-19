//! Moving a verified session to the Trash as one recoverable item.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::error::CoreError;
use crate::ledger::{Ledger, PendingAdditionalDeletionItem, PendingDeletionItem};

use super::rule_verify::{rule_additional_candidates, rule_recording_candidates};
use super::{
    DeletionFaultPoint, DeletionFaults, DeletionOutcome, DeletionProposal, DeletionReport,
    TrashAdapter,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RuleRetirementTarget {
    Session {
        relative_directory: PathBuf,
        recording_ids: Vec<String>,
        additional_file_ids: Vec<String>,
    },
    RecordingFile {
        relative_path: PathBuf,
        recording_id: String,
    },
    AdditionalFile {
        relative_path: PathBuf,
        additional_file_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuleRetirementPlan {
    pub(super) targets: Vec<RuleRetirementTarget>,
    pub(super) source_root: PathBuf,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_rule_retirement(
    proposal: &DeletionProposal,
    plan: &RuleRetirementPlan,
    started_at: &str,
    finished_at: &str,
    ledger: &mut Ledger,
    trash: &dyn TrashAdapter,
    faults: &dyn DeletionFaults,
) -> Result<DeletionReport, CoreError> {
    let recordings = rule_recording_candidates(&proposal.snapshot);
    let additional = rule_additional_candidates(&proposal.snapshot);
    let run_id = Uuid::new_v4().to_string();
    let pending_items = recordings
        .iter()
        .map(|candidate| PendingDeletionItem {
            recording_id: candidate.recording_id.clone(),
            source_size: candidate.source_size,
        })
        .collect::<Vec<_>>();
    let pending_additional_items = additional
        .iter()
        .map(|candidate| PendingAdditionalDeletionItem {
            additional_file_id: candidate.additional_file_id.clone(),
            source_size: candidate.source_size,
        })
        .collect::<Vec<_>>();
    ledger.begin_deletion_run_with_additional(
        &run_id,
        &proposal.frozen.source_id,
        started_at,
        &pending_items,
        &pending_additional_items,
    )?;
    let total_files = u64::try_from(recordings.len() + additional.len())
        .map_err(|_| CoreError::InvalidRequest)?;
    let mut deleted_files = 0_u64;
    let mut deleted_bytes = 0_u64;
    for (index, target) in plan.targets.iter().enumerate() {
        let absolute_path = plan.source_root.join(rule_target_relative_path(target));
        let move_result = faults
            .check(DeletionFaultPoint::MoveToTrash(index))
            .and_then(|()| trash.move_to_trash(&absolute_path));
        let recording_ids = rule_target_recording_ids(target);
        let additional_file_ids = rule_target_additional_ids(target);
        if move_result.is_err() {
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
            ledger.finish_deletion_run(
                &run_id,
                finished_at,
                if outcome == DeletionOutcome::Refused {
                    "refused"
                } else {
                    "partially_moved_to_trash"
                },
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
        ledger.record_deletion_target_success_with_additional(
            &run_id,
            &recording_ids,
            &additional_file_ids,
            finished_at,
            match target {
                RuleRetirementTarget::Session {
                    relative_directory, ..
                } => Some(relative_directory.as_path()),
                RuleRetirementTarget::RecordingFile { .. }
                | RuleRetirementTarget::AdditionalFile { .. } => None,
            },
        )?;
        let target_recordings = recordings
            .iter()
            .filter(|candidate| recording_ids.contains(&candidate.recording_id.as_str()))
            .collect::<Vec<_>>();
        let target_additional = additional
            .iter()
            .filter(|candidate| {
                additional_file_ids.contains(&candidate.additional_file_id.as_str())
            })
            .collect::<Vec<_>>();
        deleted_files = deleted_files
            .checked_add(
                u64::try_from(target_recordings.len() + target_additional.len())
                    .map_err(|_| CoreError::InvalidRequest)?,
            )
            .ok_or(CoreError::InvalidRequest)?;
        deleted_bytes = target_recordings
            .iter()
            .try_fold(deleted_bytes, |total, candidate| {
                total
                    .checked_add(candidate.source_size)
                    .ok_or(CoreError::InvalidRequest)
            })?;
        deleted_bytes = target_additional
            .iter()
            .try_fold(deleted_bytes, |total, candidate| {
                total
                    .checked_add(candidate.source_size)
                    .ok_or(CoreError::InvalidRequest)
            })?;
    }
    ledger.finish_deletion_run(&run_id, finished_at, "moved_to_trash", None)?;
    Ok(DeletionReport {
        run_id,
        outcome: DeletionOutcome::Deleted,
        deleted_files,
        deleted_bytes,
        remaining_files: 0,
        first_failure_code: None,
    })
}

pub(super) fn rule_target_relative_path(target: &RuleRetirementTarget) -> &Path {
    match target {
        RuleRetirementTarget::Session {
            relative_directory, ..
        } => relative_directory,
        RuleRetirementTarget::RecordingFile { relative_path, .. }
        | RuleRetirementTarget::AdditionalFile { relative_path, .. } => relative_path,
    }
}

pub(super) fn rule_target_recording_ids(target: &RuleRetirementTarget) -> Vec<&str> {
    match target {
        RuleRetirementTarget::Session { recording_ids, .. } => {
            recording_ids.iter().map(String::as_str).collect()
        }
        RuleRetirementTarget::RecordingFile { recording_id, .. } => vec![recording_id],
        RuleRetirementTarget::AdditionalFile { .. } => Vec::new(),
    }
}

pub(super) fn rule_target_additional_ids(target: &RuleRetirementTarget) -> Vec<&str> {
    match target {
        RuleRetirementTarget::Session {
            additional_file_ids,
            ..
        } => additional_file_ids.iter().map(String::as_str).collect(),
        RuleRetirementTarget::AdditionalFile {
            additional_file_id, ..
        } => vec![additional_file_id],
        RuleRetirementTarget::RecordingFile { .. } => Vec::new(),
    }
}
