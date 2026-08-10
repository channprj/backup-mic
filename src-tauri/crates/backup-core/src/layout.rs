use std::{
    fs::{self, File},
    io,
    path::{Component, Path, PathBuf},
};

use crate::{
    backup::CancellationToken,
    deletion::TrashAdapter,
    error::CoreError,
    hash::hash_file,
    ledger::{Ledger, VerifiedRecording},
};
use tempfile::NamedTempFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutMigration {
    pub from: PathBuf,
    pub to: PathBuf,
}

pub fn flatten_verified_recording_layout(
    destination_root: &Path,
    ledger: &mut Ledger,
    trash: &dyn TrashAdapter,
    cancellation: &CancellationToken,
) -> Result<Vec<LayoutMigration>, CoreError> {
    cancellation.check()?;
    let canonical_root =
        fs::canonicalize(destination_root).map_err(|_| CoreError::DestinationUnavailable)?;
    let recordings = ledger.verified_recordings()?;
    let mut migrations = Vec::new();
    for recording in recordings {
        cancellation.check()?;
        let old_relative = recording.artifact.relative_path.clone();
        let source = ledger.source(&recording.source_id)?;
        let Some(default_flat_relative) =
            legacy_flat_relative(&recording, source.legacy_slot.as_deref())
        else {
            continue;
        };
        let old_path = canonical_root.join(&old_relative);
        let target_relative =
            choose_flat_target(&canonical_root, &default_flat_relative, &recording)?;
        let target_path = canonical_root.join(&target_relative);

        if regular_artifact_matches(&canonical_root, &old_relative, &recording)? {
            if !regular_artifact_matches(&canonical_root, &target_relative, &recording)? {
                copy_verified_artifact(
                    &canonical_root,
                    &old_path,
                    &target_path,
                    &recording,
                    cancellation,
                )?;
            }
            cancellation.check()?;
            trash.move_to_trash(&old_path)?;
        } else if !regular_artifact_matches(&canonical_root, &target_relative, &recording)? {
            continue;
        }

        ledger.relocate_verified_artifact(
            &recording.id,
            &old_relative,
            &target_relative,
            recording.artifact.byte_count,
            &recording.artifact.sha256,
        )?;
        remove_empty_legacy_directory(&old_path)?;
        migrations.push(LayoutMigration {
            from: old_relative,
            to: target_relative,
        });
    }
    Ok(migrations)
}

fn legacy_flat_relative(
    recording: &VerifiedRecording,
    legacy_slot: Option<&str>,
) -> Option<PathBuf> {
    let mut components = recording.artifact.relative_path.components();
    let Component::Normal(year) = components.next()? else {
        return None;
    };
    let Component::Normal(date) = components.next()? else {
        return None;
    };
    let Component::Normal(transmitter) = components.next()? else {
        return None;
    };
    let Component::Normal(file_name) = components.next()? else {
        return None;
    };
    if components.next().is_some()
        || year.to_str().is_none_or(|value| {
            value.len() != 4 || !value.bytes().all(|byte| byte.is_ascii_digit())
        })
        || date.to_str().is_none_or(|value| {
            value.len() != 10
                || value.as_bytes().get(4) != Some(&b'-')
                || value.as_bytes().get(7) != Some(&b'-')
                || !value
                    .bytes()
                    .enumerate()
                    .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
        })
        || transmitter.to_str()? != legacy_slot?
    {
        return None;
    }
    Some(PathBuf::from(year).join(date).join(file_name))
}

fn choose_flat_target(
    root: &Path,
    default_relative: &Path,
    recording: &VerifiedRecording,
) -> Result<PathBuf, CoreError> {
    if !path_exists(root, default_relative)?
        || regular_artifact_matches(root, default_relative, recording)?
    {
        return Ok(default_relative.to_path_buf());
    }
    for prefix_length in (8..=recording.artifact.sha256.len()).step_by(4) {
        let candidate = with_hash_suffix(
            default_relative,
            &recording.artifact.sha256[..prefix_length],
        )?;
        if !path_exists(root, &candidate)? || regular_artifact_matches(root, &candidate, recording)?
        {
            return Ok(candidate);
        }
    }
    Err(CoreError::DestinationUnavailable)
}

fn path_exists(root: &Path, relative: &Path) -> Result<bool, CoreError> {
    match fs::symlink_metadata(root.join(relative)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(CoreError::CopyFailed(error)),
    }
}

fn regular_artifact_matches(
    root: &Path,
    relative: &Path,
    recording: &VerifiedRecording,
) -> Result<bool, CoreError> {
    let path = root.join(relative);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(CoreError::CopyFailed(error)),
    };
    if !metadata.file_type().is_file() || metadata.len() != recording.artifact.byte_count {
        return Ok(false);
    }
    let canonical_path = fs::canonicalize(&path).map_err(CoreError::CopyFailed)?;
    if !canonical_path.starts_with(root) {
        return Err(CoreError::DestinationUnavailable);
    }
    let digest = hash_file(&canonical_path)?;
    Ok(digest.size == recording.artifact.byte_count && digest.sha256 == recording.artifact.sha256)
}

fn copy_verified_artifact(
    root: &Path,
    old_path: &Path,
    target_path: &Path,
    recording: &VerifiedRecording,
    cancellation: &CancellationToken,
) -> Result<(), CoreError> {
    cancellation.check()?;
    let parent = target_path
        .parent()
        .ok_or(CoreError::DestinationUnavailable)?;
    fs::create_dir_all(parent).map_err(CoreError::CopyFailed)?;
    let mut temporary = NamedTempFile::new_in(parent).map_err(CoreError::CopyFailed)?;
    let mut source = File::open(old_path).map_err(CoreError::CopyFailed)?;
    let copied = io::copy(&mut source, temporary.as_file_mut()).map_err(CoreError::CopyFailed)?;
    if copied != recording.artifact.byte_count {
        return Err(CoreError::HashMismatch);
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(CoreError::SyncFailed)?;
    cancellation.check()?;
    let digest = hash_file(temporary.path())?;
    if digest.size != recording.artifact.byte_count || digest.sha256 != recording.artifact.sha256 {
        return Err(CoreError::HashMismatch);
    }
    temporary
        .persist_noclobber(target_path)
        .map_err(|error| CoreError::CopyFailed(error.error))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(CoreError::SyncFailed)?;
    let target_relative = target_path
        .strip_prefix(root)
        .map_err(|_| CoreError::DestinationUnavailable)?;
    if !regular_artifact_matches(root, target_relative, recording)? {
        return Err(CoreError::HashMismatch);
    }
    Ok(())
}

fn with_hash_suffix(path: &Path, hash_prefix: &str) -> Result<PathBuf, CoreError> {
    let parent = path.parent().ok_or(CoreError::InvalidRequest)?;
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or(CoreError::InvalidRequest)?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .ok_or(CoreError::InvalidRequest)?;
    Ok(parent.join(format!("{stem}-{hash_prefix}.{extension}")))
}

fn remove_empty_legacy_directory(old_path: &Path) -> Result<(), CoreError> {
    let parent = old_path.parent().ok_or(CoreError::InvalidRequest)?;
    let mut entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(CoreError::CopyFailed(error)),
    };
    if entries.next().is_none() {
        fs::remove_dir(parent).map_err(CoreError::CopyFailed)?;
    }
    Ok(())
}
