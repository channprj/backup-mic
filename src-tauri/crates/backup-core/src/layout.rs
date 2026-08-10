use std::{
    fs::{self, File},
    io,
    path::{Component, Path, PathBuf},
};

use crate::{
    additional_file::VerifiedAdditionalFile,
    backup::CancellationToken,
    deletion::TrashAdapter,
    error::CoreError,
    hash::hash_file,
    ledger::{Ledger, VerifiedRecording},
    rule::BackupRule,
};
use tempfile::NamedTempFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutMigration {
    pub from: PathBuf,
    pub to: PathBuf,
}

pub fn migrate_legacy_rule_layout(
    destination_root: &Path,
    ledger: &mut Ledger,
    rule: &BackupRule,
    trash: &dyn TrashAdapter,
    cancellation: &CancellationToken,
) -> Result<Vec<LayoutMigration>, CoreError> {
    migrate_legacy_rule_layout_inner(
        destination_root,
        ledger,
        rule,
        trash,
        cancellation,
        &mut |error| Err(error),
    )
}

pub fn migrate_legacy_rule_layout_resilient(
    destination_root: &Path,
    ledger: &mut Ledger,
    rule: &BackupRule,
    trash: &dyn TrashAdapter,
    cancellation: &CancellationToken,
    on_failure: &mut dyn FnMut(&CoreError),
) -> Result<Vec<LayoutMigration>, CoreError> {
    migrate_legacy_rule_layout_inner(
        destination_root,
        ledger,
        rule,
        trash,
        cancellation,
        &mut |error| {
            on_failure(&error);
            Ok(())
        },
    )
}

fn migrate_legacy_rule_layout_inner(
    destination_root: &Path,
    ledger: &mut Ledger,
    rule: &BackupRule,
    trash: &dyn TrashAdapter,
    cancellation: &CancellationToken,
    on_failure: &mut dyn FnMut(CoreError) -> Result<(), CoreError>,
) -> Result<Vec<LayoutMigration>, CoreError> {
    cancellation.check()?;
    let canonical_root =
        fs::canonicalize(destination_root).map_err(|_| CoreError::DestinationUnavailable)?;
    let recordings = ledger.verified_recordings()?;
    let additional_files = ledger.verified_additional_files()?;
    let mut has_rule_evidence = false;
    for source_id in recordings
        .iter()
        .map(|recording| &recording.source_id)
        .chain(additional_files.iter().map(|file| &file.source_id))
    {
        if ledger.source(source_id)?.rule_id == rule.id {
            has_rule_evidence = true;
            break;
        }
    }
    if has_rule_evidence {
        ledger.lock_rule_archive_directory(&rule.id, &rule.archive_directory_name)?;
    }
    let mut migrations = Vec::new();

    for recording in recordings {
        cancellation.check()?;
        let result = migrate_recording_to_rule(
            &canonical_root,
            ledger,
            rule,
            trash,
            cancellation,
            &recording,
        );
        match result {
            Ok(Some(migration)) => migrations.push(migration),
            Ok(None) => {}
            Err(error) => on_failure(error)?,
        }
    }
    for file in additional_files {
        cancellation.check()?;
        let result =
            migrate_additional_to_rule(&canonical_root, ledger, rule, trash, cancellation, &file);
        match result {
            Ok(Some(migration)) => migrations.push(migration),
            Ok(None) => {}
            Err(error) => on_failure(error)?,
        }
    }
    Ok(migrations)
}

fn migrate_recording_to_rule(
    root: &Path,
    ledger: &mut Ledger,
    rule: &BackupRule,
    trash: &dyn TrashAdapter,
    cancellation: &CancellationToken,
    recording: &VerifiedRecording,
) -> Result<Option<LayoutMigration>, CoreError> {
    if ledger.source(&recording.source_id)?.rule_id != rule.id {
        return Ok(None);
    }
    let old_relative = &recording.artifact.relative_path;
    let Some(default_target) = legacy_rule_target(rule, old_relative, false) else {
        return Ok(None);
    };
    let Some(target_relative) = migrate_verified_file(
        root,
        old_relative,
        &default_target,
        recording.artifact.byte_count,
        &recording.artifact.sha256,
        trash,
        cancellation,
    )?
    else {
        return Ok(None);
    };
    ledger.relocate_verified_artifact(
        &recording.id,
        old_relative,
        &target_relative,
        recording.artifact.byte_count,
        &recording.artifact.sha256,
    )?;
    remove_empty_legacy_ancestors(root, &root.join(old_relative))?;
    Ok(Some(LayoutMigration {
        from: old_relative.clone(),
        to: target_relative,
    }))
}

fn migrate_additional_to_rule(
    root: &Path,
    ledger: &mut Ledger,
    rule: &BackupRule,
    trash: &dyn TrashAdapter,
    cancellation: &CancellationToken,
    file: &VerifiedAdditionalFile,
) -> Result<Option<LayoutMigration>, CoreError> {
    if ledger.source(&file.source_id)?.rule_id != rule.id {
        return Ok(None);
    }
    let old_relative = &file.artifact_relative_path;
    let Some(default_target) = legacy_rule_target(rule, old_relative, true) else {
        return Ok(None);
    };
    let Some(target_relative) = migrate_verified_file(
        root,
        old_relative,
        &default_target,
        file.artifact_size,
        &file.artifact_sha256,
        trash,
        cancellation,
    )?
    else {
        return Ok(None);
    };
    ledger.relocate_verified_additional_artifact(
        &file.id,
        old_relative,
        &target_relative,
        file.artifact_size,
        &file.artifact_sha256,
    )?;
    remove_empty_legacy_ancestors(root, &root.join(old_relative))?;
    Ok(Some(LayoutMigration {
        from: old_relative.clone(),
        to: target_relative,
    }))
}

fn legacy_rule_target(rule: &BackupRule, old_relative: &Path, additional: bool) -> Option<PathBuf> {
    let archive = Path::new(&rule.archive_directory_name);
    if old_relative.starts_with(archive) {
        return None;
    }
    if additional && old_relative.starts_with("source-extras") {
        return Some(archive.join(old_relative));
    }

    let mut components = old_relative.components();
    let Component::Normal(year) = components.next()? else {
        return None;
    };
    let Component::Normal(calendar) = components.next()? else {
        return None;
    };
    let Component::Normal(file_name) = components.next()? else {
        return None;
    };
    if components.next().is_some()
        || year.to_str().is_none_or(|value| {
            value.len() != 4 || !value.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return None;
    }
    let calendar = calendar.to_str()?;
    let month = if calendar.len() == 2 && calendar.bytes().all(|byte| byte.is_ascii_digit()) {
        calendar
    } else if calendar.len() == 10
        && calendar.as_bytes().get(4) == Some(&b'-')
        && calendar.as_bytes().get(7) == Some(&b'-')
        && calendar
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    {
        &calendar[5..7]
    } else {
        return None;
    };
    if !("01"..="12").contains(&month) {
        return None;
    }
    Some(archive.join(year).join(month).join(file_name))
}

fn migrate_verified_file(
    root: &Path,
    old_relative: &Path,
    default_target: &Path,
    byte_count: u64,
    sha256: &str,
    trash: &dyn TrashAdapter,
    cancellation: &CancellationToken,
) -> Result<Option<PathBuf>, CoreError> {
    let old_exists = path_exists(root, old_relative)?;
    let old_matches = regular_file_matches(root, old_relative, byte_count, sha256)?;
    if old_exists && !old_matches {
        return Ok(None);
    }
    let target_relative = choose_verified_target(root, default_target, byte_count, sha256)?;
    let target_matches = regular_file_matches(root, &target_relative, byte_count, sha256)?;
    if !old_matches && !target_matches {
        return Ok(None);
    }
    if old_matches {
        if !target_matches {
            copy_verified_file(
                root,
                &root.join(old_relative),
                &root.join(&target_relative),
                byte_count,
                sha256,
                cancellation,
            )?;
        }
        cancellation.check()?;
        trash.move_to_trash(&root.join(old_relative))?;
    }
    Ok(Some(target_relative))
}

fn choose_verified_target(
    root: &Path,
    default_relative: &Path,
    byte_count: u64,
    sha256: &str,
) -> Result<PathBuf, CoreError> {
    if !path_exists(root, default_relative)?
        || regular_file_matches(root, default_relative, byte_count, sha256)?
    {
        return Ok(default_relative.to_path_buf());
    }
    for prefix_length in (8..=sha256.len()).step_by(4) {
        let candidate = with_hash_suffix(default_relative, &sha256[..prefix_length])?;
        if !path_exists(root, &candidate)?
            || regular_file_matches(root, &candidate, byte_count, sha256)?
        {
            return Ok(candidate);
        }
    }
    Err(CoreError::DestinationUnavailable)
}

pub fn flatten_verified_recording_layout(
    destination_root: &Path,
    ledger: &mut Ledger,
    trash: &dyn TrashAdapter,
    cancellation: &CancellationToken,
) -> Result<Vec<LayoutMigration>, CoreError> {
    flatten_verified_recording_layout_inner(
        destination_root,
        ledger,
        trash,
        cancellation,
        &mut |error| Err(error),
    )
}

pub fn flatten_verified_recording_layout_resilient(
    destination_root: &Path,
    ledger: &mut Ledger,
    trash: &dyn TrashAdapter,
    cancellation: &CancellationToken,
    on_failure: &mut dyn FnMut(&CoreError),
) -> Result<Vec<LayoutMigration>, CoreError> {
    flatten_verified_recording_layout_inner(
        destination_root,
        ledger,
        trash,
        cancellation,
        &mut |error| {
            on_failure(&error);
            Ok(())
        },
    )
}

fn flatten_verified_recording_layout_inner(
    destination_root: &Path,
    ledger: &mut Ledger,
    trash: &dyn TrashAdapter,
    cancellation: &CancellationToken,
    on_failure: &mut dyn FnMut(CoreError) -> Result<(), CoreError>,
) -> Result<Vec<LayoutMigration>, CoreError> {
    cancellation.check()?;
    let canonical_root =
        fs::canonicalize(destination_root).map_err(|_| CoreError::DestinationUnavailable)?;
    let recordings = ledger.verified_recordings()?;
    let mut migrations = Vec::new();
    for recording in recordings {
        cancellation.check()?;
        match flatten_verified_recording(&canonical_root, ledger, trash, cancellation, &recording) {
            Ok(Some(migration)) => migrations.push(migration),
            Ok(None) => {}
            Err(error) => on_failure(error)?,
        }
    }
    Ok(migrations)
}

fn flatten_verified_recording(
    root: &Path,
    ledger: &mut Ledger,
    trash: &dyn TrashAdapter,
    cancellation: &CancellationToken,
    recording: &VerifiedRecording,
) -> Result<Option<LayoutMigration>, CoreError> {
    let old_relative = recording.artifact.relative_path.clone();
    let source = ledger.source(&recording.source_id)?;
    let Some(default_flat_relative) =
        legacy_flat_relative(recording, source.legacy_slot.as_deref())
    else {
        return Ok(None);
    };
    let old_path = root.join(&old_relative);
    let target_relative = choose_flat_target(root, &default_flat_relative, recording)?;
    let target_path = root.join(&target_relative);

    let old_exists = path_exists(root, &old_relative)?;
    let old_matches = regular_artifact_matches(root, &old_relative, recording)?;
    if old_exists && !old_matches {
        return Ok(None);
    }
    if old_matches {
        if !regular_artifact_matches(root, &target_relative, recording)? {
            copy_verified_artifact(root, &old_path, &target_path, recording, cancellation)?;
        }
        cancellation.check()?;
        trash.move_to_trash(&old_path)?;
    } else if !regular_artifact_matches(root, &target_relative, recording)? {
        return Ok(None);
    }

    ledger.relocate_verified_artifact(
        &recording.id,
        &old_relative,
        &target_relative,
        recording.artifact.byte_count,
        &recording.artifact.sha256,
    )?;
    remove_empty_legacy_directory(&old_path)?;
    Ok(Some(LayoutMigration {
        from: old_relative,
        to: target_relative,
    }))
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
    regular_file_matches(
        root,
        relative,
        recording.artifact.byte_count,
        &recording.artifact.sha256,
    )
}

fn regular_file_matches(
    root: &Path,
    relative: &Path,
    byte_count: u64,
    sha256: &str,
) -> Result<bool, CoreError> {
    let path = root.join(relative);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(CoreError::CopyFailed(error)),
    };
    if !metadata.file_type().is_file() || metadata.len() != byte_count {
        return Ok(false);
    }
    let canonical_path = fs::canonicalize(&path).map_err(CoreError::CopyFailed)?;
    if !canonical_path.starts_with(root) {
        return Err(CoreError::DestinationUnavailable);
    }
    let digest = hash_file(&canonical_path)?;
    Ok(digest.size == byte_count && digest.sha256 == sha256)
}

fn copy_verified_artifact(
    root: &Path,
    old_path: &Path,
    target_path: &Path,
    recording: &VerifiedRecording,
    cancellation: &CancellationToken,
) -> Result<(), CoreError> {
    copy_verified_file(
        root,
        old_path,
        target_path,
        recording.artifact.byte_count,
        &recording.artifact.sha256,
        cancellation,
    )
}

fn copy_verified_file(
    root: &Path,
    old_path: &Path,
    target_path: &Path,
    byte_count: u64,
    sha256: &str,
    cancellation: &CancellationToken,
) -> Result<(), CoreError> {
    cancellation.check()?;
    let parent = target_path
        .parent()
        .ok_or(CoreError::DestinationUnavailable)?;
    prepare_safe_directory(root, parent)?;
    let mut temporary = NamedTempFile::new_in(parent).map_err(CoreError::CopyFailed)?;
    let mut source = File::open(old_path).map_err(CoreError::CopyFailed)?;
    let copied = io::copy(&mut source, temporary.as_file_mut()).map_err(CoreError::CopyFailed)?;
    if copied != byte_count {
        return Err(CoreError::HashMismatch);
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(CoreError::SyncFailed)?;
    cancellation.check()?;
    let digest = hash_file(temporary.path())?;
    if digest.size != byte_count || digest.sha256 != sha256 {
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
    if !regular_file_matches(root, target_relative, byte_count, sha256)? {
        return Err(CoreError::HashMismatch);
    }
    Ok(())
}

fn with_hash_suffix(path: &Path, hash_prefix: &str) -> Result<PathBuf, CoreError> {
    let parent = path.parent().ok_or(CoreError::InvalidRequest)?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(CoreError::InvalidRequest)?;
    let suffixed = match (
        path.file_stem().and_then(|value| value.to_str()),
        path.extension().and_then(|value| value.to_str()),
    ) {
        (Some(stem), Some(extension)) if !stem.is_empty() && !extension.is_empty() => {
            format!("{stem}-{hash_prefix}.{extension}")
        }
        _ => format!("{file_name}-{hash_prefix}"),
    };
    Ok(parent.join(suffixed))
}

fn prepare_safe_directory(root: &Path, directory: &Path) -> Result<(), CoreError> {
    let relative = directory
        .strip_prefix(root)
        .map_err(|_| CoreError::DestinationUnavailable)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(CoreError::DestinationUnavailable);
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(CoreError::DestinationUnavailable),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(CoreError::CopyFailed)?;
            }
            Err(error) => return Err(CoreError::CopyFailed(error)),
        }
        let canonical = fs::canonicalize(&current).map_err(CoreError::CopyFailed)?;
        if !canonical.starts_with(root) {
            return Err(CoreError::DestinationUnavailable);
        }
    }
    Ok(())
}

fn remove_empty_legacy_ancestors(root: &Path, old_path: &Path) -> Result<(), CoreError> {
    let mut current = old_path.parent().ok_or(CoreError::InvalidRequest)?;
    while current != root && current.starts_with(root) {
        let mut entries = match fs::read_dir(current) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                current = current.parent().ok_or(CoreError::InvalidRequest)?;
                continue;
            }
            Err(error) => return Err(CoreError::CopyFailed(error)),
        };
        if entries.next().is_some() {
            break;
        }
        fs::remove_dir(current).map_err(CoreError::CopyFailed)?;
        current = current.parent().ok_or(CoreError::InvalidRequest)?;
    }
    Ok(())
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
