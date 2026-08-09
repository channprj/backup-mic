use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, RecvTimeoutError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use backup_core::{
    artifact::OutputFormat,
    audit_log::{AuditDurability, AuditEvent, AuditLevel, AuditValue},
    backup::{
        BackupItemContext, NoCopyFaults, cleanup_owned_partials, ensure_capacity,
        execute_additional_file_copy, prepare_backup_item_observed, progress_for_plans,
    },
    batch::{BatchItemKey, BatchPhase, CopyBarrier, M4A_PROFILE_ID},
    deletion::{
        AdditionalDeletionCandidate, CompleteDeletionSnapshot, DeletionCandidate,
        DeletionConfirmation, DeletionContext, DeletionOutcome, NoDeletionFaults,
        ProposalInvalidation, TrashAdapter, reconcile_legacy_empty_sessions,
    },
    destination::{
        AdditionalFilePlan, DEFAULT_CAPACITY_RESERVE_BYTES, DestinationDisposition,
        DestinationPlan, plan_additional_file, plan_recording, required_additional_copy_bytes,
    },
    error::{CoreError, PublicError, PublicErrorCode},
    events::{ActivityEntry, ActivitySeverity},
    hash::hash_file,
    scanner::{ScanIssue, ScanResult, metadata_fingerprint, scan_stable},
    state::{BackupPhase, CurrentStage, DeletionPhase, Progress, Transmitter},
};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::{
    app_state::{AppState, OperationGuard, publish_locked, update_transmitter},
    artifact_pipeline::{
        finalize_prepared_m4a, order_conversion_cohort, prepare_m4a, verify_published_artifact,
    },
    dto::{ProgressDto, TrashProposalSummaryDto},
    platform::{
        device_registry::DeviceRegistry,
        macos::{DiskArbitrationMonitor, audio::AppleAudioTools, trash::MacTrash},
    },
    rescan::{RescanDecision, RescanScheduler},
};

const RESCAN_INTERVAL: Duration = Duration::from_secs(15);

struct PreparedTransmitter {
    transmitter: Transmitter,
    source_root: std::path::PathBuf,
    scan: ScanResult,
    plans: Vec<DestinationPlan>,
    additional_plans: Vec<AdditionalFilePlan>,
    existing_artifacts: HashMap<std::path::PathBuf, backup_core::ledger::VerifiedRecording>,
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
                let mut scheduler = RescanScheduler::new(RESCAN_INTERVAL);
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
                        Err(RecvTimeoutError::Disconnected) => {
                            let error = adapter_public_error("device_monitor_disconnected", true);
                            state.report_public_failure(
                                "device_lifecycle",
                                "event_channel",
                                &error,
                                None,
                            );
                            break;
                        }
                    }
                    let now = Instant::now();
                    let mounted_roots = state.mounted_roots();
                    for transmitter in [Transmitter::Tx01, Transmitter::Tx02] {
                        if !mounted_roots.contains_key(&transmitter)
                            && scheduler.is_mounted(transmitter)
                        {
                            scheduler.unmount(transmitter);
                        }
                    }
                    let local_offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
                    for (transmitter, root) in &mounted_roots {
                        if !scheduler.is_mounted(*transmitter) {
                            match metadata_fingerprint(root, *transmitter, local_offset) {
                                Ok(fingerprint) => {
                                    scheduler.mount(*transmitter, fingerprint, now);
                                }
                                Err(error) => {
                                    state.report_failure(
                                        "device_lifecycle",
                                        "initial_metadata_scan",
                                        &error,
                                        Some(*transmitter),
                                        None,
                                    );
                                    state.set_error(&app, error, Some(*transmitter));
                                }
                            }
                        }
                    }
                    for transmitter in scheduler.due_transmitters(now) {
                        if !state.automatic_backup_enabled() {
                            scheduler.defer(transmitter, now);
                            continue;
                        }
                        let Some(root) = mounted_roots.get(&transmitter) else {
                            scheduler.unmount(transmitter);
                            continue;
                        };
                        match metadata_fingerprint(root, transmitter, local_offset) {
                            Ok(fingerprint) => {
                                if matches!(
                                    scheduler.observe(
                                        transmitter,
                                        fingerprint,
                                        now,
                                        state.operation_is_active(),
                                    ),
                                    RescanDecision::RequestBackup | RescanDecision::KeepPending
                                ) {
                                    backup_pending = true;
                                }
                            }
                            Err(error) => {
                                scheduler.defer(transmitter, now);
                                state.report_failure(
                                    "automatic_rescan",
                                    "metadata_scan",
                                    &error,
                                    Some(transmitter),
                                    None,
                                );
                                state.set_error(&app, error, Some(transmitter));
                            }
                        }
                    }
                    if mounted_roots.is_empty() {
                        backup_pending = false;
                    }
                    if backup_pending {
                        match start_backup(app.clone(), state.clone()) {
                            Ok(()) => {
                                scheduler.mark_backup_started();
                                backup_pending = false;
                            }
                            Err(CoreError::Busy) => {}
                            Err(error) => {
                                state.report_failure(
                                    "automatic_backup",
                                    "operation_start",
                                    &error,
                                    None,
                                    None,
                                );
                                state.set_error(&app, error, None);
                                scheduler.mark_backup_started();
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
            state.report_failure("backup_run", "background_operation", &error, None, None);
            state.set_error(&app, error, None);
        }
    });
    Ok(())
}

fn run_backup(app: &AppHandle, state: &AppState, guard: &OperationGuard) -> Result<(), CoreError> {
    let frozen_preferences = state.frozen_preferences();
    let m4a_conversion_enabled = frozen_preferences.m4a_conversion;
    let (mounted, destination) = {
        let mut runtime = state.runtime.lock();
        runtime.snapshot.phase = BackupPhase::Scanning;
        runtime.snapshot.message_code = "scanning".to_owned();
        runtime.snapshot.error = None;
        runtime.snapshot.failure_stage = None;
        runtime.snapshot.setting_applies_next_run = true;
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
        state.append_audit(
            &AuditEvent {
                occurred_at: audit_now(),
                level: AuditLevel::Info,
                code: "scan.started",
                transmitter: Some(transmitter),
                fields: &[],
            },
            AuditDurability::Buffered,
        )?;
        let scan = match scan_stable(
            &mounted.descriptor.mount_root,
            transmitter,
            local_offset,
            &backup_core::clock::SystemClock,
        ) {
            Ok(scan) => scan,
            Err(error) => {
                state.report_failure("backup_run", "scan", &error, Some(transmitter), None);
                let reason = error.public(Some(transmitter)).message_code;
                let fields = [("reason", AuditValue::Text(&reason))];
                state.append_audit(
                    &AuditEvent {
                        occurred_at: audit_now(),
                        level: AuditLevel::Error,
                        code: "scan.refused",
                        transmitter: Some(transmitter),
                        fields: &fields,
                    },
                    AuditDurability::Buffered,
                )?;
                failed_transmitters.insert(transmitter);
                last_error = Some(error.public(Some(transmitter)));
                mark_transmitter_failed(app, state, transmitter);
                continue;
            }
        };
        let fields = [(
            "count",
            AuditValue::Unsigned(
                u64::try_from(scan.recordings.len()).map_err(|_| CoreError::InvalidRequest)?,
            ),
        )];
        state.append_audit(
            &AuditEvent {
                occurred_at: audit_now(),
                level: AuditLevel::Info,
                code: "scan.complete",
                transmitter: Some(transmitter),
                fields: &fields,
            },
            AuditDurability::Buffered,
        )?;
        let mut plans = match scan
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
                state.report_failure(
                    "backup_run",
                    "destination_planning",
                    &error,
                    Some(transmitter),
                    None,
                );
                failed_transmitters.insert(transmitter);
                last_error = Some(error.public(Some(transmitter)));
                mark_transmitter_failed(app, state, transmitter);
                continue;
            }
        };
        let mut existing_artifacts = HashMap::new();
        {
            let ledger = state.ledger.lock();
            for plan in &mut plans {
                if let Some(existing) = ledger.verified_recording_for_source(
                    transmitter,
                    &plan.source.relative_path,
                    plan.source.size,
                    plan.source.modified_nanos,
                    &plan.source_sha256,
                )? && existing.artifact.format == OutputFormat::M4a
                {
                    plan.disposition = DestinationDisposition::Reuse;
                    existing_artifacts.insert(plan.source.relative_path.clone(), existing);
                }
            }
        }
        let additional_plans = match scan
            .additional_files
            .iter()
            .cloned()
            .map(|file| {
                plan_additional_file(
                    &mounted.descriptor.mount_root,
                    &destination,
                    transmitter,
                    file,
                )
            })
            .collect::<Result<Vec<_>, CoreError>>()
        {
            Ok(plans) => plans,
            Err(error) => {
                state.report_failure(
                    "backup_run",
                    "additional_destination_planning",
                    &error,
                    Some(transmitter),
                    None,
                );
                failed_transmitters.insert(transmitter);
                last_error = Some(error.public(Some(transmitter)));
                mark_transmitter_failed(app, state, transmitter);
                continue;
            }
        };
        let required_copy_bytes = backup_core::destination::required_copy_bytes(&plans)?
            .checked_add(required_additional_copy_bytes(&additional_plans)?)
            .ok_or(CoreError::InvalidRequest)?;
        prepared.push(PreparedTransmitter {
            transmitter,
            source_root: mounted.descriptor.mount_root,
            scan,
            plans,
            additional_plans,
            existing_artifacts,
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
    let capacity_bytes = if m4a_conversion_enabled {
        required_copy_bytes
            .checked_mul(2)
            .ok_or(CoreError::InvalidRequest)?
    } else {
        required_copy_bytes
    };
    ensure_capacity(&destination, capacity_bytes, DEFAULT_CAPACITY_RESERVE_BYTES)?;

    let run_id = Uuid::new_v4().to_string();
    let started_at = now_string();
    {
        let mut ledger = state.ledger.lock();
        ledger.begin_batch_run(
            &run_id,
            &started_at,
            required_copy_bytes,
            frozen_preferences,
        )?;
        ledger.advance_batch_phase(&run_id, BatchPhase::Copying)?;
    }
    let expected_items = prepared
        .iter()
        .flat_map(|prepared| {
            prepared
                .plans
                .iter()
                .map(|plan| BatchItemKey::Recording {
                    transmitter: prepared.transmitter,
                    relative_path: plan.source.relative_path.clone(),
                })
                .chain(
                    prepared
                        .additional_plans
                        .iter()
                        .map(|plan| BatchItemKey::Additional {
                            transmitter: prepared.transmitter,
                            relative_path: plan.source.relative_path.clone(),
                        }),
                )
        })
        .collect::<BTreeSet<_>>();
    let mut copy_barrier = CopyBarrier::new(expected_items)?;
    let mut progress_by_tx = prepared
        .iter()
        .map(|prepared| Ok((prepared.transmitter, progress_for_plans(&prepared.plans)?)))
        .collect::<Result<HashMap<_, _>, CoreError>>()?;
    let mut published_progress = progress_by_tx.clone();
    let mut verified_by_tx = HashMap::new();
    let mut verified_additional_by_tx = HashMap::new();
    let mut current_ordinal = 0_u64;

    for prepared_tx in &prepared {
        let mut verified_additional = Vec::new();
        for plan in &prepared_tx.additional_plans {
            let copied = execute_additional_file_copy(
                &BackupItemContext {
                    source_root: &prepared_tx.source_root,
                    destination_root: &destination,
                    transmitter: prepared_tx.transmitter,
                    backup_run_id: &run_id,
                    verified_at: &now_string(),
                },
                &plan.source,
                &plan.relative_destination,
                &guard.cancellation,
            );
            match copied {
                Ok(file) if file.source_sha256 == plan.source_sha256 => {
                    let additional_file_id =
                        state.ledger.lock().commit_verified_additional_file(&file)?;
                    copy_barrier.record_verified(&BatchItemKey::Additional {
                        transmitter: prepared_tx.transmitter,
                        relative_path: plan.source.relative_path.clone(),
                    })?;
                    let source = file
                        .source_relative_path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or(CoreError::InvalidAuditEvent)?;
                    let output = file
                        .artifact_relative_path
                        .to_str()
                        .ok_or(CoreError::InvalidAuditEvent)?;
                    let fields = [
                        ("source", AuditValue::Text(source)),
                        ("output", AuditValue::Text(output)),
                        ("bytes", AuditValue::Unsigned(file.artifact_size)),
                    ];
                    state.append_audit(
                        &AuditEvent {
                            occurred_at: audit_now(),
                            level: AuditLevel::Info,
                            code: "backup.additional_verified",
                            transmitter: Some(prepared_tx.transmitter),
                            fields: &fields,
                        },
                        AuditDurability::Buffered,
                    )?;
                    verified_additional.push(AdditionalDeletionCandidate {
                        additional_file_id,
                        source_relative_path: file.source_relative_path,
                        source_size: file.source_size,
                        source_mtime_ns: file.source_mtime_ns,
                        source_sha256: file.source_sha256,
                        destination_relative_path: file.artifact_relative_path,
                        destination_size: file.artifact_size,
                        destination_sha256: file.artifact_sha256,
                    });
                }
                Ok(_) => {
                    copy_barrier.record_failed(&BatchItemKey::Additional {
                        transmitter: prepared_tx.transmitter,
                        relative_path: plan.source.relative_path.clone(),
                    })?;
                    let error = CoreError::HashMismatch;
                    state.report_failure(
                        "backup_run",
                        "additional_verification",
                        &error,
                        Some(prepared_tx.transmitter),
                        Some(&plan.source.file_name),
                    );
                    failed_transmitters.insert(prepared_tx.transmitter);
                    last_error = Some(error.public(Some(prepared_tx.transmitter)));
                    break;
                }
                Err(error) => {
                    copy_barrier.record_failed(&BatchItemKey::Additional {
                        transmitter: prepared_tx.transmitter,
                        relative_path: plan.source.relative_path.clone(),
                    })?;
                    state.report_failure(
                        "backup_run",
                        "additional_copy_or_verification",
                        &error,
                        Some(prepared_tx.transmitter),
                        Some(&plan.source.file_name),
                    );
                    failed_transmitters.insert(prepared_tx.transmitter);
                    last_error = Some(error.public(Some(prepared_tx.transmitter)));
                    break;
                }
            }
        }
        verified_additional_by_tx.insert(prepared_tx.transmitter, verified_additional);
    }

    for prepared_tx in &prepared {
        let mut verified = Vec::new();
        if failed_transmitters.contains(&prepared_tx.transmitter) {
            verified_by_tx.insert(prepared_tx.transmitter, verified);
            continue;
        }
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
            let verified_recording = if let Some(existing) = prepared_tx
                .existing_artifacts
                .get(&plan.source.relative_path)
            {
                resolve_live_source(&prepared_tx.source_root, &plan.source.relative_path)
                    .and_then(|source_path| {
                        verify_published_artifact(
                            &destination,
                            &source_path,
                            existing,
                            &AppleAudioTools,
                            &guard.cancellation,
                        )
                    })
                    .map(|()| {
                        progress.record_verification(plan.source.size);
                        progress.record_verified_file();
                        observer(progress, CurrentStage::ArtifactVerification);
                        existing.clone()
                    })
            } else {
                prepare_backup_item_observed(
                    &BackupItemContext {
                        source_root: &prepared_tx.source_root,
                        destination_root: &destination,
                        transmitter: prepared_tx.transmitter,
                        backup_run_id: &run_id,
                        verified_at: &now_string(),
                    },
                    plan,
                    progress,
                    &NoCopyFaults,
                    &guard.cancellation,
                    &mut observer,
                )
            };
            match verified_recording {
                Ok(mut recording) => {
                    let reused_m4a = prepared_tx
                        .existing_artifacts
                        .contains_key(&plan.source.relative_path);
                    recording.backup_run_id.clone_from(&run_id);
                    recording.verified_at = now_string();
                    recording.id = state.ledger.lock().commit_verified_recording(&recording)?;
                    if !reused_m4a {
                        progress.record_verified_file();
                        observer(progress, CurrentStage::Sha256Verification);
                    }
                    copy_barrier.record_verified(&BatchItemKey::Recording {
                        transmitter: prepared_tx.transmitter,
                        relative_path: plan.source.relative_path.clone(),
                    })?;
                    let artifact = recording.artifact;
                    let source_name = recording
                        .source_relative_path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or(CoreError::InvalidAuditEvent)?;
                    let output = artifact
                        .relative_path
                        .to_str()
                        .ok_or(CoreError::InvalidAuditEvent)?;
                    let fields = [
                        ("source", AuditValue::Text(source_name)),
                        ("output", AuditValue::Text(output)),
                        ("source_bytes", AuditValue::Unsigned(recording.source_size)),
                        ("output_bytes", AuditValue::Unsigned(artifact.byte_count)),
                        (
                            "format",
                            AuditValue::Text(match artifact.format {
                                OutputFormat::Wav => "wav",
                                OutputFormat::M4a => "m4a",
                            }),
                        ),
                    ];
                    if let Err(error) = state.append_audit(
                        &AuditEvent {
                            occurred_at: audit_now(),
                            level: AuditLevel::Info,
                            code: "backup.verified",
                            transmitter: Some(prepared_tx.transmitter),
                            fields: &fields,
                        },
                        AuditDurability::Buffered,
                    ) {
                        state.report_failure(
                            "backup_run",
                            "audit_log",
                            &error,
                            Some(prepared_tx.transmitter),
                            recording.source_relative_path.to_str(),
                        );
                        failed_transmitters.insert(prepared_tx.transmitter);
                        last_error = Some(error.public(Some(prepared_tx.transmitter)));
                        continue;
                    }
                    if artifact.format == OutputFormat::M4a {
                        let fields = [
                            ("output", AuditValue::Text(output)),
                            ("output_bytes", AuditValue::Unsigned(artifact.byte_count)),
                            ("format", AuditValue::Text("m4a")),
                        ];
                        state.append_audit(
                            &AuditEvent {
                                occurred_at: audit_now(),
                                level: AuditLevel::Info,
                                code: "conversion.complete",
                                transmitter: Some(prepared_tx.transmitter),
                                fields: &fields,
                            },
                            AuditDurability::Buffered,
                        )?;
                    }
                    verified.push(DeletionCandidate {
                        recording_id: recording.id,
                        source_relative_path: recording.source_relative_path,
                        source_size: recording.source_size,
                        source_mtime_ns: recording.source_mtime_ns,
                        source_sha256: recording.source_sha256,
                        destination_relative_path: artifact.relative_path,
                        destination_size: artifact.byte_count,
                        destination_sha256: artifact.sha256,
                    });
                }
                Err(error) => {
                    copy_barrier.record_failed(&BatchItemKey::Recording {
                        transmitter: prepared_tx.transmitter,
                        relative_path: plan.source.relative_path.clone(),
                    })?;
                    state.report_failure(
                        "backup_run",
                        "copy_or_verification",
                        &error,
                        Some(prepared_tx.transmitter),
                        plan.source.relative_path.to_str(),
                    );
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

    if copy_barrier.conversion_allowed() {
        state
            .ledger
            .lock()
            .advance_batch_phase(&run_id, BatchPhase::CopiesVerified)?;
        state.append_audit(
            &AuditEvent {
                occurred_at: audit_now(),
                level: AuditLevel::Info,
                code: "backup.copy_cohort_verified",
                transmitter: None,
                fields: &[],
            },
            AuditDurability::SyncData,
        )?;
    } else {
        for prepared_tx in &prepared {
            failed_transmitters.insert(prepared_tx.transmitter);
        }
    }

    if copy_barrier.conversion_allowed() && m4a_conversion_enabled {
        let current_recording_ids = verified_by_tx
            .values()
            .flatten()
            .map(|candidate| candidate.recording_id.clone())
            .collect::<BTreeSet<_>>();
        let mut cohort_by_id = {
            let ledger = state.ledger.lock();
            ledger
                .historical_wav_recordings()?
                .into_iter()
                .map(|recording| (recording.id.clone(), recording))
                .collect::<HashMap<_, _>>()
        };
        for recording_id in current_recording_ids {
            if let std::collections::hash_map::Entry::Vacant(entry) =
                cohort_by_id.entry(recording_id)
            {
                let recording = state
                    .ledger
                    .lock()
                    .verified_recording(entry.key())?
                    .ok_or(CoreError::LedgerCorrupt)?;
                entry.insert(recording);
            }
        }
        let conversion_cohort = if cohort_by_id.is_empty() {
            Vec::new()
        } else {
            order_conversion_cohort(cohort_by_id.into_values().collect())?
        };
        let recording_ids = conversion_cohort
            .iter()
            .map(|recording| recording.id.clone())
            .collect::<Vec<_>>();
        if !conversion_cohort.is_empty() {
            state
                .ledger
                .lock()
                .begin_conversion_cohort(&run_id, &recording_ids, M4A_PROFILE_ID)?;
        }
        'conversion: for mut recording in conversion_cohort {
            let transmitter = recording.transmitter;
            if recording.artifact.format == OutputFormat::M4a {
                state
                    .ledger
                    .lock()
                    .mark_conversion_item_verified(&run_id, &recording.id)?;
                continue;
            }
            let source_name = recording
                .source_relative_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(CoreError::InvalidAuditEvent)?;
            let fields = [
                ("source", AuditValue::Text(source_name)),
                ("format", AuditValue::Text("m4a")),
            ];
            state.append_audit(
                &AuditEvent {
                    occurred_at: audit_now(),
                    level: AuditLevel::Info,
                    code: "conversion.started",
                    transmitter: Some(transmitter),
                    fields: &fields,
                },
                AuditDurability::Buffered,
            )?;
            let app_for_artifact = app.clone();
            let state_for_artifact = state.clone();
            let mut artifact_observer = move |stage: CurrentStage| {
                let mut runtime = state_for_artifact.runtime.lock();
                runtime.snapshot.phase = BackupPhase::Verifying;
                runtime.snapshot.message_code = match stage {
                    CurrentStage::Conversion => "converting_m4a",
                    CurrentStage::ArtifactVerification => "verifying_m4a",
                    CurrentStage::Copy
                    | CurrentStage::Sha256Verification
                    | CurrentStage::SourceRevalidation
                    | CurrentStage::Trash => "backup_in_progress",
                }
                .to_owned();
                runtime.snapshot.current_stage = Some(stage);
                update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
                    snapshot.phase = BackupPhase::Verifying
                });
                publish_locked(&app_for_artifact, &mut runtime);
            };
            recording.backup_run_id.clone_from(&run_id);
            recording.verified_at = now_string();
            match prepare_m4a(
                &destination,
                &recording,
                &AppleAudioTools,
                &guard.cancellation,
                &mut artifact_observer,
            ) {
                Ok(prepared_artifact) => {
                    {
                        let mut ledger = state.ledger.lock();
                        ledger.replace_verified_artifact_with_superseded_wav(
                            &prepared_artifact.recording,
                            &prepared_artifact.superseded_wav_relative_path,
                            prepared_artifact.superseded_wav_size,
                            &prepared_artifact.superseded_wav_sha256,
                        )?;
                        ledger.mark_conversion_item_verified(
                            &run_id,
                            &prepared_artifact.recording.id,
                        )?;
                    }
                    finalize_prepared_m4a(&prepared_artifact)?;
                    for candidate in verified_by_tx.values_mut().flatten() {
                        if candidate.recording_id == prepared_artifact.recording.id {
                            candidate.destination_relative_path =
                                prepared_artifact.recording.artifact.relative_path.clone();
                            candidate.destination_size =
                                prepared_artifact.recording.artifact.byte_count;
                            candidate.destination_sha256 =
                                prepared_artifact.recording.artifact.sha256.clone();
                        }
                    }
                }
                Err(error) => {
                    state.report_failure(
                        "backup_run",
                        "conversion",
                        &error,
                        Some(transmitter),
                        recording.source_relative_path.to_str(),
                    );
                    last_error = Some(error.public(Some(transmitter)));
                    for prepared_tx in &prepared {
                        failed_transmitters.insert(prepared_tx.transmitter);
                    }
                    break 'conversion;
                }
            }
        }
        if failed_transmitters.is_empty() {
            if !recording_ids.is_empty() {
                state.ledger.lock().commit_m4a_barrier(&run_id)?;
                state.append_audit(
                    &AuditEvent {
                        occurred_at: audit_now(),
                        level: AuditLevel::Info,
                        code: "backup.m4a_cohort_verified",
                        transmitter: None,
                        fields: &[(
                            "count",
                            AuditValue::Unsigned(
                                u64::try_from(recording_ids.len())
                                    .map_err(|_| CoreError::InvalidRequest)?,
                            ),
                        )],
                    },
                    AuditDurability::SyncData,
                )?;
            }
            let pending_superseded = state.ledger.lock().pending_superseded_wavs()?;
            for superseded in pending_superseded {
                let path = destination.join(&superseded.relative_path);
                let metadata = match std::fs::symlink_metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        state
                            .ledger
                            .lock()
                            .clear_superseded_wav_evidence(&superseded.recording_id)?;
                        continue;
                    }
                    Err(error) => {
                        let error = CoreError::CopyFailed(error);
                        state.report_failure(
                            "artifact_migration",
                            "superseded_wav_revalidation",
                            &error,
                            Some(superseded.transmitter),
                            superseded.relative_path.to_str(),
                        );
                        last_error = Some(error.public(Some(superseded.transmitter)));
                        for prepared_tx in &prepared {
                            failed_transmitters.insert(prepared_tx.transmitter);
                        }
                        break;
                    }
                };
                if !metadata.file_type().is_file() {
                    let error = CoreError::DestinationUnavailable;
                    state.report_failure(
                        "artifact_migration",
                        "superseded_wav_revalidation",
                        &error,
                        Some(superseded.transmitter),
                        superseded.relative_path.to_str(),
                    );
                    last_error = Some(error.public(Some(superseded.transmitter)));
                    for prepared_tx in &prepared {
                        failed_transmitters.insert(prepared_tx.transmitter);
                    }
                    break;
                }
                let digest = hash_file(&path)?;
                if digest.size != superseded.byte_count || digest.sha256 != superseded.sha256 {
                    let error = CoreError::HashMismatch;
                    state.report_failure(
                        "artifact_migration",
                        "superseded_wav_revalidation",
                        &error,
                        Some(superseded.transmitter),
                        superseded.relative_path.to_str(),
                    );
                    last_error = Some(error.public(Some(superseded.transmitter)));
                    for prepared_tx in &prepared {
                        failed_transmitters.insert(prepared_tx.transmitter);
                    }
                    break;
                }
                if let Err(error) = MacTrash.move_to_trash(&path) {
                    state.report_failure(
                        "artifact_migration",
                        "trash",
                        &error,
                        Some(superseded.transmitter),
                        superseded.relative_path.to_str(),
                    );
                    last_error = Some(error.public(Some(superseded.transmitter)));
                    for prepared_tx in &prepared {
                        failed_transmitters.insert(prepared_tx.transmitter);
                    }
                    break;
                }
                state
                    .ledger
                    .lock()
                    .clear_superseded_wav_evidence(&superseded.recording_id)?;
            }
        }
    }

    let finished_at = now_string();
    let outcome = if !failed_transmitters.is_empty() {
        "partial_failure"
    } else if !m4a_conversion_enabled {
        "wav_backup_complete_source_retained"
    } else {
        "completed"
    };
    state.ledger.lock().finish_backup_run(
        &run_id,
        &finished_at,
        outcome,
        last_error.as_ref().map(|error| error.message_code.as_str()),
    )?;

    let total_files = prepared.iter().try_fold(0_u64, |total, prepared| {
        total
            .checked_add(
                u64::try_from(prepared.plans.len() + prepared.additional_plans.len())
                    .map_err(|_| CoreError::InvalidRequest)?,
            )
            .ok_or(CoreError::InvalidRequest)
    })?;
    let fields = [
        ("count", AuditValue::Unsigned(total_files)),
        ("bytes", AuditValue::Unsigned(required_copy_bytes)),
        ("mode", AuditValue::Text(outcome)),
    ];
    state.append_audit(
        &AuditEvent {
            occurred_at: audit_now(),
            level: if failed_transmitters.is_empty() {
                AuditLevel::Info
            } else {
                AuditLevel::Error
            },
            code: "backup.run_complete",
            transmitter: None,
            fields: &fields,
        },
        AuditDurability::SyncData,
    )?;
    let automatic_trash_enabled = frozen_preferences.automatic_trash;
    let mut automatic_snapshots = Vec::new();
    let mut legacy_contexts = Vec::new();
    let mut runtime = state.runtime.lock();
    for prepared_tx in prepared {
        let verified = verified_by_tx
            .remove(&prepared_tx.transmitter)
            .unwrap_or_default();
        let verified_additional = verified_additional_by_tx
            .remove(&prepared_tx.transmitter)
            .unwrap_or_default();
        let complete = m4a_conversion_enabled
            && !failed_transmitters.contains(&prepared_tx.transmitter)
            && verified.len() == prepared_tx.scan.recordings.len()
            && verified_additional.len() == prepared_tx.scan.additional_files.len()
            && !prepared_tx.scan.issues.iter().any(|issue| {
                matches!(
                    issue,
                    ScanIssue::TransmitterPrefixMismatch | ScanIssue::UnsafeSessionEntry
                )
            });
        let generation = runtime
            .scan_generations
            .entry(prepared_tx.transmitter)
            .or_default();
        *generation = generation.saturating_add(1);
        let scan_generation = *generation;
        let current_source_paths = source_paths_for_scan(&prepared_tx.scan);
        let context = runtime
            .mounted
            .get(&prepared_tx.transmitter)
            .map(|mounted| DeletionContext {
                transmitter: prepared_tx.transmitter,
                paired_volume_uuid: mounted.descriptor.volume_uuid.clone(),
                mount_generation: mounted.descriptor.mount_generation,
                scan_generation,
                destination_generation: runtime.destination_generation,
                source_root: mounted.descriptor.mount_root.clone(),
                destination_root: runtime.destination.clone(),
            });
        if let Some(context) = context.clone() {
            legacy_contexts.push(context);
        }
        if complete && !verified.is_empty() {
            if automatic_trash_enabled && let Some(context) = context {
                automatic_snapshots.push(CompleteDeletionSnapshot {
                    context,
                    candidates: verified.clone(),
                    additional_files: verified_additional.clone(),
                    current_source_paths: current_source_paths.clone(),
                    m4a_barrier_run_id: run_id.clone(),
                });
            }
            runtime.verified.insert(prepared_tx.transmitter, verified);
            runtime
                .verified_additional
                .insert(prepared_tx.transmitter, verified_additional);
            runtime
                .m4a_barrier_runs
                .insert(prepared_tx.transmitter, run_id.clone());
            runtime
                .current_source_paths
                .insert(prepared_tx.transmitter, current_source_paths);
        } else {
            runtime.verified.remove(&prepared_tx.transmitter);
            runtime.verified_additional.remove(&prepared_tx.transmitter);
            runtime.m4a_barrier_runs.remove(&prepared_tx.transmitter);
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
    runtime.snapshot.failure_stage = None;
    runtime.snapshot.setting_applies_next_run = false;
    runtime.snapshot.current_item_ordinal = None;
    runtime.snapshot.last_success_at = failed_transmitters
        .is_empty()
        .then_some(finished_at.clone());
    runtime.snapshot.error = last_error;
    runtime.snapshot.overall_progress =
        ProgressDto::from(&combine_progress(progress_by_tx.values()));
    publish_locked(app, &mut runtime);
    drop(runtime);

    for context in legacy_contexts {
        if let Err(error) = reconcile_legacy_sessions(app, state, &context) {
            state.report_failure(
                "legacy_session_cleanup",
                "trash",
                &error,
                Some(context.transmitter),
                None,
            );
            state.set_error(app, error, Some(context.transmitter));
        }
    }
    for snapshot in automatic_snapshots {
        if let Err(error) = retire_automatically(app, state, snapshot) {
            state.report_failure("automatic_trash", "trash", &error, None, None);
            state.set_error(app, error, None);
        }
    }

    let (activity_code, severity) = if failed_transmitters.is_empty() {
        if total_files == 0 || required_copy_bytes == 0 {
            ("nothing_new", ActivitySeverity::Info)
        } else {
            ("backup_complete", ActivitySeverity::Success)
        }
    } else {
        ("partial_failure", ActivitySeverity::Error)
    };
    if let Err(error) = state.record_activity(
        app,
        ActivityEntry {
            occurred_at: finished_at,
            code: activity_code.to_owned(),
            transmitter: None,
            count_value: Some(total_files),
            byte_value: Some(required_copy_bytes),
            severity,
        },
    ) {
        state.report_failure("backup_run", "activity_persistence", &error, None, None);
    }

    let body = if failed_transmitters.is_empty() {
        if total_files == 0 || required_copy_bytes == 0 {
            "새 녹음이 없습니다."
        } else {
            "모든 녹음의 복사와 SHA-256 검증을 마쳤습니다."
        }
    } else {
        "일부 파일을 백업하지 못했습니다. 원본은 그대로 남아 있습니다."
    };
    if app
        .notification()
        .builder()
        .title("DJI Mic Backup")
        .body(body)
        .show()
        .is_err()
    {
        let error = adapter_public_error("notification_failed", true);
        state.report_public_failure("backup_run", "notification", &error, None);
    }
    Ok(())
}

fn reconcile_legacy_sessions(
    app: &AppHandle,
    state: &AppState,
    context: &DeletionContext,
) -> Result<(), CoreError> {
    if state
        .ledger
        .lock()
        .legacy_retired_recordings(context.transmitter)?
        .is_empty()
    {
        return Ok(());
    }
    let fields = [("mode", AuditValue::Text("legacy_empty_session"))];
    state.append_audit(
        &AuditEvent {
            occurred_at: audit_now(),
            level: AuditLevel::Info,
            code: "retirement.preflight",
            transmitter: Some(context.transmitter),
            fields: &fields,
        },
        AuditDurability::SyncData,
    )?;
    let moved_at = now_string();
    let moved =
        reconcile_legacy_empty_sessions(context, &mut state.ledger.lock(), &MacTrash, &moved_at)?;
    if moved == 0 {
        return Ok(());
    }
    let fields = [
        ("count", AuditValue::Unsigned(moved)),
        ("mode", AuditValue::Text("legacy_empty_session")),
    ];
    state.append_audit(
        &AuditEvent {
            occurred_at: audit_now(),
            level: AuditLevel::Info,
            code: "trash.legacy_empty_session",
            transmitter: Some(context.transmitter),
            fields: &fields,
        },
        AuditDurability::SyncData,
    )?;
    if let Err(error) = state.record_activity(
        app,
        ActivityEntry {
            occurred_at: moved_at,
            code: "legacy_session_moved_to_trash".to_owned(),
            transmitter: Some(context.transmitter),
            count_value: Some(moved),
            byte_value: None,
            severity: ActivitySeverity::Success,
        },
    ) {
        state.report_failure(
            "legacy_session_cleanup",
            "activity_persistence",
            &error,
            Some(context.transmitter),
            None,
        );
    }
    Ok(())
}

fn retire_automatically(
    app: &AppHandle,
    state: &AppState,
    snapshot: CompleteDeletionSnapshot,
) -> Result<(), CoreError> {
    let transmitter = snapshot.context.transmitter;
    {
        let mut runtime = state.runtime.lock();
        runtime.snapshot.current_stage = Some(CurrentStage::SourceRevalidation);
        update_transmitter(&mut runtime.snapshot, transmitter, |transmitter_snapshot| {
            transmitter_snapshot.deletion_phase = DeletionPhase::Revalidating;
        });
        publish_locked(app, &mut runtime);
    }
    let fields = [("mode", AuditValue::Text("automatic"))];
    state.append_audit(
        &AuditEvent {
            occurred_at: audit_now(),
            level: AuditLevel::Info,
            code: "retirement.preflight",
            transmitter: Some(transmitter),
            fields: &fields,
        },
        AuditDurability::SyncData,
    )?;
    let deletion_allowed = !state.ledger.lock().deletion_disabled();
    let proposal = state.proposals.lock().prepare(
        snapshot.clone(),
        state.elapsed(),
        deletion_allowed,
        &NoDeletionFaults,
    )?;
    state.append_audit(
        &AuditEvent {
            occurred_at: audit_now(),
            level: AuditLevel::Info,
            code: "retirement.authorized",
            transmitter: Some(transmitter),
            fields: &fields,
        },
        AuditDurability::SyncData,
    )?;
    {
        let mut runtime = state.runtime.lock();
        runtime.snapshot.current_stage = Some(CurrentStage::Trash);
        publish_locked(app, &mut runtime);
    }
    let started_at = now_string();
    let finished_at = now_string();
    let report = state.proposals.lock().confirm(
        &proposal.proposal_id,
        DeletionConfirmation {
            current_context: &snapshot.context,
            now: state.elapsed(),
            started_at: &started_at,
            finished_at: &finished_at,
        },
        &mut state.ledger.lock(),
        &MacTrash,
        &NoDeletionFaults,
    )?;
    {
        let mut runtime = state.runtime.lock();
        update_transmitter(&mut runtime.snapshot, transmitter, |transmitter_snapshot| {
            transmitter_snapshot.deletion_phase = match report.outcome {
                DeletionOutcome::Deleted => DeletionPhase::Deleted,
                DeletionOutcome::Refused => DeletionPhase::Refused,
                DeletionOutcome::PartiallyDeleted => DeletionPhase::PartiallyDeleted,
            };
            transmitter_snapshot.deletion_ready = false;
        });
        runtime.verified.remove(&transmitter);
        runtime.verified_additional.remove(&transmitter);
        runtime.m4a_barrier_runs.remove(&transmitter);
        runtime.current_source_paths.remove(&transmitter);
        runtime.snapshot.current_stage = None;
        publish_locked(app, &mut runtime);
    }
    let fields = [
        ("files", AuditValue::Unsigned(report.deleted_files)),
        ("bytes", AuditValue::Unsigned(report.deleted_bytes)),
        ("mode", AuditValue::Text("automatic")),
    ];
    state.append_audit(
        &AuditEvent {
            occurred_at: audit_now(),
            level: if report.outcome == DeletionOutcome::Deleted {
                AuditLevel::Info
            } else {
                AuditLevel::Error
            },
            code: "retirement.complete",
            transmitter: Some(transmitter),
            fields: &fields,
        },
        AuditDurability::SyncData,
    )?;
    if let Err(error) = state.record_activity(
        app,
        ActivityEntry {
            occurred_at: finished_at,
            code: if report.outcome == DeletionOutcome::Deleted {
                "automatic_trash_complete"
            } else {
                "automatic_trash_partial"
            }
            .to_owned(),
            transmitter: Some(transmitter),
            count_value: Some(report.deleted_files),
            byte_value: Some(report.deleted_bytes),
            severity: if report.outcome == DeletionOutcome::Deleted {
                ActivitySeverity::Success
            } else {
                ActivitySeverity::Error
            },
        },
    ) {
        state.report_failure(
            "automatic_trash",
            "activity_persistence",
            &error,
            Some(transmitter),
            None,
        );
    }
    Ok(())
}

pub fn prepare_trash(
    app: &AppHandle,
    state: &AppState,
    transmitter: Transmitter,
) -> Result<TrashProposalSummaryDto, CoreError> {
    let _guard = state.begin_operation()?;
    {
        let mut runtime = state.runtime.lock();
        update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
            snapshot.deletion_phase = DeletionPhase::Preparing;
        });
        runtime.snapshot.current_stage = Some(CurrentStage::SourceRevalidation);
        publish_locked(app, &mut runtime);
    }
    let (mounted, destination, destination_generation, verified, additional_files, barrier_run) = {
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
            runtime
                .verified_additional
                .get(&transmitter)
                .cloned()
                .unwrap_or_default(),
            runtime
                .m4a_barrier_runs
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
    let fields = [("mode", AuditValue::Text("manual"))];
    state.append_audit(
        &AuditEvent {
            occurred_at: audit_now(),
            level: AuditLevel::Info,
            code: "retirement.preflight",
            transmitter: Some(transmitter),
            fields: &fields,
        },
        AuditDurability::SyncData,
    )?;
    let summary = state.proposals.lock().prepare(
        CompleteDeletionSnapshot {
            context,
            candidates: verified,
            additional_files,
            current_source_paths: source_paths_for_scan(&scan),
            m4a_barrier_run_id: barrier_run,
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
    Ok(TrashProposalSummaryDto {
        proposal_id: summary.proposal_id,
        transmitter: summary.transmitter,
        session_count: summary.session_count,
        file_count: summary.file_count,
        byte_count: summary.byte_count,
        destination_summary: "DJI-Mic-Mini-2S 백업 폴더".to_owned(),
        expires_at,
    })
}

pub fn confirm_trash(
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
        runtime.snapshot.current_stage = Some(CurrentStage::SourceRevalidation);
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
        runtime.snapshot.current_stage = Some(CurrentStage::Trash);
        update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
            snapshot.deletion_phase = DeletionPhase::Deleting;
        });
        publish_locked(&app_for_deletion, &mut runtime);
    };
    let fields = [("mode", AuditValue::Text("manual"))];
    state.append_audit(
        &AuditEvent {
            occurred_at: audit_now(),
            level: AuditLevel::Info,
            code: "retirement.authorized",
            transmitter: Some(transmitter),
            fields: &fields,
        },
        AuditDurability::SyncData,
    )?;
    let report = state.proposals.lock().confirm_observed(
        proposal_id,
        DeletionConfirmation {
            current_context: &context,
            now: state.elapsed(),
            started_at: &now_string(),
            finished_at: &now_string(),
        },
        &mut state.ledger.lock(),
        &MacTrash,
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
    runtime.verified_additional.remove(&context.transmitter);
    runtime.m4a_barrier_runs.remove(&context.transmitter);
    runtime.current_source_paths.remove(&context.transmitter);
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
            occurred_at: now_string(),
            code: match report.outcome {
                DeletionOutcome::Deleted => "trash_complete",
                DeletionOutcome::Refused => "trash_refused",
                DeletionOutcome::PartiallyDeleted => "partial_trash",
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
    ) {
        state.report_failure(
            "confirm_trash",
            "activity_persistence",
            &error,
            Some(context.transmitter),
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
            occurred_at: audit_now(),
            level: match report.outcome {
                DeletionOutcome::Deleted => AuditLevel::Info,
                DeletionOutcome::Refused => AuditLevel::Warning,
                DeletionOutcome::PartiallyDeleted => AuditLevel::Error,
            },
            code: "retirement.complete",
            transmitter: Some(context.transmitter),
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

fn source_paths_for_scan(scan: &ScanResult) -> BTreeSet<std::path::PathBuf> {
    scan.recordings
        .iter()
        .map(|recording| recording.relative_path.clone())
        .chain(
            scan.additional_files
                .iter()
                .map(|file| file.relative_path.clone()),
        )
        .collect()
}

fn resolve_live_source(
    root: &std::path::Path,
    relative: &std::path::Path,
) -> Result<std::path::PathBuf, CoreError> {
    let canonical_root = std::fs::canonicalize(root).map_err(CoreError::CopyFailed)?;
    let path = canonical_root.join(relative);
    let metadata = std::fs::symlink_metadata(&path).map_err(CoreError::CopyFailed)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(CoreError::SourceChanged);
    }
    let canonical = std::fs::canonicalize(path).map_err(CoreError::CopyFailed)?;
    if !canonical.starts_with(canonical_root) {
        return Err(CoreError::SourceChanged);
    }
    Ok(canonical)
}

pub(crate) fn now_string() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

fn audit_now() -> OffsetDateTime {
    let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    OffsetDateTime::now_utc().to_offset(offset)
}

fn adapter_public_error(message_code: &str, retryable: bool) -> PublicError {
    PublicError {
        code: PublicErrorCode::Internal,
        message_code: message_code.to_owned(),
        retryable,
        transmitter: None,
    }
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
