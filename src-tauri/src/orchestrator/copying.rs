//! Executing one recorder's copies, conversion cohort, and retirement evidence.

use std::collections::BTreeMap;

use backup_core::artifact::OutputFormat;
use backup_core::backup::{
    BackupItemContext, CopyFaultPoint, CopyFaults, execute_additional_file_copy,
    prepare_backup_item_observed, progress_for_plans,
};
use backup_core::batch::{BatchItemKey, BatchPhase, M4A_PROFILE_ID};
use backup_core::deletion::{
    AdditionalDeletionCandidate, CompleteRuleDeletionSnapshot, DeletionCandidate,
    RuleSessionDeletionCandidate, TrashAdapter,
};
use backup_core::destination::DestinationDisposition;
use backup_core::error::CoreError;
use backup_core::rule_scanner::SelectedFileKind;
use backup_core::source::SourceId;
use backup_core::state::BackupPhase;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::artifact_pipeline::{
    finalize_prepared_m4a, order_conversion_cohort, prepare_m4a, retire_superseded_wavs_for_source,
    verify_published_artifact,
};
use crate::clock;
use crate::platform::macos::audio::AudioTools;

use super::pipeline::{
    CompletedRuleDeletionEvidence, ScopedCopyFaults, SourceCopyFaults, SourceRunOutcome,
    classify_source_failure, revalidate_run_context, source_authority_is_available,
    source_failure_is_process_wide,
};
use super::planning::PreparedSource;
use super::shared::resolve_live_source;

#[allow(clippy::too_many_arguments)]
pub(super) fn process_prepared_source(
    state: &AppState,
    mut source: PreparedSource,
    destination: &std::path::Path,
    destination_generation: u64,
    preferences: backup_core::batch::FrozenPreferences,
    audio_tools: &dyn AudioTools,
    source_faults: &dyn SourceCopyFaults,
    destination_trash: &dyn TrashAdapter,
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
    let started_at = clock::now_string();
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
                        verified_at: &clock::now_string(),
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
                existing.verified_at = clock::now_string();
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
                    verified_at: &clock::now_string(),
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
                recording.verified_at = clock::now_string();
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
        let retirement = {
            let mut ledger = state.ledger.lock();
            retire_superseded_wavs_for_source(
                destination,
                &mut ledger,
                &source.source_id,
                audio_tools,
                destination_trash,
                cancellation,
            )
        };
        if let Err(error) = retirement {
            return finish_source_failure(state, source, run_id, error);
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
    let finished_at = clock::now_string();
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

pub(super) fn finish_source_failure(
    state: &AppState,
    source: PreparedSource,
    run_id: String,
    error: CoreError,
) -> Result<SourceRunOutcome, CoreError> {
    let error = classify_source_failure(
        source_authority_is_available(state, &source.authority),
        error,
    );
    let public = error.public(None);
    state.ledger.lock().finish_backup_run(
        &run_id,
        &clock::now_string(),
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

pub(super) fn failed_without_run(source_id: SourceId, error: CoreError) -> SourceRunOutcome {
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

pub(super) fn build_rule_deletion_evidence(
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
