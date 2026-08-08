use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
    },
    thread::{self, JoinHandle},
};

use backup_core::{
    backup::{
        BackupItemContext, NoCopyFaults, cleanup_owned_partials, ensure_capacity,
        execute_backup_item_observed, progress_for_plans,
    },
    deletion::{
        CompleteDeletionSnapshot, DeletionCandidate, DeletionConfirmation, DeletionContext,
        DeletionOutcome, NoDeletionFaults, ProposalInvalidation,
    },
    destination::{DEFAULT_CAPACITY_RESERVE_BYTES, DestinationPlan, plan_recording},
    error::{CoreError, PublicError},
    events::{ActivityEntry, ActivitySeverity},
    recording::RecordingObservation,
    scanner::{ScanIssue, ScanResult, scan_stable},
    state::{BackupPhase, CurrentStage, DeletionPhase, Progress, Transmitter},
};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::{
    app_state::{AppState, OperationGuard, publish_locked, update_transmitter},
    dto::{DeletionProposalSummaryDto, ProgressDto},
    platform::{device_registry::DeviceRegistry, macos::DiskArbitrationMonitor},
};

struct PreparedTransmitter {
    transmitter: Transmitter,
    source_root: std::path::PathBuf,
    scan: ScanResult,
    plans: Vec<DestinationPlan>,
    required_copy_bytes: u64,
}

pub struct DeviceOrchestrator {
    stop: Arc<AtomicBool>,
    monitor: Option<DiskArbitrationMonitor>,
    thread: Option<JoinHandle<()>>,
}

impl DeviceOrchestrator {
    pub fn start(app: AppHandle, state: AppState) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        let monitor = DiskArbitrationMonitor::start(sender)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("dji-device-events".to_owned())
            .spawn(move || {
                let mut registry = DeviceRegistry::default();
                let mut backup_pending = false;
                while !thread_stop.load(Ordering::SeqCst) {
                    match receiver.recv_timeout(std::time::Duration::from_millis(250)) {
                        Ok(event) => {
                            for lifecycle in registry.apply(event) {
                                if state.handle_lifecycle(&app, lifecycle).is_some() {
                                    backup_pending = true;
                                }
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    if backup_pending {
                        match start_backup(app.clone(), state.clone()) {
                            Ok(()) => backup_pending = false,
                            Err(CoreError::Busy) => {}
                            Err(error) => {
                                state.set_error(&app, error, None);
                                backup_pending = false;
                            }
                        }
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stop,
            monitor: Some(monitor),
            thread: Some(thread),
        })
    }
}

impl Drop for DeviceOrchestrator {
    fn drop(&mut self) {
        self.monitor.take();
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn start_backup(app: AppHandle, state: AppState) -> Result<(), CoreError> {
    if !state.backup_is_ready() {
        return Err(CoreError::InvalidRequest);
    }
    let guard = state.begin_operation()?;
    state
        .proposals
        .lock()
        .invalidate(ProposalInvalidation::BackupStarted);
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = run_backup(&app, &state, &guard) {
            state.set_error(&app, error, None);
        }
    });
    Ok(())
}

fn run_backup(app: &AppHandle, state: &AppState, guard: &OperationGuard) -> Result<(), CoreError> {
    let (mounted, destination) = {
        let mut runtime = state.runtime.lock();
        runtime.snapshot.phase = BackupPhase::Scanning;
        runtime.snapshot.message_code = "scanning".to_owned();
        runtime.snapshot.error = None;
        for snapshot in &mut runtime.snapshot.transmitters {
            if snapshot.mounted {
                snapshot.phase = BackupPhase::Scanning;
                snapshot.deletion_ready = false;
            }
        }
        publish_locked(app, &mut runtime);
        (
            runtime.mounted.clone().into_iter().collect::<Vec<_>>(),
            runtime.destination.clone(),
        )
    };
    if mounted.is_empty() {
        return Err(CoreError::DeviceRemoved);
    }
    std::fs::create_dir_all(&destination).map_err(CoreError::CopyFailed)?;
    cleanup_owned_partials(&destination)?;

    let local_offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let mut prepared = Vec::new();
    let mut failed_transmitters = HashSet::new();
    let mut last_error: Option<PublicError> = None;
    for (transmitter, mounted) in mounted {
        guard.cancellation.check()?;
        let scan = match scan_stable(
            &mounted.descriptor.mount_root,
            transmitter,
            local_offset,
            &backup_core::clock::SystemClock,
        ) {
            Ok(scan) => scan,
            Err(error) => {
                failed_transmitters.insert(transmitter);
                last_error = Some(error.public(Some(transmitter)));
                mark_transmitter_failed(app, state, transmitter);
                continue;
            }
        };
        let plans = match scan
            .recordings
            .iter()
            .cloned()
            .map(|recording| {
                plan_recording(
                    &mounted.descriptor.mount_root,
                    &destination,
                    transmitter,
                    recording,
                )
            })
            .collect::<Result<Vec<_>, CoreError>>()
        {
            Ok(plans) => plans,
            Err(error) => {
                failed_transmitters.insert(transmitter);
                last_error = Some(error.public(Some(transmitter)));
                mark_transmitter_failed(app, state, transmitter);
                continue;
            }
        };
        let required_copy_bytes = backup_core::destination::required_copy_bytes(&plans)?;
        prepared.push(PreparedTransmitter {
            transmitter,
            source_root: mounted.descriptor.mount_root,
            scan,
            plans,
            required_copy_bytes,
        });
    }
    state
        .proposals
        .lock()
        .invalidate(ProposalInvalidation::NewScanResults);

    let required_copy_bytes = prepared.iter().try_fold(0_u64, |total, prepared| {
        total
            .checked_add(prepared.required_copy_bytes)
            .ok_or(CoreError::InvalidRequest)
    })?;
    {
        let mut runtime = state.runtime.lock();
        runtime.snapshot.phase = BackupPhase::CheckingCapacity;
        runtime.snapshot.message_code = "checking_capacity".to_owned();
        for prepared in &prepared {
            let progress = progress_for_plans(&prepared.plans)?;
            update_transmitter(&mut runtime.snapshot, prepared.transmitter, |snapshot| {
                snapshot.phase = BackupPhase::CheckingCapacity;
                snapshot.progress = ProgressDto::from(&progress);
            });
        }
        publish_locked(app, &mut runtime);
    }
    ensure_capacity(
        &destination,
        required_copy_bytes,
        DEFAULT_CAPACITY_RESERVE_BYTES,
    )?;

    let run_id = Uuid::new_v4().to_string();
    let started_at = now_string();
    let mut ledger = state.ledger.lock();
    ledger.begin_backup_run(&run_id, &started_at, required_copy_bytes)?;
    let mut progress_by_tx = prepared
        .iter()
        .map(|prepared| Ok((prepared.transmitter, progress_for_plans(&prepared.plans)?)))
        .collect::<Result<HashMap<_, _>, CoreError>>()?;
    let mut published_progress = progress_by_tx.clone();
    let mut verified_by_tx = HashMap::new();
    let mut current_ordinal = 0_u64;

    for prepared_tx in &prepared {
        let mut verified = Vec::new();
        for plan in &prepared_tx.plans {
            current_ordinal = current_ordinal.saturating_add(1);
            let progress = progress_by_tx
                .get_mut(&prepared_tx.transmitter)
                .ok_or(CoreError::InvalidRequest)?;
            let app_for_progress = app.clone();
            let state_for_progress = state.clone();
            let transmitter = prepared_tx.transmitter;
            let mut observer = |observed: &Progress, stage: CurrentStage| {
                let mut runtime = state_for_progress.runtime.lock();
                let phase = if stage == CurrentStage::Copy {
                    BackupPhase::Copying
                } else {
                    BackupPhase::Verifying
                };
                runtime.snapshot.phase = phase;
                runtime.snapshot.message_code = "backup_in_progress".to_owned();
                runtime.snapshot.current_stage = Some(stage);
                runtime.snapshot.current_item_ordinal = Some(current_ordinal);
                update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
                    snapshot.phase = phase;
                    snapshot.progress = ProgressDto::from(observed);
                });
                published_progress.insert(transmitter, observed.clone());
                runtime.snapshot.overall_progress =
                    ProgressDto::from(&combine_progress(published_progress.values()));
                publish_locked(&app_for_progress, &mut runtime);
            };
            let verified_recording = execute_backup_item_observed(
                &BackupItemContext {
                    source_root: &prepared_tx.source_root,
                    destination_root: &destination,
                    transmitter: prepared_tx.transmitter,
                    backup_run_id: &run_id,
                    verified_at: &now_string(),
                },
                plan,
                &mut ledger,
                progress,
                &NoCopyFaults,
                &guard.cancellation,
                &mut observer,
            );
            match verified_recording {
                Ok(recording) => verified.push(DeletionCandidate {
                    recording_id: recording.id,
                    source_relative_path: recording.source_relative_path,
                    source_size: recording.source_size,
                    source_mtime_ns: recording.source_mtime_ns,
                    source_sha256: recording.source_sha256,
                    destination_relative_path: recording.destination_relative_path,
                    destination_size: recording.destination_size,
                    destination_sha256: recording.destination_sha256,
                }),
                Err(error) => {
                    failed_transmitters.insert(prepared_tx.transmitter);
                    last_error = Some(error.public(Some(prepared_tx.transmitter)));
                    if guard.cancellation.check().is_err() {
                        break;
                    }
                }
            }
        }
        verified_by_tx.insert(prepared_tx.transmitter, verified);
    }

    let finished_at = now_string();
    let outcome = if failed_transmitters.is_empty() {
        "completed"
    } else {
        "partial_failure"
    };
    ledger.finish_backup_run(
        &run_id,
        &finished_at,
        outcome,
        last_error.as_ref().map(|error| error.message_code.as_str()),
    )?;
    drop(ledger);

    let total_files = prepared.iter().try_fold(0_u64, |total, prepared| {
        total
            .checked_add(
                u64::try_from(prepared.plans.len()).map_err(|_| CoreError::InvalidRequest)?,
            )
            .ok_or(CoreError::InvalidRequest)
    })?;
    let mut runtime = state.runtime.lock();
    for prepared_tx in prepared {
        let verified = verified_by_tx
            .remove(&prepared_tx.transmitter)
            .unwrap_or_default();
        let complete = !failed_transmitters.contains(&prepared_tx.transmitter)
            && verified.len() == prepared_tx.scan.recordings.len()
            && !prepared_tx
                .scan
                .issues
                .contains(&ScanIssue::TransmitterPrefixMismatch);
        let generation = runtime
            .scan_generations
            .entry(prepared_tx.transmitter)
            .or_default();
        *generation = generation.saturating_add(1);
        if complete && !verified.is_empty() {
            runtime.verified.insert(prepared_tx.transmitter, verified);
            runtime.current_source_paths.insert(
                prepared_tx.transmitter,
                source_paths(&prepared_tx.scan.recordings),
            );
        } else {
            runtime.verified.remove(&prepared_tx.transmitter);
            runtime
                .current_source_paths
                .remove(&prepared_tx.transmitter);
        }
        update_transmitter(&mut runtime.snapshot, prepared_tx.transmitter, |snapshot| {
            snapshot.phase = if failed_transmitters.contains(&prepared_tx.transmitter) {
                BackupPhase::PartialFailure
            } else if prepared_tx.required_copy_bytes == 0 {
                BackupPhase::NothingNew
            } else {
                BackupPhase::CompletedDeletionPending
            };
            snapshot.deletion_ready = complete && !prepared_tx.scan.recordings.is_empty();
            snapshot.deletion_phase = DeletionPhase::Inactive;
        });
    }
    runtime.snapshot.phase = if !failed_transmitters.is_empty() {
        BackupPhase::PartialFailure
    } else if total_files == 0 || required_copy_bytes == 0 {
        BackupPhase::NothingNew
    } else {
        BackupPhase::CompletedDeletionPending
    };
    runtime.snapshot.message_code = match runtime.snapshot.phase {
        BackupPhase::PartialFailure => "partial_failure",
        BackupPhase::NothingNew => "nothing_new",
        _ => "backup_complete",
    }
    .to_owned();
    runtime.snapshot.current_stage = None;
    runtime.snapshot.current_item_ordinal = None;
    runtime.snapshot.last_success_at = failed_transmitters
        .is_empty()
        .then_some(finished_at.clone());
    runtime.snapshot.error = last_error;
    runtime.snapshot.overall_progress =
        ProgressDto::from(&combine_progress(progress_by_tx.values()));
    publish_locked(app, &mut runtime);
    drop(runtime);

    let (activity_code, severity) = if failed_transmitters.is_empty() {
        if total_files == 0 || required_copy_bytes == 0 {
            ("nothing_new", ActivitySeverity::Info)
        } else {
            ("backup_complete", ActivitySeverity::Success)
        }
    } else {
        ("partial_failure", ActivitySeverity::Error)
    };
    let _ = state.record_activity(
        app,
        ActivityEntry {
            occurred_at: finished_at,
            code: activity_code.to_owned(),
            transmitter: None,
            count_value: Some(total_files),
            byte_value: Some(required_copy_bytes),
            severity,
        },
    );

    let body = if failed_transmitters.is_empty() {
        if total_files == 0 || required_copy_bytes == 0 {
            "새 녹음이 없습니다."
        } else {
            "모든 녹음의 복사와 SHA-256 검증을 마쳤습니다."
        }
    } else {
        "일부 파일을 백업하지 못했습니다. 원본은 그대로 남아 있습니다."
    };
    let _ = app
        .notification()
        .builder()
        .title("DJI Mic Backup")
        .body(body)
        .show();
    Ok(())
}

pub fn prepare_deletion(
    app: &AppHandle,
    state: &AppState,
    transmitter: Transmitter,
) -> Result<DeletionProposalSummaryDto, CoreError> {
    let _guard = state.begin_operation()?;
    {
        let mut runtime = state.runtime.lock();
        update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
            snapshot.deletion_phase = DeletionPhase::Preparing;
        });
        publish_locked(app, &mut runtime);
    }
    let (mounted, destination, destination_generation, verified) = {
        let runtime = state.runtime.lock();
        (
            runtime
                .mounted
                .get(&transmitter)
                .cloned()
                .ok_or(CoreError::DeviceRemoved)?,
            runtime.destination.clone(),
            runtime.destination_generation,
            runtime
                .verified
                .get(&transmitter)
                .cloned()
                .ok_or(CoreError::DeletionPreflightRefused)?,
        )
    };
    let scan = scan_stable(
        &mounted.descriptor.mount_root,
        transmitter,
        UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC),
        &backup_core::clock::SystemClock,
    )?;
    state
        .proposals
        .lock()
        .invalidate(ProposalInvalidation::NewScanResults);
    let scan_generation = {
        let mut runtime = state.runtime.lock();
        let generation = runtime.scan_generations.entry(transmitter).or_default();
        *generation = generation.saturating_add(1);
        *generation
    };
    let context = DeletionContext {
        transmitter,
        paired_volume_uuid: mounted.descriptor.volume_uuid,
        mount_generation: mounted.descriptor.mount_generation,
        scan_generation,
        destination_generation,
        source_root: mounted.descriptor.mount_root,
        destination_root: destination,
    };
    let summary = state.proposals.lock().prepare(
        CompleteDeletionSnapshot {
            context,
            candidates: verified,
            current_source_paths: source_paths(&scan.recordings),
        },
        state.elapsed(),
        !state.ledger.lock().deletion_disabled(),
        &NoDeletionFaults,
    )?;
    let expires_at = (OffsetDateTime::now_utc()
        + time::Duration::seconds(
            i64::try_from(summary.expires_in_seconds).map_err(|_| CoreError::InvalidRequest)?,
        ))
    .format(&Rfc3339)
    .map_err(|_| CoreError::InvalidRequest)?;
    {
        let mut runtime = state.runtime.lock();
        update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
            snapshot.deletion_phase = DeletionPhase::AwaitingConfirmation;
        });
        publish_locked(app, &mut runtime);
    }
    Ok(DeletionProposalSummaryDto {
        proposal_id: summary.proposal_id,
        transmitter: summary.transmitter,
        file_count: summary.file_count,
        byte_count: summary.byte_count,
        destination_summary: "DJI-Mic-Mini-2S 백업 폴더".to_owned(),
        expires_at,
    })
}

pub fn confirm_deletion(
    app: &AppHandle,
    state: &AppState,
    proposal_id: &str,
) -> Result<(), CoreError> {
    let _guard = state.begin_operation()?;
    let context = {
        let mut runtime = state.runtime.lock();
        let transmitter = runtime
            .snapshot
            .transmitters
            .iter()
            .find(|snapshot| snapshot.deletion_phase == DeletionPhase::AwaitingConfirmation)
            .map(|snapshot| snapshot.transmitter)
            .ok_or(CoreError::ProposalInvalidated)?;
        update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
            snapshot.deletion_phase = DeletionPhase::Revalidating;
        });
        let mounted = runtime
            .mounted
            .get(&transmitter)
            .cloned()
            .ok_or(CoreError::DeviceRemoved)?;
        let context = DeletionContext {
            transmitter,
            paired_volume_uuid: mounted.descriptor.volume_uuid,
            mount_generation: mounted.descriptor.mount_generation,
            scan_generation: *runtime.scan_generations.get(&transmitter).unwrap_or(&0),
            destination_generation: runtime.destination_generation,
            source_root: mounted.descriptor.mount_root,
            destination_root: runtime.destination.clone(),
        };
        publish_locked(app, &mut runtime);
        context
    };
    let app_for_deletion = app.clone();
    let state_for_deletion = state.clone();
    let transmitter = context.transmitter;
    let mut observer = move || {
        let mut runtime = state_for_deletion.runtime.lock();
        update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
            snapshot.deletion_phase = DeletionPhase::Deleting;
        });
        publish_locked(&app_for_deletion, &mut runtime);
    };
    let report = state.proposals.lock().confirm_observed(
        proposal_id,
        DeletionConfirmation {
            current_context: &context,
            now: state.elapsed(),
            started_at: &now_string(),
            finished_at: &now_string(),
        },
        &mut state.ledger.lock(),
        &NoDeletionFaults,
        &mut observer,
    )?;
    let mut runtime = state.runtime.lock();
    update_transmitter(&mut runtime.snapshot, context.transmitter, |snapshot| {
        snapshot.deletion_phase = match report.outcome {
            DeletionOutcome::Deleted => DeletionPhase::Deleted,
            DeletionOutcome::Refused => DeletionPhase::Refused,
            DeletionOutcome::PartiallyDeleted => DeletionPhase::PartiallyDeleted,
        };
        snapshot.deletion_ready = false;
        if report.outcome != DeletionOutcome::Deleted {
            snapshot.phase = BackupPhase::Error;
        }
    });
    runtime.verified.remove(&context.transmitter);
    runtime.current_source_paths.remove(&context.transmitter);
    if report.outcome == DeletionOutcome::PartiallyDeleted {
        runtime.snapshot.phase = BackupPhase::Error;
        runtime.snapshot.message_code = "partial_deletion".to_owned();
    }
    publish_locked(app, &mut runtime);
    drop(runtime);
    let _ = state.record_activity(
        app,
        ActivityEntry {
            occurred_at: now_string(),
            code: match report.outcome {
                DeletionOutcome::Deleted => "deletion_complete",
                DeletionOutcome::Refused => "deletion_refused",
                DeletionOutcome::PartiallyDeleted => "partial_deletion",
            }
            .to_owned(),
            transmitter: Some(context.transmitter),
            count_value: Some(report.deleted_files),
            byte_value: Some(report.deleted_bytes),
            severity: match report.outcome {
                DeletionOutcome::Deleted => ActivitySeverity::Success,
                DeletionOutcome::Refused => ActivitySeverity::Warning,
                DeletionOutcome::PartiallyDeleted => ActivitySeverity::Error,
            },
        },
    );
    let body = match report.outcome {
        DeletionOutcome::Deleted => "검증된 원본 삭제를 완료했습니다.",
        DeletionOutcome::Refused => "원본을 삭제하지 못했습니다.",
        DeletionOutcome::PartiallyDeleted => "일부 원본만 삭제되었습니다. 상태를 확인해 주세요.",
    };
    let _ = app
        .notification()
        .builder()
        .title("DJI Mic Backup")
        .body(body)
        .show();
    Ok(())
}

fn combine_progress<'a>(progresses: impl Iterator<Item = &'a Progress>) -> Progress {
    progresses.fold(Progress::default(), |mut combined, progress| {
        combined.completed_work_units = combined
            .completed_work_units
            .saturating_add(progress.completed_work_units);
        combined.total_work_units = combined
            .total_work_units
            .saturating_add(progress.total_work_units);
        combined.copied_bytes = combined.copied_bytes.saturating_add(progress.copied_bytes);
        combined.bytes_requiring_copy = combined
            .bytes_requiring_copy
            .saturating_add(progress.bytes_requiring_copy);
        combined.verified_files = combined
            .verified_files
            .saturating_add(progress.verified_files);
        combined.total_files = combined.total_files.saturating_add(progress.total_files);
        combined
    })
}

fn mark_transmitter_failed(app: &AppHandle, state: &AppState, transmitter: Transmitter) {
    let mut runtime = state.runtime.lock();
    update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
        snapshot.phase = BackupPhase::PartialFailure;
        snapshot.deletion_ready = false;
        snapshot.deletion_phase = DeletionPhase::Inactive;
    });
    runtime.snapshot.phase = BackupPhase::PartialFailure;
    runtime.snapshot.message_code = "partial_failure".to_owned();
    publish_locked(app, &mut runtime);
}

fn source_paths(recordings: &[RecordingObservation]) -> BTreeSet<std::path::PathBuf> {
    recordings
        .iter()
        .map(|recording| recording.relative_path.clone())
        .collect()
}

pub(crate) fn now_string() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_progress_uses_work_units_instead_of_file_weighting() {
        let tiny = Progress {
            completed_work_units: 2,
            total_work_units: 2,
            copied_bytes: 1,
            bytes_requiring_copy: 1,
            verified_files: 1,
            total_files: 1,
        };
        let large = Progress {
            completed_work_units: 0,
            total_work_units: 198,
            copied_bytes: 0,
            bytes_requiring_copy: 99,
            verified_files: 0,
            total_files: 1,
        };
        let combined = combine_progress([&tiny, &large].into_iter());
        assert_eq!(combined.percent(), 1);
        assert_eq!(combined.total_work_units, 200);
    }
}
