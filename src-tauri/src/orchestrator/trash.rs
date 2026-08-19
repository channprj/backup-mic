//! The two-phase source retirement flow, manual and automatic.

use backup_core::audit_log::{AuditDurability, AuditEvent, AuditLevel, AuditValue};
use backup_core::deletion::{
    DeletionOutcome, DeletionProposal, NoDeletionFaults, RuleDeletionConfirmation,
    RuleDeletionContext, TrashAdapter,
};
use backup_core::error::CoreError;
use backup_core::events::{ActivityEntry, ActivitySeverity};
use backup_core::source::SourceId;
use backup_core::state::{BackupPhase, CurrentStage, DeletionPhase};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::app_state::{AppState, publish_locked, update_source};
use crate::clock;
use crate::dto::TrashProposalSummaryDto;
use crate::platform::macos::trash::MacTrash;

use super::pipeline::{SourceDeletionOutcome, SourceRunOutcome};
use super::shared::{matched_legacy_transmitter, source_label};

pub fn prepare_trash_for_source(
    app: &AppHandle,
    state: &AppState,
    source_id: &SourceId,
) -> Result<TrashProposalSummaryDto, CoreError> {
    let _guard = state.begin_operation()?;
    let (source_label, destination_summary, transmitter) = {
        let mut runtime = state.runtime.lock();
        let matched = runtime
            .matched
            .get(source_id)
            .cloned()
            .ok_or(CoreError::DeviceRemoved)?;
        let evidence = runtime
            .rule_deletions
            .get(source_id)
            .ok_or(CoreError::DeletionPreflightRefused)?;
        let source_label = crate::dto::safe_label(&matched.authority.source.display_name);
        let destination_summary = format!("{} 백업 폴더", evidence.rule.archive_directory_name);
        let transmitter = matched_legacy_transmitter(&matched);
        update_source(&mut runtime.snapshot, source_id, |snapshot| {
            snapshot.retirement_outcome = DeletionPhase::Preparing;
        });
        runtime.snapshot.current_stage = Some(CurrentStage::SourceRevalidation);
        publish_locked(app, &mut runtime);
        (source_label, destination_summary, transmitter)
    };
    let fields = [("mode", AuditValue::Text("manual"))];
    state.append_audit(
        &AuditEvent {
            occurred_at: clock::local_now(),
            level: AuditLevel::Info,
            code: "retirement.preflight",
            transmitter,
            fields: &fields,
        },
        AuditDurability::SyncData,
    )?;
    let proposal = prepare_rule_trash(state, source_id, OffsetDateTime::now_utc())?;
    let expires_at = proposal
        .expires_at
        .format(&Rfc3339)
        .map_err(|_| CoreError::InvalidRequest)?;
    {
        let mut runtime = state.runtime.lock();
        update_source(&mut runtime.snapshot, source_id, |snapshot| {
            snapshot.retirement_outcome = DeletionPhase::AwaitingConfirmation;
        });
        publish_locked(app, &mut runtime);
    }
    Ok(TrashProposalSummaryDto {
        proposal_id: proposal.proposal_id,
        source_id: source_id.as_str().to_owned(),
        source_label,
        session_count: proposal.session_count,
        file_count: proposal.file_count,
        byte_count: proposal.byte_count,
        destination_summary,
        expires_at,
    })
}

pub fn prepare_rule_trash(
    state: &AppState,
    source_id: &SourceId,
    now: OffsetDateTime,
) -> Result<DeletionProposal, CoreError> {
    let (evidence, current_authority, destination_generation, scan_generation) = {
        let runtime = state.runtime.lock();
        let evidence = runtime
            .rule_deletions
            .get(source_id)
            .cloned()
            .ok_or(CoreError::DeletionPreflightRefused)?;
        let current_authority = runtime
            .matched
            .get(source_id)
            .map(|matched| matched.authority.clone())
            .ok_or(CoreError::DeviceRemoved)?;
        (
            evidence,
            current_authority,
            runtime.destination_generation,
            *runtime.rule_scan_generations.get(source_id).unwrap_or(&0),
        )
    };
    if current_authority != evidence.authority
        || destination_generation != evidence.destination_generation
        || scan_generation != evidence.scan_generation
    {
        return Err(CoreError::ProposalInvalidated);
    }
    let current_rule = state
        .ledger
        .lock()
        .backup_rule(&evidence.rule.id)?
        .ok_or(CoreError::DeletionPreflightRefused)?;
    if current_rule != evidence.rule {
        return Err(CoreError::ProposalInvalidated);
    }
    let context = RuleDeletionContext {
        source_id,
        rule_id: &current_rule.id,
        rule_updated_at: &current_rule.updated_at,
        authority: &current_authority,
        destination_generation,
        scan_generation,
    };
    let deletion_allowed = !state.ledger.lock().deletion_disabled();
    let proposal = {
        let ledger = state.ledger.lock();
        state.proposals.lock().prepare_rule(
            context,
            &current_rule,
            evidence.snapshot,
            now,
            deletion_allowed,
            &ledger,
            &NoDeletionFaults,
        )?
    };
    state.runtime.lock().awaiting_rule_deletion = Some(source_id.clone());
    Ok(proposal)
}

pub fn confirm_rule_trash_with_adapter(
    state: &AppState,
    source_id: &SourceId,
    proposal_id: &str,
    now: OffsetDateTime,
    trash: &dyn TrashAdapter,
    observer: &mut dyn FnMut(),
) -> Result<backup_core::deletion::DeletionReport, CoreError> {
    let (current_authority, destination_generation, scan_generation) = {
        let runtime = state.runtime.lock();
        if runtime.awaiting_rule_deletion.as_ref() != Some(source_id) {
            return Err(CoreError::ProposalInvalidated);
        }
        (
            runtime
                .matched
                .get(source_id)
                .map(|matched| matched.authority.clone())
                .ok_or(CoreError::DeviceRemoved)?,
            runtime.destination_generation,
            *runtime.rule_scan_generations.get(source_id).unwrap_or(&0),
        )
    };
    let current_rule = state
        .ledger
        .lock()
        .backup_rule(&current_authority.source.rule_id)?
        .ok_or(CoreError::DeletionPreflightRefused)?;
    let context = RuleDeletionContext {
        source_id,
        rule_id: &current_rule.id,
        rule_updated_at: &current_rule.updated_at,
        authority: &current_authority,
        destination_generation,
        scan_generation,
    };
    let started_at = clock::now_string();
    let finished_at = clock::now_string();
    let result = {
        let mut ledger = state.ledger.lock();
        state.proposals.lock().confirm_rule_observed(
            proposal_id,
            RuleDeletionConfirmation {
                current_context: context,
                current_rule: &current_rule,
                now,
                started_at: &started_at,
                finished_at: &finished_at,
            },
            &mut ledger,
            trash,
            &NoDeletionFaults,
            observer,
        )
    };
    let mut runtime = state.runtime.lock();
    runtime.awaiting_rule_deletion = None;
    runtime.rule_deletions.remove(source_id);
    result
}

pub fn retire_ready_rule_sources_with_adapter(
    state: &AppState,
    outcomes: &[SourceRunOutcome],
    now: OffsetDateTime,
    trash: &dyn TrashAdapter,
    cancellation: &backup_core::backup::CancellationToken,
) -> Result<Vec<SourceDeletionOutcome>, CoreError> {
    cancellation.check()?;
    let mut retirements = Vec::new();
    for outcome in outcomes.iter().filter(|outcome| outcome.deletion_ready) {
        cancellation.check()?;
        let automatic_trash = state
            .ledger
            .lock()
            .batch_run_evidence(&outcome.batch_run_id)?
            .is_some_and(|run| run.frozen_preferences.automatic_trash);
        if !automatic_trash {
            continue;
        }
        let source_id = outcome.source_id.clone();
        let proposal = match prepare_rule_trash(state, &source_id, now) {
            Ok(proposal) => proposal,
            Err(error) => {
                state.runtime.lock().rule_deletions.remove(&source_id);
                retirements.push(SourceDeletionOutcome {
                    source_id,
                    report: None,
                    error: Some(error.public(None)),
                });
                continue;
            }
        };
        cancellation.check()?;
        let result = confirm_rule_trash_with_adapter(
            state,
            &source_id,
            &proposal.proposal_id,
            now + time::Duration::seconds(1),
            trash,
            &mut || {},
        );
        match result {
            Ok(report) => retirements.push(SourceDeletionOutcome {
                source_id,
                report: Some(report),
                error: None,
            }),
            Err(error) => retirements.push(SourceDeletionOutcome {
                source_id,
                report: None,
                error: Some(error.public(None)),
            }),
        }
    }
    Ok(retirements)
}

pub fn confirm_trash(
    app: &AppHandle,
    state: &AppState,
    proposal_id: &str,
) -> Result<(), CoreError> {
    let _guard = state.begin_operation()?;
    let (source_id, transmitter) = {
        let mut runtime = state.runtime.lock();
        let source_id = runtime
            .awaiting_rule_deletion
            .clone()
            .ok_or(CoreError::ProposalInvalidated)?;
        let transmitter = runtime
            .matched
            .get(&source_id)
            .and_then(matched_legacy_transmitter);
        update_source(&mut runtime.snapshot, &source_id, |snapshot| {
            snapshot.retirement_outcome = DeletionPhase::Revalidating;
        });
        runtime.snapshot.current_stage = Some(CurrentStage::SourceRevalidation);
        publish_locked(app, &mut runtime);
        (source_id, transmitter)
    };
    let app_for_deletion = app.clone();
    let state_for_deletion = state.clone();
    let source_for_deletion = source_id.clone();
    let mut observer = move || {
        let mut runtime = state_for_deletion.runtime.lock();
        runtime.snapshot.current_stage = Some(CurrentStage::Trash);
        update_source(&mut runtime.snapshot, &source_for_deletion, |snapshot| {
            snapshot.retirement_outcome = DeletionPhase::Deleting;
        });
        publish_locked(&app_for_deletion, &mut runtime);
    };
    let fields = [("mode", AuditValue::Text("manual"))];
    state.append_audit(
        &AuditEvent {
            occurred_at: clock::local_now(),
            level: AuditLevel::Info,
            code: "retirement.authorized",
            transmitter,
            fields: &fields,
        },
        AuditDurability::SyncData,
    )?;
    let report = confirm_rule_trash_with_adapter(
        state,
        &source_id,
        proposal_id,
        OffsetDateTime::now_utc(),
        &MacTrash,
        &mut observer,
    )?;
    let mut runtime = state.runtime.lock();
    update_source(&mut runtime.snapshot, &source_id, |snapshot| {
        snapshot.retirement_outcome = match report.outcome {
            DeletionOutcome::Deleted => DeletionPhase::Deleted,
            DeletionOutcome::Refused => DeletionPhase::Refused,
            DeletionOutcome::PartiallyDeleted => DeletionPhase::PartiallyDeleted,
        };
        snapshot.deletion_ready = false;
        if report.outcome != DeletionOutcome::Deleted {
            snapshot.phase = BackupPhase::Error;
        }
    });
    if report.outcome == DeletionOutcome::PartiallyDeleted {
        runtime.snapshot.phase = BackupPhase::Error;
        runtime.snapshot.message_code = "partial_trash".to_owned();
    }
    runtime.snapshot.current_stage = None;
    publish_locked(app, &mut runtime);
    drop(runtime);
    if let Err(error) = state.record_activity(
        app,
        ActivityEntry {
            occurred_at: clock::now_string(),
            code: match report.outcome {
                DeletionOutcome::Deleted => "trash_complete",
                DeletionOutcome::Refused => "trash_refused",
                DeletionOutcome::PartiallyDeleted => "partial_trash",
            }
            .to_owned(),
            source_id: Some(source_id.clone()),
            source_label: source_label(state, &source_id),
            count_value: Some(report.deleted_files),
            byte_value: Some(report.deleted_bytes),
            severity: match report.outcome {
                DeletionOutcome::Deleted => ActivitySeverity::Success,
                DeletionOutcome::Refused => ActivitySeverity::Warning,
                DeletionOutcome::PartiallyDeleted => ActivitySeverity::Error,
            },
        },
    ) {
        state.report_failure(
            "confirm_trash",
            "activity_persistence",
            &error,
            transmitter,
            None,
        );
    }
    let fields = [
        ("files", AuditValue::Unsigned(report.deleted_files)),
        ("bytes", AuditValue::Unsigned(report.deleted_bytes)),
        (
            "mode",
            AuditValue::Text(match report.outcome {
                DeletionOutcome::Deleted => "complete",
                DeletionOutcome::Refused => "refused",
                DeletionOutcome::PartiallyDeleted => "partial",
            }),
        ),
    ];
    state.append_audit(
        &AuditEvent {
            occurred_at: clock::local_now(),
            level: match report.outcome {
                DeletionOutcome::Deleted => AuditLevel::Info,
                DeletionOutcome::Refused => AuditLevel::Warning,
                DeletionOutcome::PartiallyDeleted => AuditLevel::Error,
            },
            code: "retirement.complete",
            transmitter,
            fields: &fields,
        },
        AuditDurability::SyncData,
    )?;
    let body = match report.outcome {
        DeletionOutcome::Deleted => "검증된 원본을 휴지통으로 이동했습니다.",
        DeletionOutcome::Refused => "원본을 휴지통으로 이동하지 못했습니다.",
        DeletionOutcome::PartiallyDeleted => {
            "일부 원본만 휴지통으로 이동했습니다. 상태를 확인해 주세요."
        }
    };
    let _ = app
        .notification()
        .builder()
        .title("Backup Mic")
        .body(body)
        .show();
    Ok(())
}
