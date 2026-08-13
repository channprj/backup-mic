use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
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
        BackupItemContext, CopyFaultPoint, CopyFaults, cleanup_owned_partials, ensure_capacity,
        execute_additional_file_copy, prepare_backup_item_observed, progress_for_plans,
    },
    batch::{BatchItemKey, BatchPhase, CopyBarrier, M4A_PROFILE_ID},
    clock::{Clock, SystemClock},
    deletion::{
        AdditionalDeletionCandidate, CompleteRuleDeletionSnapshot, DeletionCandidate,
        DeletionOutcome, DeletionProposal, NoDeletionFaults, ProposalInvalidation,
        RuleDeletionConfirmation, RuleDeletionContext, RuleSessionDeletionCandidate, TrashAdapter,
    },
    destination::{
        AdditionalFilePlan, DEFAULT_CAPACITY_RESERVE_BYTES, DestinationDisposition,
        DestinationPlan, RuleDestinationPlan, plan_rule_file_from_metadata,
    },
    error::{CoreError, PublicError, PublicErrorCode},
    events::{ActivityEntry, ActivitySeverity},
    ledger::VerifiedRecording,
    recording::{AdditionalFileObservation, ParsedRecordingName, RecordingObservation},
    rule::{BackupRule, compile_rule},
    rule_scanner::{
        RuleScanResult, SelectedFileKind, begin_rule_stable_scan, finish_rule_stable_scan,
        scan_rule_once,
    },
    scanner::STABILITY_INTERVAL,
    source::{MountedSourceAuthority, SourceId},
    state::{BackupPhase, CurrentStage, DeletionPhase, Transmitter},
};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::{
    app_state::{
        AppState, OperationGuard, RuleDeletionState, publish_locked, update_source,
        update_transmitter,
    },
    artifact_pipeline::{
        finalize_prepared_m4a, order_conversion_cohort, prepare_m4a, verify_published_artifact,
    },
    dto::{ProgressDto, TrashProposalSummaryDto},
    platform::{
        device_registry::DeviceRegistry,
        macos::{
            DiskArbitrationMonitor,
            audio::{AppleAudioTools, AudioTools},
            trash::MacTrash,
        },
    },
    rescan::{RescanDecision, RescanScheduler},
    rule_runtime::MatchedSource,
};

const RESCAN_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRunOutcome {
    pub source_id: SourceId,
    pub batch_run_id: String,
    pub phase: BackupPhase,
    pub verified_files: u64,
    pub deletion_ready: bool,
    pub error: Option<PublicError>,
    pub deletion_evidence: Option<CompletedRuleDeletionEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedRuleDeletionEvidence {
    pub authority: MountedSourceAuthority,
    pub rule: BackupRule,
    pub snapshot: CompleteRuleDeletionSnapshot,
}

#[derive(Debug)]
pub struct SourceDeletionOutcome {
    pub source_id: SourceId,
    pub report: Option<backup_core::deletion::DeletionReport>,
    pub error: Option<PublicError>,
}

pub trait SourceCopyFaults: Send + Sync {
    fn check(&self, source_id: &SourceId, point: CopyFaultPoint) -> Result<(), CoreError>;
}

#[derive(Debug, Default)]
pub struct NoSourceCopyFaults;

impl SourceCopyFaults for NoSourceCopyFaults {
    fn check(&self, _source_id: &SourceId, _point: CopyFaultPoint) -> Result<(), CoreError> {
        Ok(())
    }
}

struct ScopedCopyFaults<'a> {
    source_id: &'a SourceId,
    faults: &'a dyn SourceCopyFaults,
}

impl CopyFaults for ScopedCopyFaults<'_> {
    fn check(&self, point: CopyFaultPoint) -> Result<(), CoreError> {
        self.faults.check(self.source_id, point)
    }
}

struct PreparedRuleRecording {
    plan: DestinationPlan,
    existing: Option<VerifiedRecording>,
    replace_existing_m4a: bool,
}

struct PreparedSource {
    source_id: SourceId,
    authority: MountedSourceAuthority,
    rule: BackupRule,
    source_root: std::path::PathBuf,
    scan: RuleScanResult,
    recording_plans: Vec<PreparedRuleRecording>,
    companion_plans: Vec<AdditionalFilePlan>,
    copy_barrier: CopyBarrier,
    required_copy_bytes: u64,
    required_capacity_bytes: u64,
}

pub fn run_matched_sources_with_adapters(
    state: &AppState,
    matched_sources: &[MatchedSource],
    audio_tools: &dyn AudioTools,
    source_faults: &dyn SourceCopyFaults,
    clock: &dyn Clock,
    cancellation: &backup_core::backup::CancellationToken,
) -> Result<Vec<SourceRunOutcome>, CoreError> {
    let (destination, destination_generation) = state.backup_destination_snapshot();
    {
        let mut runtime = state.runtime.lock();
        for source in matched_sources {
            runtime.rule_deletions.remove(&source.authority.source.id);
        }
        runtime.awaiting_rule_deletion = None;
    }
    std::fs::create_dir_all(&destination).map_err(CoreError::CopyFailed)?;
    cleanup_owned_partials(&destination)?;
    let preferences = state.frozen_preferences();
    let local_offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let mut frozen_sources = matched_sources.to_vec();
    frozen_sources.sort_by(|left, right| left.authority.source.id.cmp(&right.authority.source.id));
    let mut outcomes = Vec::new();
    let mut eligible_sources = Vec::new();
    for mut matched in frozen_sources {
        let current = state
            .ledger
            .lock()
            .backup_rule(&matched.authority.source.rule_id);
        match current {
            Ok(Some(current))
                if current.enabled
                    && current.archived_at.is_none()
                    && current.id == matched.rule.id
                    && current.id == matched.authority.source.rule_id
                    && matched
                        .authority
                        .source
                        .volume_uuid
                        .eq_ignore_ascii_case(&matched.authority.descriptor.volume_uuid) =>
            {
                matched.rule = current;
                eligible_sources.push(matched);
            }
            Ok(_) => outcomes.push(failed_without_run(
                matched.authority.source.id,
                CoreError::IdentityMismatch,
            )),
            Err(error) => return Err(error),
        }
    }

    let mut scan_snapshots = Vec::new();
    for matched in eligible_sources {
        let first = compile_rule(matched.rule.clone()).and_then(|compiled| {
            begin_rule_stable_scan(
                &matched.authority.descriptor.mount_root,
                &compiled,
                local_offset,
                cancellation,
            )
            .map(|snapshot| (compiled, snapshot))
        });
        match first {
            Ok((compiled, snapshot)) => scan_snapshots.push((matched, compiled, snapshot)),
            Err(error) if source_failure_is_process_wide(&error) => return Err(error),
            Err(error) => outcomes.push(failed_without_run(matched.authority.source.id, error)),
        }
    }
    if !scan_snapshots.is_empty() {
        cancellation.check()?;
        clock.sleep(STABILITY_INTERVAL);
        cancellation.check()?;
    }

    let mut prepared = Vec::new();
    for (matched, compiled, first) in scan_snapshots {
        let scan = finish_rule_stable_scan(
            &matched.authority.descriptor.mount_root,
            &compiled,
            local_offset,
            first,
            cancellation,
        );
        match scan.and_then(|scan| {
            prepare_matched_source(
                state,
                matched.clone(),
                scan,
                &destination,
                destination_generation,
                preferences,
                cancellation,
            )
        }) {
            Ok(source) => prepared.push(source),
            Err(error) if source_failure_is_process_wide(&error) => return Err(error),
            Err(error) => outcomes.push(failed_without_run(matched.authority.source.id, error)),
        }
    }

    let required_capacity_bytes = prepared.iter().try_fold(0_u64, |total, source| {
        total
            .checked_add(source.required_capacity_bytes)
            .ok_or(CoreError::InvalidRequest)
    })?;
    if !state.destination_snapshot_is_current(&destination, destination_generation) {
        return Err(CoreError::DestinationUnavailable);
    }
    if required_capacity_bytes > 0 {
        ensure_capacity(
            &destination,
            required_capacity_bytes,
            DEFAULT_CAPACITY_RESERVE_BYTES,
        )?;
    }

    for source in prepared {
        outcomes.push(process_prepared_source(
            state,
            source,
            &destination,
            destination_generation,
            preferences,
            audio_tools,
            source_faults,
            cancellation,
        )?);
    }
    outcomes.sort_by(|left, right| left.source_id.cmp(&right.source_id));
    install_rule_deletion_evidence(state, &outcomes, destination_generation);
    Ok(outcomes)
}

fn install_rule_deletion_evidence(
    state: &AppState,
    outcomes: &[SourceRunOutcome],
    destination_generation: u64,
) {
    let mut runtime = state.runtime.lock();
    for outcome in outcomes {
        let Some(evidence) = &outcome.deletion_evidence else {
            continue;
        };
        let scan_generation = {
            let generation = runtime
                .rule_scan_generations
                .entry(outcome.source_id.clone())
                .or_default();
            *generation = generation.saturating_add(1);
            *generation
        };
        runtime.rule_deletions.insert(
            outcome.source_id.clone(),
            RuleDeletionState {
                authority: evidence.authority.clone(),
                rule: evidence.rule.clone(),
                snapshot: evidence.snapshot.clone(),
                destination_generation,
                scan_generation,
            },
        );
    }
}

fn prepare_matched_source(
    state: &AppState,
    matched: MatchedSource,
    scan: RuleScanResult,
    destination: &std::path::Path,
    destination_generation: u64,
    preferences: backup_core::batch::FrozenPreferences,
    cancellation: &backup_core::backup::CancellationToken,
) -> Result<PreparedSource, CoreError> {
    let source_root = matched.authority.descriptor.mount_root.clone();
    cancellation.check()?;
    if !state.destination_snapshot_is_current(destination, destination_generation) {
        return Err(CoreError::DestinationUnavailable);
    }
    if !state.source_authority_is_current(&matched.authority) {
        return Err(CoreError::DeviceRemoved);
    }
    if !scan.files.is_empty() {
        state
            .ledger
            .lock()
            .lock_rule_archive_directory(&matched.rule.id, &matched.rule.archive_directory_name)?;
    }
    let mut recording_plans = Vec::new();
    let mut companion_plans = Vec::new();
    let mut expected = BTreeSet::new();

    for observation in scan.files.iter().cloned() {
        match observation.kind {
            SelectedFileKind::RecordingWav => {
                let existing = state
                    .ledger
                    .lock()
                    .verified_recording_candidate_for_source(
                        &matched.authority.source.id,
                        &observation.relative_path,
                        observation.size,
                        observation.modified_nanos,
                    )?;
                let planned = plan_rule_file_from_metadata(
                    &source_root,
                    destination,
                    &matched.rule,
                    &matched.authority.source.id,
                    observation,
                    existing
                        .as_ref()
                        .map(|existing| (&existing.artifact, existing.source_sha256.as_str())),
                )?;
                expected.insert(BatchItemKey::Recording {
                    source_id: matched.authority.source.id.clone(),
                    relative_path: planned.source.relative_path.clone(),
                });
                recording_plans.push(PreparedRuleRecording {
                    plan: destination_plan_from_rule(planned),
                    existing,
                    replace_existing_m4a: false,
                });
            }
            SelectedFileKind::Companion => {
                let existing = state
                    .ledger
                    .lock()
                    .verified_additional_file_candidate_for_source(
                        &matched.authority.source.id,
                        &observation.relative_path,
                        observation.size,
                        observation.modified_nanos,
                    )?;
                let existing_artifact =
                    existing
                        .as_ref()
                        .map(|existing| backup_core::artifact::VerifiedArtifact {
                            relative_path: existing.artifact_relative_path.clone(),
                            format: OutputFormat::Wav,
                            byte_count: existing.artifact_size,
                            sha256: existing.artifact_sha256.clone(),
                            audio: None,
                        });
                let planned = plan_rule_file_from_metadata(
                    &source_root,
                    destination,
                    &matched.rule,
                    &matched.authority.source.id,
                    observation,
                    existing_artifact.as_ref().zip(
                        existing
                            .as_ref()
                            .map(|existing| existing.source_sha256.as_str()),
                    ),
                )?;
                expected.insert(BatchItemKey::Additional {
                    source_id: matched.authority.source.id.clone(),
                    relative_path: planned.source.relative_path.clone(),
                });
                companion_plans.push(additional_plan_from_rule(planned));
            }
        }
    }
    let required_copy_bytes = recording_plans
        .iter()
        .filter(|prepared| prepared.plan.disposition == DestinationDisposition::Copy)
        .map(|prepared| prepared.plan.source.size)
        .chain(
            companion_plans
                .iter()
                .filter(|plan| plan.disposition == DestinationDisposition::Copy)
                .map(|plan| plan.source.size),
        )
        .try_fold(0_u64, |total, bytes| {
            total.checked_add(bytes).ok_or(CoreError::InvalidRequest)
        })?;
    let conversion_bytes = if preferences.m4a_conversion {
        let historical = state
            .ledger
            .lock()
            .historical_wav_recordings_for_source(&matched.authority.source.id)?
            .into_iter()
            .try_fold(0_u64, |total, recording| {
                total
                    .checked_add(recording.artifact.byte_count)
                    .ok_or(CoreError::InvalidRequest)
            })?;
        recording_plans
            .iter()
            .filter(|prepared| prepared.existing.is_none() || prepared.replace_existing_m4a)
            .try_fold(historical, |total, prepared| {
                total
                    .checked_add(prepared.plan.source.size)
                    .ok_or(CoreError::InvalidRequest)
            })?
    } else {
        0
    };
    let required_capacity_bytes = required_copy_bytes
        .checked_add(conversion_bytes)
        .ok_or(CoreError::InvalidRequest)?;
    Ok(PreparedSource {
        source_id: matched.authority.source.id.clone(),
        authority: matched.authority,
        rule: matched.rule,
        source_root,
        scan,
        recording_plans,
        companion_plans,
        copy_barrier: CopyBarrier::new(expected)?,
        required_copy_bytes,
        required_capacity_bytes,
    })
}

fn destination_plan_from_rule(plan: RuleDestinationPlan) -> DestinationPlan {
    DestinationPlan {
        source: RecordingObservation {
            relative_path: plan.source.relative_path,
            file_name: plan.source.file_name,
            size: plan.source.size,
            modified_nanos: plan.source.modified_nanos,
            parsed_name: ParsedRecordingName {
                transmitter_hint: None,
                destination_date: plan.source.archive_date,
                used_fallback_date: true,
            },
        },
        source_sha256: plan.source_sha256,
        relative_destination: plan.relative_destination,
        disposition: plan.disposition,
    }
}

fn additional_plan_from_rule(plan: RuleDestinationPlan) -> AdditionalFilePlan {
    let classification = if plan
        .source
        .relative_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("m4a"))
    {
        backup_core::additional_file::AdditionalFileClass::M4a
    } else if plan.source.file_name.starts_with("._") {
        backup_core::additional_file::AdditionalFileClass::AppleDouble
    } else {
        backup_core::additional_file::AdditionalFileClass::Other
    };
    AdditionalFilePlan {
        source: AdditionalFileObservation {
            relative_path: plan.source.relative_path,
            file_name: plan.source.file_name,
            size: plan.source.size,
            modified_nanos: plan.source.modified_nanos,
            classification,
        },
        source_sha256: plan.source_sha256,
        relative_destination: plan.relative_destination,
        disposition: plan.disposition,
    }
}

#[allow(clippy::too_many_arguments)]
fn process_prepared_source(
    state: &AppState,
    mut source: PreparedSource,
    destination: &std::path::Path,
    destination_generation: u64,
    preferences: backup_core::batch::FrozenPreferences,
    audio_tools: &dyn AudioTools,
    source_faults: &dyn SourceCopyFaults,
    cancellation: &backup_core::backup::CancellationToken,
) -> Result<SourceRunOutcome, CoreError> {
    if source.authority.source.id != source.source_id
        || source.authority.source.rule_id != source.rule.id
        || source.scan.files.len() != source.recording_plans.len() + source.companion_plans.len()
    {
        return Err(CoreError::IdentityMismatch);
    }
    if let Err(error) = revalidate_run_context(
        state,
        &source.authority,
        destination,
        destination_generation,
    ) {
        return if source_failure_is_process_wide(&error) {
            Err(error)
        } else {
            Ok(failed_without_run(source.source_id, error))
        };
    }
    let run_id = Uuid::new_v4().to_string();
    let started_at = now_string();
    {
        let mut ledger = state.ledger.lock();
        ledger.begin_batch_run(
            &run_id,
            &source.source_id,
            &started_at,
            source.required_copy_bytes,
            preferences,
        )?;
        ledger.advance_batch_phase(&run_id, BatchPhase::Copying)?;
    }
    let scoped_faults = ScopedCopyFaults {
        source_id: &source.source_id,
        faults: source_faults,
    };
    let mut progress = progress_for_plans(
        &source
            .recording_plans
            .iter()
            .map(|prepared| prepared.plan.clone())
            .collect::<Vec<_>>(),
    )?;

    for plan in &source.companion_plans {
        if let Err(error) = revalidate_run_context(
            state,
            &source.authority,
            destination,
            destination_generation,
        ) {
            return finish_source_failure(state, source, run_id, error);
        }
        if let Err(error) = scoped_faults
            .check(CopyFaultPoint::SourceOpen)
            .and_then(|()| {
                execute_additional_file_copy(
                    &BackupItemContext {
                        source_root: &source.source_root,
                        destination_root: destination,
                        source_id: &source.source_id,
                        backup_run_id: &run_id,
                        verified_at: &now_string(),
                    },
                    &plan.source,
                    &plan.relative_destination,
                    cancellation,
                )
                .and_then(|file| {
                    if plan
                        .source_sha256
                        .as_ref()
                        .is_some_and(|expected| file.source_sha256 != *expected)
                    {
                        return Err(CoreError::HashMismatch);
                    }
                    state.ledger.lock().commit_verified_additional_file(&file)?;
                    Ok(())
                })
            })
        {
            source
                .copy_barrier
                .record_failed(&BatchItemKey::Additional {
                    source_id: source.source_id.clone(),
                    relative_path: plan.source.relative_path.clone(),
                })?;
            return finish_source_failure(state, source, run_id, error);
        }
        source
            .copy_barrier
            .record_verified(&BatchItemKey::Additional {
                source_id: source.source_id.clone(),
                relative_path: plan.source.relative_path.clone(),
            })?;
    }

    for prepared in &source.recording_plans {
        if let Err(error) = revalidate_run_context(
            state,
            &source.authority,
            destination,
            destination_generation,
        ) {
            return finish_source_failure(state, source, run_id, error);
        }
        let verified = if prepared.plan.disposition == DestinationDisposition::Reuse
            && prepared
                .existing
                .as_ref()
                .is_some_and(|existing| existing.artifact.format == OutputFormat::M4a)
        {
            (|| {
                let mut existing = prepared.existing.clone().ok_or(CoreError::LedgerCorrupt)?;
                resolve_live_source(&source.source_root, &prepared.plan.source.relative_path)
                    .and_then(|source_path| {
                        verify_published_artifact(
                            destination,
                            &source_path,
                            &existing,
                            audio_tools,
                            cancellation,
                        )
                    })?;
                existing.backup_run_id.clone_from(&run_id);
                existing.verified_at = now_string();
                existing.id = state.ledger.lock().commit_verified_recording(&existing)?;
                Ok(existing)
            })()
        } else {
            prepare_backup_item_observed(
                &BackupItemContext {
                    source_root: &source.source_root,
                    destination_root: destination,
                    source_id: &source.source_id,
                    backup_run_id: &run_id,
                    verified_at: &now_string(),
                },
                &prepared.plan,
                &mut progress,
                &scoped_faults,
                cancellation,
                &mut |_, _| {},
            )
            .and_then(|mut recording| {
                recording.id = if prepared.replace_existing_m4a {
                    state
                        .ledger
                        .lock()
                        .replace_verified_recording_from_fresh_copy(&recording)?
                } else {
                    state.ledger.lock().commit_verified_recording(&recording)?
                };
                Ok(recording)
            })
        };
        if let Err(error) = verified {
            source
                .copy_barrier
                .record_failed(&BatchItemKey::Recording {
                    source_id: source.source_id.clone(),
                    relative_path: prepared.plan.source.relative_path.clone(),
                })?;
            return finish_source_failure(state, source, run_id, error);
        }
        source
            .copy_barrier
            .record_verified(&BatchItemKey::Recording {
                source_id: source.source_id.clone(),
                relative_path: prepared.plan.source.relative_path.clone(),
            })?;
    }

    if !source.copy_barrier.conversion_allowed() {
        return finish_source_failure(state, source, run_id, CoreError::InvalidRequest);
    }
    state
        .ledger
        .lock()
        .advance_batch_phase(&run_id, BatchPhase::CopiesVerified)?;

    if let Err(error) = revalidate_run_context(
        state,
        &source.authority,
        destination,
        destination_generation,
    ) {
        return finish_source_failure(state, source, run_id, error);
    }

    let mut converted_files = 0_u64;
    if preferences.m4a_conversion {
        let cohort = state
            .ledger
            .lock()
            .historical_wav_recordings_for_source(&source.source_id)?;
        let cohort = if cohort.is_empty() {
            Vec::new()
        } else {
            match order_conversion_cohort(cohort) {
                Ok(cohort) => cohort,
                Err(error) => return finish_source_failure(state, source, run_id, error),
            }
        };
        if cohort.is_empty() {
            state.ledger.lock().commit_empty_m4a_barrier(&run_id)?;
        } else {
            let ids = cohort
                .iter()
                .map(|recording| recording.id.clone())
                .collect::<Vec<_>>();
            state
                .ledger
                .lock()
                .begin_conversion_cohort(&run_id, &ids, M4A_PROFILE_ID)?;
            for mut recording in cohort {
                recording.backup_run_id.clone_from(&run_id);
                recording.verified_at = now_string();
                let prepared = match prepare_m4a(
                    destination,
                    &recording,
                    audio_tools,
                    cancellation,
                    &mut |_| {},
                ) {
                    Ok(prepared) => prepared,
                    Err(error) => return finish_source_failure(state, source, run_id, error),
                };
                {
                    let mut ledger = state.ledger.lock();
                    ledger.replace_verified_artifact_with_superseded_wav(
                        &prepared.recording,
                        &prepared.superseded_wav_relative_path,
                        prepared.superseded_wav_size,
                        &prepared.superseded_wav_sha256,
                    )?;
                    ledger.mark_conversion_item_verified(&run_id, &prepared.recording.id)?;
                }
                finalize_prepared_m4a(&prepared)?;
                converted_files = converted_files.saturating_add(1);
            }
            state.ledger.lock().commit_m4a_barrier(&run_id)?;
        }
    }

    if let Err(error) = revalidate_run_context(
        state,
        &source.authority,
        destination,
        destination_generation,
    ) {
        return finish_source_failure(state, source, run_id, error);
    }

    let deletion_candidate_ready = preferences.m4a_conversion
        && !source.scan.files.is_empty()
        && source.scan.unsafe_session_count == 0
        && source.copy_barrier.verified_count() == source.scan.files.len();
    let deletion_evidence = if deletion_candidate_ready {
        match build_rule_deletion_evidence(state, &source, destination, &run_id) {
            Ok(evidence) => evidence,
            Err(error) => return finish_source_failure(state, source, run_id, error),
        }
    } else {
        None
    };
    let deletion_ready = deletion_evidence.is_some();
    let finished_at = now_string();
    state.ledger.lock().finish_backup_run(
        &run_id,
        &finished_at,
        if preferences.m4a_conversion {
            "completed"
        } else {
            "wav_backup_complete_source_retained"
        },
        None,
    )?;
    let verified_files = u64::try_from(source.copy_barrier.verified_count())
        .map_err(|_| CoreError::InvalidRequest)?;
    Ok(SourceRunOutcome {
        source_id: source.source_id,
        batch_run_id: run_id,
        phase: if source.required_copy_bytes == 0 && converted_files == 0 {
            BackupPhase::NothingNew
        } else {
            BackupPhase::CompletedDeletionPending
        },
        verified_files,
        deletion_ready,
        error: None,
        deletion_evidence,
    })
}

fn finish_source_failure(
    state: &AppState,
    source: PreparedSource,
    run_id: String,
    error: CoreError,
) -> Result<SourceRunOutcome, CoreError> {
    let public = error.public(None);
    state.ledger.lock().finish_backup_run(
        &run_id,
        &now_string(),
        "partial_failure",
        Some(&public.message_code),
    )?;
    if source_failure_is_process_wide(&error) {
        return Err(error);
    }
    Ok(SourceRunOutcome {
        source_id: source.source_id,
        batch_run_id: run_id,
        phase: BackupPhase::PartialFailure,
        verified_files: u64::try_from(source.copy_barrier.verified_count())
            .map_err(|_| CoreError::InvalidRequest)?,
        deletion_ready: false,
        error: Some(public),
        deletion_evidence: None,
    })
}

fn failed_without_run(source_id: SourceId, error: CoreError) -> SourceRunOutcome {
    SourceRunOutcome {
        source_id,
        batch_run_id: String::new(),
        phase: BackupPhase::PartialFailure,
        verified_files: 0,
        deletion_ready: false,
        error: Some(error.public(None)),
        deletion_evidence: None,
    }
}

fn build_rule_deletion_evidence(
    state: &AppState,
    source: &PreparedSource,
    destination_root: &std::path::Path,
    backup_run_id: &str,
) -> Result<Option<CompletedRuleDeletionEvidence>, CoreError> {
    let (recordings, additional_files, current_rule) = {
        let ledger = state.ledger.lock();
        (
            ledger.verified_recordings_for_backup_run(&source.source_id, backup_run_id)?,
            ledger.verified_additional_files_for_backup_run(&source.source_id, backup_run_id)?,
            ledger
                .backup_rule(&source.rule.id)?
                .ok_or(CoreError::LedgerCorrupt)?,
        )
    };
    if current_rule.id != source.rule.id || current_rule.updated_at != source.rule.updated_at {
        return Ok(None);
    }
    let mut recordings = recordings
        .into_iter()
        .map(|recording| {
            (
                recording.source_relative_path.clone(),
                DeletionCandidate {
                    recording_id: recording.id,
                    source_relative_path: recording.source_relative_path,
                    source_size: recording.source_size,
                    source_mtime_ns: recording.source_mtime_ns,
                    source_sha256: recording.source_sha256,
                    destination_relative_path: recording.artifact.relative_path,
                    destination_size: recording.artifact.byte_count,
                    destination_sha256: recording.artifact.sha256,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut additional_files = additional_files
        .into_iter()
        .map(|file| {
            (
                file.source_relative_path.clone(),
                AdditionalDeletionCandidate {
                    additional_file_id: file.id,
                    source_relative_path: file.source_relative_path,
                    source_size: file.source_size,
                    source_mtime_ns: file.source_mtime_ns,
                    source_sha256: file.source_sha256,
                    destination_relative_path: file.artifact_relative_path,
                    destination_size: file.artifact_size,
                    destination_sha256: file.artifact_sha256,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut files = Vec::new();
    let mut loose_additional = Vec::new();
    let mut sessions: BTreeMap<
        std::path::PathBuf,
        (Vec<DeletionCandidate>, Vec<AdditionalDeletionCandidate>),
    > = BTreeMap::new();
    for observation in &source.scan.files {
        match observation.kind {
            SelectedFileKind::RecordingWav => {
                let candidate = recordings
                    .remove(&observation.relative_path)
                    .ok_or(CoreError::LedgerCorrupt)?;
                if let Some(session) = &observation.session_relative_path {
                    sessions
                        .entry(session.clone())
                        .or_default()
                        .0
                        .push(candidate);
                } else {
                    files.push(candidate);
                }
            }
            SelectedFileKind::Companion => {
                let candidate = additional_files
                    .remove(&observation.relative_path)
                    .ok_or(CoreError::LedgerCorrupt)?;
                if let Some(session) = &observation.session_relative_path {
                    sessions
                        .entry(session.clone())
                        .or_default()
                        .1
                        .push(candidate);
                } else {
                    loose_additional.push(candidate);
                }
            }
        }
    }
    if !recordings.is_empty() || !additional_files.is_empty() {
        return Err(CoreError::LedgerCorrupt);
    }
    let sessions = sessions
        .into_iter()
        .map(
            |(relative_directory, (files, additional_files))| RuleSessionDeletionCandidate {
                relative_directory,
                files,
                additional_files,
            },
        )
        .collect();
    Ok(Some(CompletedRuleDeletionEvidence {
        authority: source.authority.clone(),
        rule: current_rule,
        snapshot: CompleteRuleDeletionSnapshot {
            files,
            additional_files: loose_additional,
            sessions,
            destination_root: destination_root.to_path_buf(),
            m4a_barrier_run_id: backup_run_id.to_owned(),
        },
    }))
}

fn revalidate_run_context(
    state: &AppState,
    authority: &MountedSourceAuthority,
    destination: &std::path::Path,
    destination_generation: u64,
) -> Result<(), CoreError> {
    if !state.destination_snapshot_is_current(destination, destination_generation) {
        return Err(CoreError::DestinationUnavailable);
    }
    if !state.source_authority_is_current(authority) {
        return Err(CoreError::DeviceRemoved);
    }
    Ok(())
}

fn source_failure_is_process_wide(error: &CoreError) -> bool {
    matches!(
        error,
        CoreError::DestinationUnavailable
            | CoreError::LedgerCorrupt
            | CoreError::Ledger(_)
            | CoreError::LedgerIo(_)
            | CoreError::Cancelled
    )
}

pub fn overall_backup_phase(outcomes: &[SourceRunOutcome]) -> BackupPhase {
    if outcomes.iter().any(|outcome| {
        matches!(
            outcome.phase,
            BackupPhase::PartialFailure | BackupPhase::Error
        )
    }) {
        BackupPhase::PartialFailure
    } else if outcomes.is_empty()
        || outcomes
            .iter()
            .all(|outcome| outcome.phase == BackupPhase::NothingNew)
    {
        BackupPhase::NothingNew
    } else {
        BackupPhase::CompletedDeletionPending
    }
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
            .name("backup-mic-device-events".to_owned())
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
                    let matched_sources = state.matched_sources();
                    for source_id in scheduler.mounted_sources() {
                        if !matched_sources.contains_key(&source_id) {
                            scheduler.unmount(&source_id);
                        }
                    }
                    let local_offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
                    for (source_id, matched) in &matched_sources {
                        if !scheduler.is_mounted(source_id) {
                            scheduler.mount(
                                source_id.clone(),
                                matched.initial_fingerprint.clone(),
                                now,
                            );
                        }
                    }
                    for source_id in scheduler.due_sources(now) {
                        if !state.automatic_backup_enabled() {
                            scheduler.defer(&source_id, now);
                            continue;
                        }
                        let Some(matched) = matched_sources.get(&source_id) else {
                            scheduler.unmount(&source_id);
                            continue;
                        };
                        let fingerprint = compile_rule(matched.rule.clone()).and_then(|rule| {
                            scan_rule_once(
                                &matched.authority.descriptor.mount_root,
                                &rule,
                                local_offset,
                            )
                            .map(|scan| scan.fingerprint)
                        });
                        match fingerprint {
                            Ok(fingerprint) => {
                                if matches!(
                                    scheduler.observe(
                                        source_id.clone(),
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
                                scheduler.defer(&source_id, now);
                                let transmitter = matched_legacy_transmitter(matched);
                                state.report_failure(
                                    "automatic_rescan",
                                    "metadata_scan",
                                    &error,
                                    transmitter,
                                    None,
                                );
                                state.set_error(&app, error, transmitter);
                            }
                        }
                    }
                    if matched_sources.is_empty() {
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
    state.backup_start_preflight()?;
    let guard = state.begin_operation()?;
    state
        .proposals
        .lock()
        .invalidate(ProposalInvalidation::BackupStarted);
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = run_backup(&app, &state, &guard) {
            if matches!(error, CoreError::Cancelled) {
                state.settle_cancelled_operation(&app);
            } else {
                state.report_failure("backup_run", "background_operation", &error, None, None);
                state.set_error(&app, error, None);
            }
        }
    });
    Ok(())
}

fn run_backup(app: &AppHandle, state: &AppState, guard: &OperationGuard) -> Result<(), CoreError> {
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
    let phase = if automatic_retirement_failed {
        BackupPhase::PartialFailure
    } else {
        overall_backup_phase(&outcomes)
    };
    let finished_at = now_string();
    let total_files = outcomes.iter().try_fold(0_u64, |total, outcome| {
        total
            .checked_add(outcome.verified_files)
            .ok_or(CoreError::InvalidRequest)
    })?;
    let last_error = outcomes
        .iter()
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
            BackupPhase::NothingNew => "nothing_new",
            _ => "backup_complete",
        }
        .to_owned();
        runtime.snapshot.current_stage = None;
        runtime.snapshot.failure_stage = None;
        runtime.snapshot.setting_applies_next_run = false;
        runtime.snapshot.current_item_ordinal = None;
        runtime.snapshot.error = last_error;
        runtime.snapshot.last_success_at =
            (phase != BackupPhase::PartialFailure).then_some(finished_at.clone());
        for matched_source in &matched {
            let source_phase = outcomes
                .iter()
                .find(|outcome| outcome.source_id == matched_source.authority.source.id)
                .map(|outcome| outcome.phase)
                .unwrap_or(BackupPhase::PartialFailure);
            let deletion_ready = outcomes
                .iter()
                .find(|outcome| outcome.source_id == matched_source.authority.source.id)
                .is_some_and(|outcome| outcome.deletion_ready)
                && !automatic_retirements
                    .iter()
                    .any(|retirement| retirement.source_id == matched_source.authority.source.id);
            let retirement = automatic_retirements
                .iter()
                .find(|retirement| retirement.source_id == matched_source.authority.source.id);
            let effective_phase = if retirement.is_some_and(|retirement| {
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
            if let Some(transmitter) = matched_legacy_transmitter(matched_source) {
                update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
                    snapshot.phase = effective_phase;
                    snapshot.deletion_ready = deletion_ready;
                    snapshot.deletion_phase = deletion_phase;
                });
            }
        }
        publish_locked(app, &mut runtime);
    }
    for outcome in &outcomes {
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
            AuditValue::Text(if phase == BackupPhase::PartialFailure {
                "partial_failure"
            } else {
                "completed"
            }),
        ),
    ];
    state.append_audit(
        &AuditEvent {
            occurred_at: audit_now(),
            level: if phase == BackupPhase::PartialFailure {
                AuditLevel::Error
            } else {
                AuditLevel::Info
            },
            code: "backup.run_complete",
            transmitter: None,
            fields: &fields,
        },
        AuditDurability::SyncData,
    )?;
    if app
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
    Ok(())
}

pub fn prepare_trash(
    app: &AppHandle,
    state: &AppState,
    transmitter: Transmitter,
) -> Result<TrashProposalSummaryDto, CoreError> {
    let source_id = {
        let runtime = state.runtime.lock();
        runtime
            .rule_deletions
            .iter()
            .find(|(source_id, _)| {
                runtime
                    .matched
                    .get(*source_id)
                    .and_then(matched_legacy_transmitter)
                    == Some(transmitter)
            })
            .map(|(source_id, _)| source_id.clone())
            .ok_or(CoreError::DeletionPreflightRefused)?
    };
    prepare_trash_for_source(app, state, &source_id)
}

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
        if let Some(transmitter) = transmitter {
            update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
                snapshot.deletion_phase = DeletionPhase::Preparing;
            });
        }
        runtime.snapshot.current_stage = Some(CurrentStage::SourceRevalidation);
        publish_locked(app, &mut runtime);
        (source_label, destination_summary, transmitter)
    };
    let fields = [("mode", AuditValue::Text("manual"))];
    state.append_audit(
        &AuditEvent {
            occurred_at: audit_now(),
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
        if let Some(transmitter) = transmitter {
            update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
                snapshot.deletion_phase = DeletionPhase::AwaitingConfirmation;
            });
        }
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
    let started_at = now_string();
    let finished_at = now_string();
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
        if let Some(transmitter) = transmitter {
            update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
                snapshot.deletion_phase = DeletionPhase::Revalidating;
            });
        }
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
        if let Some(transmitter) = transmitter {
            update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
                snapshot.deletion_phase = DeletionPhase::Deleting;
            });
        }
        publish_locked(&app_for_deletion, &mut runtime);
    };
    let fields = [("mode", AuditValue::Text("manual"))];
    state.append_audit(
        &AuditEvent {
            occurred_at: audit_now(),
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
    if let Some(transmitter) = transmitter {
        update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
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
    }
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
            occurred_at: audit_now(),
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

fn matched_legacy_transmitter(matched: &MatchedSource) -> Option<Transmitter> {
    match matched.authority.source.legacy_slot.as_deref() {
        Some("TX01") => Some(Transmitter::Tx01),
        Some("TX02") => Some(Transmitter::Tx02),
        _ => None,
    }
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
        source_id: None,
        source_label: None,
    }
}

fn source_label(state: &AppState, source_id: &SourceId) -> Option<String> {
    state
        .runtime
        .lock()
        .matched
        .get(source_id)
        .map(|matched| matched.authority.source.display_name.clone())
}

fn source_failure_stage(outcome: &SourceRunOutcome) -> &'static str {
    if outcome.batch_run_id.is_empty() {
        "source_preparation"
    } else {
        "source_pipeline"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_without_a_run_are_reported_as_source_preparation() {
        let preparation = SourceRunOutcome {
            source_id: SourceId::new(),
            batch_run_id: String::new(),
            phase: BackupPhase::PartialFailure,
            verified_files: 0,
            deletion_ready: false,
            error: Some(CoreError::InvalidRule.public(None)),
            deletion_evidence: None,
        };
        let mut pipeline = preparation.clone();
        pipeline.batch_run_id = "run".to_owned();

        assert_eq!(source_failure_stage(&preparation), "source_preparation");
        assert_eq!(source_failure_stage(&pipeline), "source_pipeline");
    }
}
