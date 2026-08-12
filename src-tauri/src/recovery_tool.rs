use std::{collections::HashSet, fs, path::Path};

use backup_core::{
    artifact::OutputFormat,
    backup::{
        BackupItemContext, CancellationToken, NoCopyFaults, prepare_backup_item_observed,
        progress_for_plans,
    },
    destination::{DestinationPlan, plan_rule_file_with_digest},
    error::CoreError,
    filesystem::{canonical_regular_file, modified_nanos},
    hash::hash_file,
    ledger::{Ledger, VerifiedRecording},
    recording::{ParsedRecordingName, RecordingObservation, archive_date_for_filename},
    rule_scanner::{RuleFileObservation, SelectedFileKind},
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    artifact_pipeline::{finalize_prepared_m4a, prepare_m4a, verify_published_artifact},
    platform::macos::audio::AudioTools,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryMode {
    DryRun,
    Apply,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Serialize)]
pub struct RecoverySummary {
    pub selected: u64,
    pub recoverable: u64,
    pub recovered: u64,
    pub failed: u64,
}

pub fn recover_verified_recordings(
    ledger: &mut Ledger,
    destination_root: &Path,
    source_root: &Path,
    recording_ids: &[String],
    mode: RecoveryMode,
    audio_tools: &dyn AudioTools,
) -> Result<RecoverySummary, CoreError> {
    if recording_ids.is_empty()
        || recording_ids.len() > 10_000
        || recording_ids.iter().any(|id| id.trim().is_empty())
        || recording_ids.iter().collect::<HashSet<_>>().len() != recording_ids.len()
    {
        return Err(CoreError::InvalidRequest);
    }
    let destination_root = canonical_directory(destination_root)?;
    let source_root = canonical_directory(source_root)?;
    if destination_root.starts_with(&source_root) || source_root.starts_with(&destination_root) {
        return Err(CoreError::InvalidRequest);
    }

    let cancellation = CancellationToken::default();
    let mut summary = RecoverySummary {
        selected: u64::try_from(recording_ids.len()).map_err(|_| CoreError::InvalidRequest)?,
        ..RecoverySummary::default()
    };
    for recording_id in recording_ids {
        let result = recover_one(
            ledger,
            &destination_root,
            &source_root,
            recording_id,
            mode,
            audio_tools,
            &cancellation,
        );
        match result {
            Ok(applied) => {
                summary.recoverable = summary.recoverable.saturating_add(1);
                if applied {
                    summary.recovered = summary.recovered.saturating_add(1);
                }
            }
            Err(_) => summary.failed = summary.failed.saturating_add(1),
        }
    }
    Ok(summary)
}

#[allow(clippy::too_many_arguments)]
fn recover_one(
    ledger: &mut Ledger,
    destination_root: &Path,
    source_root: &Path,
    recording_id: &str,
    mode: RecoveryMode,
    audio_tools: &dyn AudioTools,
    cancellation: &CancellationToken,
) -> Result<bool, CoreError> {
    let expected = ledger
        .verified_recording(recording_id)?
        .ok_or(CoreError::InvalidRequest)?;
    let source = ledger.source(&expected.source_id)?;
    let rule = ledger
        .backup_rule(&source.rule_id)?
        .ok_or(CoreError::LedgerCorrupt)?;
    let source_path = canonical_regular_file(source_root, &expected.source_relative_path)?;
    let source_digest = verify_source_evidence(&source_path, &expected)?;

    if expected.artifact.format == OutputFormat::M4a
        && destination_root
            .join(&expected.artifact.relative_path)
            .exists()
        && verify_published_artifact(
            destination_root,
            &source_path,
            &expected,
            audio_tools,
            cancellation,
        )
        .is_ok()
    {
        return Ok(mode == RecoveryMode::Apply);
    }

    let file_name = expected
        .source_relative_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(CoreError::InvalidRequest)?
        .to_owned();
    let fallback_date = OffsetDateTime::from_unix_timestamp_nanos(expected.source_mtime_ns)
        .map_err(|_| CoreError::InvalidRequest)?
        .date();
    let archive_date = archive_date_for_filename(rule.filename_profile, &file_name, fallback_date);
    let observed = RuleFileObservation {
        relative_path: expected.source_relative_path.clone(),
        file_name: file_name.clone(),
        kind: SelectedFileKind::RecordingWav,
        session_relative_path: expected
            .source_relative_path
            .parent()
            .map(Path::to_path_buf),
        size: expected.source_size,
        modified_nanos: expected.source_mtime_ns,
        archive_date,
    };
    let planned = plan_rule_file_with_digest(
        destination_root,
        &rule,
        &expected.source_id,
        observed,
        source_digest,
        None,
    )?;
    let plan = DestinationPlan {
        source: RecordingObservation {
            relative_path: planned.source.relative_path,
            file_name: planned.source.file_name,
            size: planned.source.size,
            modified_nanos: planned.source.modified_nanos,
            parsed_name: ParsedRecordingName {
                transmitter_hint: None,
                destination_date: planned.source.archive_date,
                used_fallback_date: true,
            },
        },
        source_sha256: planned.source_sha256,
        relative_destination: planned.relative_destination,
        disposition: planned.disposition,
    };
    if mode == RecoveryMode::DryRun {
        return Ok(false);
    }

    let verified_at = now_string();
    let mut progress = progress_for_plans(std::slice::from_ref(&plan))?;
    let mut wav = prepare_backup_item_observed(
        &BackupItemContext {
            source_root,
            destination_root,
            source_id: &expected.source_id,
            backup_run_id: &expected.backup_run_id,
            verified_at: &verified_at,
        },
        &plan,
        &mut progress,
        &NoCopyFaults,
        cancellation,
        &mut |_, _| {},
    )?;
    wav.id.clone_from(&expected.id);
    wav.retirement_status = expected.retirement_status;
    wav.retired_session_relative_path = expected.retired_session_relative_path.clone();
    ledger.replace_verified_recording_for_recovery(&expected, &wav)?;

    let prepared = prepare_m4a(
        destination_root,
        &wav,
        audio_tools,
        cancellation,
        &mut |_| {},
    )?;
    verify_published_artifact(
        destination_root,
        &source_path,
        &prepared.recording,
        audio_tools,
        cancellation,
    )?;
    ledger.replace_verified_artifact_with_superseded_wav(
        &prepared.recording,
        &prepared.superseded_wav_relative_path,
        prepared.superseded_wav_size,
        &prepared.superseded_wav_sha256,
    )?;
    finalize_prepared_m4a(&prepared)?;
    Ok(prepared.recording.artifact.format == OutputFormat::M4a)
}

fn verify_source_evidence(
    path: &Path,
    expected: &VerifiedRecording,
) -> Result<backup_core::hash::FileDigest, CoreError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| CoreError::SourceChanged)?;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_file()
        || metadata.len() != expected.source_size
        || modified_nanos(&metadata).map_err(|_| CoreError::SourceChanged)?
            != expected.source_mtime_ns
    {
        return Err(CoreError::SourceChanged);
    }
    let digest = hash_file(path)?;
    if digest.size != expected.source_size || digest.sha256 != expected.source_sha256 {
        return Err(CoreError::SourceChanged);
    }
    Ok(digest)
}

fn canonical_directory(path: &Path) -> Result<std::path::PathBuf, CoreError> {
    let metadata = fs::symlink_metadata(path).map_err(CoreError::CopyFailed)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CoreError::InvalidRequest);
    }
    fs::canonicalize(path).map_err(CoreError::CopyFailed)
}

fn now_string() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}
