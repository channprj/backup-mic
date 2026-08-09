use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use backup_core::{
    artifact::{ConversionStatus, OutputFormat, VerifiedArtifact, validate_m4a_artifact},
    backup::CancellationToken,
    error::CoreError,
    filesystem::{is_safe_relative_path, modified_nanos},
    hash::hash_file,
    ledger::{Ledger, VerifiedRecording},
    state::CurrentStage,
};
use tempfile::TempPath;
use uuid::Uuid;

use crate::platform::macos::audio::AudioTools;

pub struct M4aPublishResult {
    pub recording: VerifiedRecording,
    pub superseded_wav: PathBuf,
}

pub struct PreparedM4aArtifact {
    pub recording: VerifiedRecording,
    pub superseded_wav_relative_path: PathBuf,
    pub superseded_wav_size: u64,
    pub superseded_wav_sha256: String,
    recovery_marker: Option<PathBuf>,
}

pub fn order_conversion_cohort(
    mut recordings: Vec<VerifiedRecording>,
) -> Result<Vec<VerifiedRecording>, CoreError> {
    if recordings.is_empty()
        || recordings.iter().any(|recording| recording.id.is_empty())
        || recordings
            .iter()
            .map(|recording| recording.id.as_str())
            .collect::<HashSet<_>>()
            .len()
            != recordings.len()
    {
        return Err(CoreError::InvalidRequest);
    }
    recordings.sort_by(|left, right| {
        left.artifact
            .relative_path
            .cmp(&right.artifact.relative_path)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(recordings)
}

pub fn publish_m4a(
    destination_root: &Path,
    wav: &VerifiedRecording,
    ledger: &mut Ledger,
    tools: &dyn AudioTools,
    cancellation: &CancellationToken,
    observer: &mut dyn FnMut(CurrentStage),
) -> Result<M4aPublishResult, CoreError> {
    let prepared = prepare_m4a(destination_root, wav, tools, cancellation, observer)?;
    ledger.replace_verified_artifact_with_superseded_wav(
        &prepared.recording,
        &prepared.superseded_wav_relative_path,
        prepared.superseded_wav_size,
        &prepared.superseded_wav_sha256,
    )?;
    finalize_prepared_m4a(&prepared)?;
    Ok(M4aPublishResult {
        recording: prepared.recording,
        superseded_wav: prepared.superseded_wav_relative_path,
    })
}

pub fn prepare_m4a(
    destination_root: &Path,
    wav: &VerifiedRecording,
    tools: &dyn AudioTools,
    cancellation: &CancellationToken,
    observer: &mut dyn FnMut(CurrentStage),
) -> Result<PreparedM4aArtifact, CoreError> {
    cancellation.check()?;
    if wav.artifact.format != OutputFormat::Wav
        || wav.conversion_status != ConversionStatus::NotRequired
        || !is_safe_relative_path(&wav.artifact.relative_path)
    {
        return Err(CoreError::InvalidRequest);
    }
    let canonical_root =
        fs::canonicalize(destination_root).map_err(|_| CoreError::DestinationUnavailable)?;
    let wav_path = resolve_regular(&canonical_root, &wav.artifact.relative_path)?;
    let wav_digest = hash_file(&wav_path)?;
    if wav_digest.size != wav.artifact.byte_count
        || wav_digest.sha256 != wav.artifact.sha256
        || wav.source_size != wav.artifact.byte_count
        || wav.source_sha256 != wav.artifact.sha256
    {
        return Err(CoreError::HashMismatch);
    }
    let source_audio = tools.inspect(&wav_path)?;
    let default_relative = wav.artifact.relative_path.with_extension("m4a");
    if let Some(recovered) = recover_finalized_m4a(
        &canonical_root,
        &default_relative,
        wav,
        &source_audio,
        tools,
        cancellation,
        observer,
    )? {
        return Ok(recovered);
    }
    let final_relative = choose_m4a_relative(
        &canonical_root,
        &wav.artifact.relative_path,
        &wav.source_sha256,
    )?;
    let final_path = canonical_root.join(&final_relative);
    let parent = final_path
        .parent()
        .ok_or(CoreError::DestinationUnavailable)?;
    fs::create_dir_all(parent).map_err(CoreError::CopyFailed)?;
    let stem = final_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or(CoreError::InvalidRequest)?;
    let part_path = parent.join(format!(".{stem}.m4a.part-{}", Uuid::new_v4()));

    observer(CurrentStage::Conversion);
    cancellation.check()?;
    if let Err(error) = tools.convert_aac_lc_128k(&wav_path, &part_path) {
        cleanup_owned_part(&part_path);
        return Err(error);
    }
    let part = TempPath::try_from_path(part_path).map_err(CoreError::CopyFailed)?;
    fs::File::open(&part)
        .and_then(|file| file.sync_all())
        .map_err(CoreError::SyncFailed)?;

    observer(CurrentStage::ArtifactVerification);
    cancellation.check()?;
    let converted_audio = tools.inspect(&part)?;
    let verified_audio = validate_m4a_artifact(&source_audio, &converted_audio)?;
    let artifact_digest = hash_file(&part)?;
    let marker_path = create_recovery_marker(
        parent,
        &final_path,
        &wav.source_sha256,
        &artifact_digest.sha256,
        artifact_digest.size,
    )?;
    match part.persist_noclobber(&final_path) {
        Ok(()) => {}
        Err(error) => {
            cleanup_recovery_marker(&marker_path, parent)?;
            return Err(CoreError::CopyFailed(error.error));
        }
    }
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(CoreError::SyncFailed)?;

    let mut recording = wav.clone();
    recording.artifact = VerifiedArtifact {
        relative_path: final_relative,
        format: OutputFormat::M4a,
        byte_count: artifact_digest.size,
        sha256: artifact_digest.sha256,
        audio: Some(verified_audio),
    };
    recording.conversion_status = ConversionStatus::Complete;
    recording.conversion_error_code = None;
    Ok(PreparedM4aArtifact {
        recording,
        superseded_wav_relative_path: wav.artifact.relative_path.clone(),
        superseded_wav_size: wav.artifact.byte_count,
        superseded_wav_sha256: wav.artifact.sha256.clone(),
        recovery_marker: Some(marker_path),
    })
}

pub fn finalize_prepared_m4a(prepared: &PreparedM4aArtifact) -> Result<(), CoreError> {
    let Some(marker_path) = &prepared.recovery_marker else {
        return Ok(());
    };
    let parent = marker_path
        .parent()
        .ok_or(CoreError::DestinationUnavailable)?;
    cleanup_recovery_marker(marker_path, parent)
}

#[allow(clippy::too_many_arguments)]
fn recover_finalized_m4a(
    destination_root: &Path,
    final_relative: &Path,
    wav: &VerifiedRecording,
    source_audio: &backup_core::artifact::AudioDescription,
    tools: &dyn AudioTools,
    cancellation: &CancellationToken,
    observer: &mut dyn FnMut(CurrentStage),
) -> Result<Option<PreparedM4aArtifact>, CoreError> {
    let final_path = destination_root.join(final_relative);
    let parent = final_path
        .parent()
        .ok_or(CoreError::DestinationUnavailable)?;
    let markers = recovery_markers(parent, &final_path)?;
    if markers.len() > 1 {
        return Err(CoreError::ArtifactInvalid);
    }
    let Some(marker_path) = markers.into_iter().next() else {
        return Ok(None);
    };
    if !final_path.exists() {
        cleanup_recovery_marker(&marker_path, parent)?;
        return Ok(None);
    }
    cancellation.check()?;
    let marker = read_recovery_marker(&marker_path)?;
    if marker.source_sha256 != wav.source_sha256 {
        return Err(CoreError::ArtifactInvalid);
    }
    let recovered_path = resolve_regular(destination_root, final_relative)?;
    let digest = hash_file(&recovered_path)?;
    if digest.size != marker.artifact_bytes || digest.sha256 != marker.artifact_sha256 {
        return Err(CoreError::ArtifactInvalid);
    }
    observer(CurrentStage::ArtifactVerification);
    let artifact_audio = tools.inspect(&recovered_path)?;
    let verified_audio = validate_m4a_artifact(source_audio, &artifact_audio)?;
    let mut recording = wav.clone();
    recording.artifact = VerifiedArtifact {
        relative_path: final_relative.to_path_buf(),
        format: OutputFormat::M4a,
        byte_count: digest.size,
        sha256: digest.sha256,
        audio: Some(verified_audio),
    };
    recording.conversion_status = ConversionStatus::Complete;
    recording.conversion_error_code = None;
    Ok(Some(PreparedM4aArtifact {
        recording,
        superseded_wav_relative_path: wav.artifact.relative_path.clone(),
        superseded_wav_size: wav.artifact.byte_count,
        superseded_wav_sha256: wav.artifact.sha256.clone(),
        recovery_marker: Some(marker_path),
    }))
}

struct RecoveryMarker {
    source_sha256: String,
    artifact_sha256: String,
    artifact_bytes: u64,
}

fn create_recovery_marker(
    parent: &Path,
    final_path: &Path,
    source_sha256: &str,
    artifact_sha256: &str,
    artifact_bytes: u64,
) -> Result<PathBuf, CoreError> {
    let file_name = final_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(CoreError::InvalidRequest)?;
    let marker_path = parent.join(format!(".{file_name}.recovery-{}", Uuid::new_v4()));
    let mut marker = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker_path)
        .map_err(CoreError::CopyFailed)?;
    write!(
        marker,
        "v1\nsource_sha256={source_sha256}\nartifact_sha256={artifact_sha256}\nartifact_bytes={artifact_bytes}\n"
    )
    .map_err(CoreError::CopyFailed)?;
    marker.sync_all().map_err(CoreError::SyncFailed)?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(CoreError::SyncFailed)?;
    Ok(marker_path)
}

fn recovery_markers(parent: &Path, final_path: &Path) -> Result<Vec<PathBuf>, CoreError> {
    let file_name = final_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(CoreError::InvalidRequest)?;
    let prefix = format!(".{file_name}.recovery-");
    let mut markers = Vec::new();
    for entry in fs::read_dir(parent).map_err(CoreError::CopyFailed)? {
        let entry = entry.map_err(CoreError::CopyFailed)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(id) = name.strip_prefix(&prefix) else {
            continue;
        };
        if Uuid::parse_str(id).is_ok() {
            let metadata = entry.metadata().map_err(CoreError::CopyFailed)?;
            if !metadata.is_file()
                || entry
                    .file_type()
                    .map_err(CoreError::CopyFailed)?
                    .is_symlink()
            {
                return Err(CoreError::ArtifactInvalid);
            }
            markers.push(entry.path());
        }
    }
    Ok(markers)
}

fn read_recovery_marker(path: &Path) -> Result<RecoveryMarker, CoreError> {
    let contents = fs::read_to_string(path).map_err(CoreError::CopyFailed)?;
    if contents.len() > 512 {
        return Err(CoreError::ArtifactInvalid);
    }
    let mut lines = contents.lines();
    if lines.next() != Some("v1") {
        return Err(CoreError::ArtifactInvalid);
    }
    let source_sha256 = lines
        .next()
        .and_then(|line| line.strip_prefix("source_sha256="))
        .filter(|value| is_sha256(value))
        .ok_or(CoreError::ArtifactInvalid)?
        .to_owned();
    let artifact_sha256 = lines
        .next()
        .and_then(|line| line.strip_prefix("artifact_sha256="))
        .filter(|value| is_sha256(value))
        .ok_or(CoreError::ArtifactInvalid)?
        .to_owned();
    let artifact_bytes = lines
        .next()
        .and_then(|line| line.strip_prefix("artifact_bytes="))
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .ok_or(CoreError::ArtifactInvalid)?;
    if lines.next().is_some() {
        return Err(CoreError::ArtifactInvalid);
    }
    Ok(RecoveryMarker {
        source_sha256,
        artifact_sha256,
        artifact_bytes,
    })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn cleanup_recovery_marker(path: &Path, parent: &Path) -> Result<(), CoreError> {
    fs::remove_file(path).map_err(CoreError::CopyFailed)?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(CoreError::SyncFailed)
}

pub fn verify_published_artifact(
    destination_root: &Path,
    live_source_wav: &Path,
    recording: &VerifiedRecording,
    tools: &dyn AudioTools,
    cancellation: &CancellationToken,
) -> Result<(), CoreError> {
    cancellation.check()?;
    verify_source_metadata(live_source_wav, recording)?;
    let source_digest = hash_file(live_source_wav)?;
    verify_source_metadata(live_source_wav, recording)?;
    if source_digest.size != recording.source_size
        || source_digest.sha256 != recording.source_sha256
    {
        return Err(CoreError::SourceChanged);
    }
    let canonical_root =
        fs::canonicalize(destination_root).map_err(|_| CoreError::DestinationUnavailable)?;
    let artifact_path = resolve_regular(&canonical_root, &recording.artifact.relative_path)?;
    let artifact_digest = hash_file(&artifact_path)?;
    if artifact_digest.size != recording.artifact.byte_count
        || artifact_digest.sha256 != recording.artifact.sha256
    {
        return Err(CoreError::HashMismatch);
    }
    match recording.artifact.format {
        OutputFormat::Wav => {
            if source_digest != artifact_digest {
                return Err(CoreError::HashMismatch);
            }
        }
        OutputFormat::M4a => {
            let source_audio = tools.inspect(live_source_wav)?;
            let artifact_audio = tools.inspect(&artifact_path)?;
            let verified = validate_m4a_artifact(&source_audio, &artifact_audio)?;
            if recording.artifact.audio.as_ref() != Some(&verified) {
                return Err(CoreError::ArtifactInvalid);
            }
        }
    }
    cancellation.check()
}

fn verify_source_metadata(
    live_source_wav: &Path,
    recording: &VerifiedRecording,
) -> Result<(), CoreError> {
    let metadata = fs::symlink_metadata(live_source_wav).map_err(|_| CoreError::SourceChanged)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() != recording.source_size
        || modified_nanos(&metadata).map_err(|_| CoreError::SourceChanged)?
            != recording.source_mtime_ns
    {
        return Err(CoreError::SourceChanged);
    }
    Ok(())
}

fn choose_m4a_relative(
    destination_root: &Path,
    wav_relative: &Path,
    source_sha256: &str,
) -> Result<PathBuf, CoreError> {
    let default = wav_relative.with_extension("m4a");
    if !destination_root.join(&default).exists() {
        return Ok(default);
    }
    let parent = default.parent().ok_or(CoreError::InvalidRequest)?;
    let stem = default
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or(CoreError::InvalidRequest)?;
    for prefix_length in (8..=source_sha256.len()).step_by(4) {
        let candidate = parent.join(format!("{stem}-{}.m4a", &source_sha256[..prefix_length]));
        if !destination_root.join(&candidate).exists() {
            return Ok(candidate);
        }
    }
    Err(CoreError::DestinationUnavailable)
}

fn resolve_regular(root: &Path, relative: &Path) -> Result<PathBuf, CoreError> {
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path).map_err(|_| CoreError::DestinationUnavailable)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(CoreError::DestinationUnavailable);
    }
    let canonical = fs::canonicalize(path).map_err(|_| CoreError::DestinationUnavailable)?;
    if !canonical.starts_with(root) {
        return Err(CoreError::DestinationUnavailable);
    }
    Ok(canonical)
}

fn cleanup_owned_part(path: &Path) {
    if path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.starts_with('.') && value.contains(".m4a.part-"))
    {
        let _ = fs::remove_file(path);
    }
}
