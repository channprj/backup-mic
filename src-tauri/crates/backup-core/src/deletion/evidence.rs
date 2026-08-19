//! Re-hashing one file against its stored evidence. Every retirement path ends here.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::CoreError;
use crate::filesystem::modified_nanos;
use crate::hash::hash_file;

use super::{AdditionalDeletionCandidate, DeletionCandidate};

pub(super) fn verify_source(
    root: &Path,
    candidate: &DeletionCandidate,
) -> Result<PathBuf, CoreError> {
    let path = root.join(&candidate.source_relative_path);
    let metadata = fs::symlink_metadata(&path).map_err(CoreError::CopyFailed)?;
    if !metadata.file_type().is_file()
        || metadata.len() != candidate.source_size
        || modified_nanos(&metadata)? != candidate.source_mtime_ns
    {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let canonical_path = fs::canonicalize(&path).map_err(CoreError::CopyFailed)?;
    if !canonical_path.starts_with(root) {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let digest = hash_file(&canonical_path)?;
    if digest.size != candidate.source_size || digest.sha256 != candidate.source_sha256 {
        return Err(CoreError::DeletionPreflightRefused);
    }
    Ok(canonical_path)
}

pub(super) fn verify_destination(
    root: &Path,
    candidate: &DeletionCandidate,
) -> Result<(), CoreError> {
    let path = root.join(&candidate.destination_relative_path);
    let metadata = fs::symlink_metadata(&path).map_err(CoreError::CopyFailed)?;
    if !metadata.file_type().is_file() || metadata.len() != candidate.destination_size {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let canonical_path = fs::canonicalize(&path).map_err(CoreError::CopyFailed)?;
    if !canonical_path.starts_with(root) {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let digest = hash_file(&canonical_path)?;
    if digest.size != candidate.destination_size || digest.sha256 != candidate.destination_sha256 {
        return Err(CoreError::DeletionPreflightRefused);
    }
    Ok(())
}

pub(super) fn verify_additional_source(
    root: &Path,
    candidate: &AdditionalDeletionCandidate,
) -> Result<PathBuf, CoreError> {
    verify_file_evidence(
        root,
        &candidate.source_relative_path,
        candidate.source_size,
        Some(candidate.source_mtime_ns),
        &candidate.source_sha256,
    )
}

pub(super) fn verify_additional_destination(
    root: &Path,
    candidate: &AdditionalDeletionCandidate,
) -> Result<(), CoreError> {
    verify_file_evidence(
        root,
        &candidate.destination_relative_path,
        candidate.destination_size,
        None,
        &candidate.destination_sha256,
    )?;
    Ok(())
}

pub(super) fn verify_file_evidence(
    root: &Path,
    relative_path: &Path,
    expected_size: u64,
    expected_mtime_ns: Option<i128>,
    expected_sha256: &str,
) -> Result<PathBuf, CoreError> {
    let path = root.join(relative_path);
    let metadata = fs::symlink_metadata(&path).map_err(CoreError::CopyFailed)?;
    if !metadata.file_type().is_file()
        || metadata.len() != expected_size
        || expected_mtime_ns.is_some_and(|expected| {
            modified_nanos(&metadata).map_or(true, |observed| observed != expected)
        })
    {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let canonical_path = fs::canonicalize(&path).map_err(CoreError::CopyFailed)?;
    if !canonical_path.starts_with(root) {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let digest = hash_file(&canonical_path)?;
    if digest.size != expected_size || digest.sha256 != expected_sha256 {
        return Err(CoreError::DeletionPreflightRefused);
    }
    Ok(canonical_path)
}
