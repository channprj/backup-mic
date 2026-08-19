//! Starting a run and publishing its progress to the popover.

use std::collections::{BTreeSet, HashMap};

use backup_core::audit_log::{AuditDurability, AuditEvent, AuditLevel, AuditValue};
use backup_core::clock::SystemClock;
use backup_core::deletion::{DeletionOutcome, ProposalInvalidation};
use backup_core::error::{CoreError, PublicError};
use backup_core::events::{ActivityEntry, ActivitySeverity};
use backup_core::source::SourceId;
use backup_core::state::{BackupPhase, DeletionPhase};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use time::OffsetDateTime;

use crate::app_state::{AppState, OperationGuard, publish_locked, update_source};
use crate::clock;
use crate::dto::ProgressDto;
use crate::manual_backup::ManualBackupClaim;
use crate::platform::macos::audio::AppleAudioTools;
use crate::platform::macos::trash::MacTrash;

use super::pipeline::{
    NoSourceCopyFaults, overall_backup_phase, run_matched_sources_with_adapters,
};
use super::shared::{adapter_public_error, source_failure_stage, source_label};
use super::trash::retire_ready_rule_sources_with_adapter;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BackupTrigger {
    Automatic,
    Manual,
}

pub fn request_backup(app: AppHandle, state: AppState) -> Result<(), CoreError> {
    state.backup_configuration_preflight()?;
    state.request_manual_backup();
    if state.matched_sources().is_empty() {
        state.mark_manual_backup_waiting();
        publish_waiting_for_device(&app, &state, "backup.waiting_for_device");
        return Ok(());
    }
    match start_backup(app.clone(), state.clone(), BackupTrigger::Manual) {
        Ok(()) | Err(CoreError::Busy) => Ok(()),
        Err(CoreError::DeviceRemoved) => {
            state.mark_manual_backup_waiting();
            publish_waiting_for_device(&app, &state, "backup.waiting_for_device");
            Ok(())
        }
        Err(error) => {
            state.cancel_manual_backup();
            Err(error)
        }
    }
}

pub(crate) fn start_backup(
    app: AppHandle,
    state: AppState,
    trigger: BackupTrigger,
) -> Result<(), CoreError> {
    state.backup_start_preflight()?;
    let guard = state.begin_operation()?;
    let scheduled_sources = state.matched_sources().into_keys().collect::<Vec<_>>();
    if scheduled_sources.is_empty() {
        return Err(CoreError::DeviceRemoved);
    }
    let manual_claim = match trigger {
        BackupTrigger::Automatic => None,
        BackupTrigger::Manual => {
            let Some(claim) = state.claim_manual_backup(&scheduled_sources) else {
                return Err(CoreError::DeviceRemoved);
            };
            Some(claim)
        }
    };
    state
        .proposals
        .lock()
        .invalidate(ProposalInvalidation::BackupStarted);
    if manual_claim
        .as_ref()
        .is_some_and(ManualBackupClaim::resumed)
    {
        append_reconnect_audit(&state, "backup.resumed_after_reconnect");
    }
    tauri::async_runtime::spawn_blocking(move || match run_backup(&app, &state, &guard) {
        Ok(result) => {
            let interrupted = result.should_resume_after_reconnect();
            if result.has_real_failure {
                state.cancel_manual_backup();
            } else if !interrupted {
                if let Some(claim) = manual_claim {
                    state.finish_manual_backup(claim);
                }
            } else {
                let cancelled = guard.cancellation.is_cancelled();
                if let Some(claim) = manual_claim {
                    state.interrupt_manual_backup_claim(
                        claim,
                        result.interrupted_sources,
                        cancelled,
                    );
                } else if !cancelled {
                    state.interrupt_manual_backup(result.interrupted_sources);
                }
            }
            if !result.has_real_failure && state.manual_backup_is_waiting() {
                if interrupted {
                    state.set_waiting_for_device(&app);
                } else {
                    publish_waiting_for_device(&app, &state, "backup.waiting_for_device");
                }
            }
        }
        Err(CoreError::Cancelled) => {
            state.cancel_manual_backup();
            state.settle_cancelled_operation(&app);
        }
        Err(CoreError::DeviceRemoved) => {
            let cancelled = guard.cancellation.is_cancelled();
            if let Some(claim) = manual_claim {
                state.interrupt_manual_backup_claim(claim, scheduled_sources, cancelled);
            } else if !cancelled {
                state.interrupt_manual_backup(scheduled_sources);
            }
            if !cancelled {
                publish_waiting_for_device(&app, &state, "backup.waiting_for_device");
            }
        }
        Err(error) => {
            state.cancel_manual_backup();
            state.report_failure("backup_run", "background_operation", &error, None, None);
            state.set_error(&app, error, None);
        }
    });
    Ok(())
}

pub(super) struct BackupRunResult {
    pub(super) interrupted_sources: BTreeSet<SourceId>,
    pub(super) has_real_failure: bool,
}

impl BackupRunResult {
    pub(super) fn should_resume_after_reconnect(&self) -> bool {
        !self.has_real_failure && !self.interrupted_sources.is_empty()
    }
}

pub(super) fn run_backup(
    app: &AppHandle,
    state: &AppState,
    guard: &OperationGuard,
) -> Result<BackupRunResult, CoreError> {
    let matched = state.matched_sources().into_values().collect::<Vec<_>>();
    if matched.is_empty() {
        return Err(CoreError::DeviceRemoved);
    }
    {
        let mut runtime = state.runtime.lock();
        for source in &matched {
            runtime.rule_deletions.remove(&source.authority.source.id);
        }
        runtime.awaiting_rule_deletion = None;
        runtime.snapshot.phase = BackupPhase::Scanning;
        runtime.snapshot.message_code = "scanning".to_owned();
        runtime.snapshot.error = None;
        runtime.snapshot.failure_stage = None;
        runtime.snapshot.setting_applies_next_run = true;
        for source in &matched {
            update_source(
                &mut runtime.snapshot,
                &source.authority.source.id,
                |snapshot| {
                    snapshot.phase = BackupPhase::Scanning;
                    snapshot.deletion_ready = false;
                    snapshot.retirement_outcome = DeletionPhase::Inactive;
                    snapshot.error = None;
                },
            );
        }
        publish_locked(app, &mut runtime);
    }
    let outcomes = run_matched_sources_with_adapters(
        state,
        &matched,
        &AppleAudioTools,
        &NoSourceCopyFaults,
        &MacTrash,
        &SystemClock,
        &guard.cancellation,
    )?;
    guard.cancellation.check()?;
    let automatic_retirements = retire_ready_rule_sources_with_adapter(
        state,
        &outcomes,
        OffsetDateTime::now_utc(),
        &MacTrash,
        &guard.cancellation,
    )?;
    let automatic_retirement_failed = automatic_retirements.iter().any(|retirement| {
        retirement.error.is_some()
            || retirement.report.as_ref().is_some_and(|report| {
                matches!(
                    report.outcome,
                    DeletionOutcome::Refused | DeletionOutcome::PartiallyDeleted
                )
            })
    });
    let interrupted_sources = outcomes
        .iter()
        .filter(|outcome| outcome.error.as_ref().is_some_and(is_device_removed))
        .map(|outcome| outcome.source_id.clone())
        .collect::<BTreeSet<_>>();
    let non_device_failure = outcomes.iter().any(|outcome| {
        outcome
            .error
            .as_ref()
            .is_some_and(|error| !is_device_removed(error))
    });
    let phase = if automatic_retirement_failed || non_device_failure {
        BackupPhase::PartialFailure
    } else if !interrupted_sources.is_empty() {
        BackupPhase::Detecting
    } else {
        overall_backup_phase(&outcomes)
    };
    let finished_at = clock::now_string();
    let total_files = outcomes.iter().try_fold(0_u64, |total, outcome| {
        total
            .checked_add(outcome.verified_files)
            .ok_or(CoreError::InvalidRequest)
    })?;
    let last_error = outcomes
        .iter()
        .filter(|outcome| !outcome.error.as_ref().is_some_and(is_device_removed))
        .find_map(|outcome| {
            outcome
                .error
                .clone()
                .map(|error| state.public_error_for_source(error, &outcome.source_id))
        })
        .or_else(|| {
            automatic_retirements.iter().find_map(|retirement| {
                retirement
                    .error
                    .clone()
                    .map(|error| state.public_error_for_source(error, &retirement.source_id))
            })
        })
        .or_else(|| automatic_retirement_failed.then(|| CoreError::TrashFailed.public(None)));
    let source_errors = matched
        .iter()
        .filter_map(|matched_source| {
            let source_id = &matched_source.authority.source.id;
            let error = outcomes
                .iter()
                .find(|outcome| outcome.source_id == *source_id)
                .and_then(|outcome| outcome.error.clone())
                .filter(|error| !is_device_removed(error))
                .or_else(|| {
                    automatic_retirements
                        .iter()
                        .find(|retirement| retirement.source_id == *source_id)
                        .and_then(|retirement| {
                            retirement.error.clone().or_else(|| {
                                retirement.report.as_ref().and_then(|report| {
                                    (report.outcome != DeletionOutcome::Deleted)
                                        .then(|| CoreError::TrashFailed.public(None))
                                })
                            })
                        })
                })?;
            Some((
                source_id.clone(),
                state.public_error_for_source(error, source_id),
            ))
        })
        .collect::<HashMap<_, _>>();
    {
        let mut runtime = state.runtime.lock();
        runtime.snapshot.phase = phase;
        runtime.snapshot.message_code = match phase {
            BackupPhase::PartialFailure => "partial_failure",
            BackupPhase::Detecting => "waiting_for_device",
            BackupPhase::NothingNew => "nothing_new",
            _ => "backup_complete",
        }
        .to_owned();
        runtime.snapshot.current_stage = None;
        runtime.snapshot.failure_stage = None;
        runtime.snapshot.setting_applies_next_run = false;
        runtime.snapshot.current_item_ordinal = None;
        runtime.snapshot.error = last_error;
        if matches!(
            phase,
            BackupPhase::CompletedDeletionPending | BackupPhase::NothingNew
        ) {
            runtime.snapshot.last_success_at = Some(finished_at.clone());
        }
        for matched_source in &matched {
            let source_phase = outcomes
                .iter()
                .find(|outcome| outcome.source_id == matched_source.authority.source.id)
                .map(|outcome| outcome.phase)
                .unwrap_or(BackupPhase::PartialFailure);
            let deletion_ready = !interrupted_sources.contains(&matched_source.authority.source.id)
                && outcomes
                    .iter()
                    .find(|outcome| outcome.source_id == matched_source.authority.source.id)
                    .is_some_and(|outcome| outcome.deletion_ready)
                && !automatic_retirements
                    .iter()
                    .any(|retirement| retirement.source_id == matched_source.authority.source.id);
            let retirement = automatic_retirements
                .iter()
                .find(|retirement| retirement.source_id == matched_source.authority.source.id);
            let effective_phase =
                if interrupted_sources.contains(&matched_source.authority.source.id) {
                    BackupPhase::Idle
                } else if retirement.is_some_and(|retirement| {
                    retirement.error.is_some()
                        || retirement
                            .report
                            .as_ref()
                            .is_some_and(|report| report.outcome != DeletionOutcome::Deleted)
                }) {
                    BackupPhase::Error
                } else {
                    source_phase
                };
            let deletion_phase = retirement
                .map(
                    |retirement| match retirement.report.as_ref().map(|report| report.outcome) {
                        Some(DeletionOutcome::Deleted) => DeletionPhase::Deleted,
                        Some(DeletionOutcome::PartiallyDeleted) => DeletionPhase::PartiallyDeleted,
                        Some(DeletionOutcome::Refused) | None => DeletionPhase::Refused,
                    },
                )
                .unwrap_or(DeletionPhase::Inactive);
            let verified_files = outcomes
                .iter()
                .find(|outcome| outcome.source_id == matched_source.authority.source.id)
                .map_or(0, |outcome| outcome.verified_files);
            let source_error = source_errors
                .get(&matched_source.authority.source.id)
                .cloned();
            update_source(
                &mut runtime.snapshot,
                &matched_source.authority.source.id,
                |snapshot| {
                    snapshot.phase = effective_phase;
                    snapshot.progress = ProgressDto {
                        percent: if matches!(
                            effective_phase,
                            BackupPhase::CompletedDeletionPending | BackupPhase::NothingNew
                        ) {
                            100
                        } else {
                            0
                        },
                        copied_bytes: 0,
                        bytes_requiring_copy: 0,
                        verified_files,
                        total_files: verified_files,
                    };
                    snapshot.deletion_ready = deletion_ready;
                    snapshot.retirement_outcome = deletion_phase;
                    snapshot.error = source_error;
                },
            );
        }
        publish_locked(app, &mut runtime);
    }
    for outcome in &outcomes {
        if outcome.error.as_ref().is_some_and(is_device_removed) {
            continue;
        }
        if let Some(error) = &outcome.error {
            state.report_public_failure("backup_run", source_failure_stage(outcome), error, None);
        }
        let (code, severity) = match outcome.phase {
            BackupPhase::PartialFailure | BackupPhase::Error => {
                ("partial_failure", ActivitySeverity::Error)
            }
            BackupPhase::NothingNew => ("nothing_new", ActivitySeverity::Info),
            _ => ("backup_complete", ActivitySeverity::Success),
        };
        if let Err(error) = state.record_activity(
            app,
            ActivityEntry {
                occurred_at: finished_at.clone(),
                code: code.to_owned(),
                source_id: Some(outcome.source_id.clone()),
                source_label: source_label(state, &outcome.source_id),
                count_value: Some(outcome.verified_files),
                byte_value: None,
                severity,
            },
        ) {
            state.report_failure("backup_run", "activity_persistence", &error, None, None);
        }
    }
    for retirement in &automatic_retirements {
        let (code, severity, count, bytes) = match &retirement.report {
            Some(report) if report.outcome == DeletionOutcome::Deleted => (
                "trash_complete",
                ActivitySeverity::Success,
                Some(report.deleted_files),
                Some(report.deleted_bytes),
            ),
            Some(report) if report.outcome == DeletionOutcome::PartiallyDeleted => (
                "partial_trash",
                ActivitySeverity::Error,
                Some(report.deleted_files),
                Some(report.deleted_bytes),
            ),
            Some(report) => (
                "trash_refused",
                ActivitySeverity::Warning,
                Some(report.deleted_files),
                Some(report.deleted_bytes),
            ),
            None => ("trash_refused", ActivitySeverity::Warning, None, None),
        };
        if let Some(error) = retirement
            .error
            .clone()
            .or_else(|| (code != "trash_complete").then(|| CoreError::TrashFailed.public(None)))
        {
            state.report_public_failure("automatic_trash", "source_revalidation", &error, None);
        }
        if let Err(error) = state.record_activity(
            app,
            ActivityEntry {
                occurred_at: finished_at.clone(),
                code: code.to_owned(),
                source_id: Some(retirement.source_id.clone()),
                source_label: source_label(state, &retirement.source_id),
                count_value: count,
                byte_value: bytes,
                severity,
            },
        ) {
            state.report_failure(
                "automatic_trash",
                "activity_persistence",
                &error,
                None,
                None,
            );
        }
    }
    let fields = [
        ("count", AuditValue::Unsigned(total_files)),
        (
            "mode",
            AuditValue::Text(match phase {
                BackupPhase::PartialFailure => "partial_failure",
                BackupPhase::Detecting => "waiting_for_device",
                _ => "completed",
            }),
        ),
    ];
    state.append_audit(
        &AuditEvent {
            occurred_at: clock::local_now(),
            level: if phase == BackupPhase::PartialFailure {
                AuditLevel::Error
            } else {
                AuditLevel::Info
            },
            code: if phase == BackupPhase::Detecting {
                "backup.waiting_for_device"
            } else {
                "backup.run_complete"
            },
            transmitter: None,
            fields: &fields,
        },
        AuditDurability::SyncData,
    )?;
    if phase != BackupPhase::Detecting
        && app
            .notification()
            .builder()
            .title("Backup Mic")
            .body(if phase == BackupPhase::PartialFailure {
                "일부 파일을 백업하지 못했습니다. 원본은 그대로 남아 있습니다."
            } else if phase == BackupPhase::NothingNew {
                "새 녹음이 없습니다."
            } else {
                "모든 녹음의 복사와 SHA-256 검증을 마쳤습니다."
            })
            .show()
            .is_err()
    {
        let error = adapter_public_error("notification_failed", true);
        state.report_public_failure("backup_run", "notification", &error, None);
    }
    Ok(BackupRunResult {
        interrupted_sources,
        has_real_failure: automatic_retirement_failed || non_device_failure,
    })
}

pub(super) fn publish_waiting_for_device(
    app: &AppHandle,
    state: &AppState,
    audit_code: &'static str,
) {
    state.set_waiting_for_device(app);
    append_reconnect_audit(state, audit_code);
}

pub(super) fn append_reconnect_audit(state: &AppState, code: &'static str) {
    if let Err(error) = state.append_audit(
        &AuditEvent {
            occurred_at: clock::local_now(),
            level: AuditLevel::Info,
            code,
            transmitter: None,
            fields: &[],
        },
        AuditDurability::Buffered,
    ) {
        state.report_failure("backup_reconnect", "audit_log", &error, None, None);
    }
}

pub(super) fn is_device_removed(error: &PublicError) -> bool {
    error.message_code == "device_removed"
}
