//! The pre-rule-engine retirement path, kept for ledgers written by earlier versions.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use crate::artifact::{ConversionStatus, OutputFormat};
use crate::batch::{BatchPhase, M4A_PROFILE_ID};
use crate::error::CoreError;
use crate::filesystem::{
    is_recognized_session_name, is_safe_additional_relative_path, is_safe_relative_path,
};
use crate::ledger::Ledger;
use crate::scanner::scan_once;

use super::evidence::{
    verify_additional_destination, verify_additional_source, verify_destination, verify_source,
};
use super::{
    AdditionalDeletionCandidate, DeletionCandidate, DeletionContext, DeletionFaultPoint,
    DeletionFaults, PrivateProposal, RetirementPlan, RetirementTarget, TrashAdapter,
};

pub(super) fn verify_complete_snapshot(
    context: &DeletionContext,
    candidates: &[DeletionCandidate],
    additional_files: &[AdditionalDeletionCandidate],
    faults: &dyn DeletionFaults,
) -> Result<RetirementPlan, CoreError> {
    let expected_source_paths = candidates
        .iter()
        .map(|candidate| candidate.source_relative_path.clone())
        .chain(
            additional_files
                .iter()
                .map(|candidate| candidate.source_relative_path.clone()),
        )
        .collect::<BTreeSet<_>>();
    let scan = scan_once(
        &context.source_root,
        context.transmitter,
        time::UtcOffset::UTC,
    )?;
    if scan.issues.iter().any(|issue| {
        matches!(
            issue,
            crate::scanner::ScanIssue::TransmitterPrefixMismatch
                | crate::scanner::ScanIssue::UnsafeSessionEntry
        )
    }) {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let observed_source_paths = scan
        .recordings
        .into_iter()
        .map(|recording| recording.relative_path)
        .chain(
            scan.additional_files
                .into_iter()
                .map(|file| file.relative_path),
        )
        .collect::<BTreeSet<_>>();
    if observed_source_paths != expected_source_paths {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let source_root = fs::canonicalize(&context.source_root).map_err(CoreError::CopyFailed)?;
    let destination_root =
        fs::canonicalize(&context.destination_root).map_err(CoreError::CopyFailed)?;
    let mut session_candidates: HashMap<PathBuf, Vec<&DeletionCandidate>> = HashMap::new();
    let mut session_additional: HashMap<PathBuf, Vec<&AdditionalDeletionCandidate>> =
        HashMap::new();
    let mut root_candidates = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if !is_safe_relative_path(&candidate.source_relative_path)
            || !is_safe_relative_path(&candidate.destination_relative_path)
        {
            return Err(CoreError::DeletionPreflightRefused);
        }
        faults.check(DeletionFaultPoint::RevalidateSource(index))?;
        verify_source(&source_root, candidate)?;
        faults.check(DeletionFaultPoint::RevalidateDestination(index))?;
        verify_destination(&destination_root, candidate)?;
        let mut components = candidate.source_relative_path.components();
        let first = components
            .next()
            .ok_or(CoreError::DeletionPreflightRefused)?;
        let second = components.next();
        if components.next().is_some() {
            return Err(CoreError::DeletionPreflightRefused);
        }
        let first = PathBuf::from(first.as_os_str());
        match second {
            None => root_candidates.push(candidate),
            Some(_) if is_recognized_session_name(&first) => {
                session_candidates.entry(first).or_default().push(candidate);
            }
            Some(_) => return Err(CoreError::DeletionPreflightRefused),
        }
    }
    for (index, candidate) in additional_files.iter().enumerate() {
        if !is_safe_additional_relative_path(&candidate.source_relative_path)
            || !is_safe_additional_relative_path(&candidate.destination_relative_path)
        {
            return Err(CoreError::DeletionPreflightRefused);
        }
        faults.check(DeletionFaultPoint::RevalidateSource(
            candidates.len() + index,
        ))?;
        verify_additional_source(&source_root, candidate)?;
        faults.check(DeletionFaultPoint::RevalidateDestination(
            candidates.len() + index,
        ))?;
        verify_additional_destination(&destination_root, candidate)?;
        let mut components = candidate.source_relative_path.components();
        let first = components
            .next()
            .ok_or(CoreError::DeletionPreflightRefused)?;
        if components.next().is_none() || components.next().is_some() {
            return Err(CoreError::DeletionPreflightRefused);
        }
        let session = PathBuf::from(first.as_os_str());
        if !is_recognized_session_name(&session) {
            return Err(CoreError::DeletionPreflightRefused);
        }
        session_additional
            .entry(session)
            .or_default()
            .push(candidate);
    }
    let mut targets = Vec::new();
    let mut sessions: Vec<_> = session_candidates.into_iter().collect();
    sessions.sort_by(|left, right| left.0.cmp(&right.0));
    for (session, grouped) in sessions {
        let additional = session_additional.remove(&session).unwrap_or_default();
        verify_session_inventory(&source_root, &session, &grouped, &additional)?;
        let mut recording_ids = grouped
            .into_iter()
            .map(|candidate| candidate.recording_id.clone())
            .collect::<Vec<_>>();
        recording_ids.sort();
        let mut additional_file_ids = additional
            .into_iter()
            .map(|candidate| candidate.additional_file_id.clone())
            .collect::<Vec<_>>();
        additional_file_ids.sort();
        targets.push(RetirementTarget::Session {
            relative_directory: session,
            recording_ids,
            additional_file_ids,
        });
    }
    if !session_additional.is_empty() {
        return Err(CoreError::DeletionPreflightRefused);
    }
    root_candidates
        .sort_by(|left, right| left.source_relative_path.cmp(&right.source_relative_path));
    targets.extend(
        root_candidates
            .into_iter()
            .map(|candidate| RetirementTarget::RootFile {
                relative_path: candidate.source_relative_path.clone(),
                recording_id: candidate.recording_id.clone(),
            }),
    );
    let byte_count = candidates.iter().try_fold(0_u64, |total, candidate| {
        total
            .checked_add(candidate.source_size)
            .ok_or(CoreError::DeletionPreflightRefused)
    })?;
    let byte_count = additional_files
        .iter()
        .try_fold(byte_count, |total, candidate| {
            total
                .checked_add(candidate.source_size)
                .ok_or(CoreError::DeletionPreflightRefused)
        })?;
    Ok(RetirementPlan {
        transmitter: context.transmitter,
        targets,
        recording_count: candidates.len(),
        additional_file_count: additional_files.len(),
        byte_count,
    })
}

pub(super) fn verify_ledger_authority(
    ledger: &Ledger,
    proposal: &PrivateProposal,
) -> Result<(), CoreError> {
    let run = ledger
        .batch_run_evidence(&proposal.m4a_barrier_run_id)?
        .ok_or(CoreError::DeletionPreflightRefused)?;
    if run.source_id.as_ref() != Some(&proposal.context.source_id)
        || run.phase != BatchPhase::M4aCohortVerified
        || !run.frozen_preferences.m4a_conversion
        || run.m4a_profile_id.as_deref() != Some(M4A_PROFILE_ID)
    {
        return Err(CoreError::DeletionPreflightRefused);
    }
    for candidate in &proposal.candidates {
        let recording = ledger
            .verified_recording(&candidate.recording_id)?
            .ok_or(CoreError::DeletionPreflightRefused)?;
        if recording.source_id != proposal.context.source_id
            || recording.backup_run_id != proposal.m4a_barrier_run_id
            || recording.source_relative_path != candidate.source_relative_path
            || recording.source_size != candidate.source_size
            || recording.source_mtime_ns != candidate.source_mtime_ns
            || recording.source_sha256 != candidate.source_sha256
            || recording.artifact.relative_path != candidate.destination_relative_path
            || recording.artifact.byte_count != candidate.destination_size
            || recording.artifact.sha256 != candidate.destination_sha256
            || recording.artifact.format != OutputFormat::M4a
            || recording.conversion_status != ConversionStatus::Complete
            || recording
                .artifact
                .audio
                .as_ref()
                .is_none_or(|audio| audio.codec != "aac")
        {
            return Err(CoreError::DeletionPreflightRefused);
        }
    }
    for candidate in &proposal.additional_files {
        let file = ledger
            .verified_additional_file(&candidate.additional_file_id)?
            .ok_or(CoreError::DeletionPreflightRefused)?;
        if file.source_id != proposal.context.source_id
            || file.backup_run_id != proposal.m4a_barrier_run_id
            || file.source_relative_path != candidate.source_relative_path
            || file.source_size != candidate.source_size
            || file.source_mtime_ns != candidate.source_mtime_ns
            || file.source_sha256 != candidate.source_sha256
            || file.artifact_relative_path != candidate.destination_relative_path
            || file.artifact_size != candidate.destination_size
            || file.artifact_sha256 != candidate.destination_sha256
        {
            return Err(CoreError::DeletionPreflightRefused);
        }
    }
    Ok(())
}

pub(super) fn verify_session_inventory(
    source_root: &Path,
    session: &Path,
    candidates: &[&DeletionCandidate],
    additional_files: &[&AdditionalDeletionCandidate],
) -> Result<(), CoreError> {
    let session_path = source_root.join(session);
    let metadata = fs::symlink_metadata(&session_path).map_err(CoreError::CopyFailed)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let canonical_session = fs::canonicalize(&session_path).map_err(CoreError::CopyFailed)?;
    if canonical_session.parent() != Some(source_root) {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let expected = candidates
        .iter()
        .map(|candidate| candidate.source_relative_path.clone())
        .chain(
            additional_files
                .iter()
                .map(|candidate| candidate.source_relative_path.clone()),
        )
        .collect::<BTreeSet<_>>();
    let mut observed = BTreeSet::new();
    for entry in fs::read_dir(&canonical_session).map_err(CoreError::CopyFailed)? {
        let entry = entry.map_err(CoreError::CopyFailed)?;
        let file_type = entry.file_type().map_err(CoreError::CopyFailed)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(CoreError::DeletionPreflightRefused);
        };
        if file_type.is_symlink() || !file_type.is_file() {
            return Err(CoreError::DeletionPreflightRefused);
        }
        observed.insert(session.join(name));
    }
    if observed != expected {
        return Err(CoreError::DeletionPreflightRefused);
    }
    Ok(())
}

pub(super) fn target_relative_path(target: &RetirementTarget) -> &Path {
    match target {
        RetirementTarget::Session {
            relative_directory, ..
        } => relative_directory,
        RetirementTarget::RootFile { relative_path, .. } => relative_path,
    }
}

pub(super) fn target_recording_ids(target: &RetirementTarget) -> Vec<&str> {
    match target {
        RetirementTarget::Session { recording_ids, .. } => {
            recording_ids.iter().map(String::as_str).collect()
        }
        RetirementTarget::RootFile { recording_id, .. } => vec![recording_id],
    }
}

pub(super) fn target_additional_file_ids(target: &RetirementTarget) -> Vec<&str> {
    match target {
        RetirementTarget::Session {
            additional_file_ids,
            ..
        } => additional_file_ids.iter().map(String::as_str).collect(),
        RetirementTarget::RootFile { .. } => Vec::new(),
    }
}

pub(super) fn target_contains_recording(target: &RetirementTarget, recording_id: &str) -> bool {
    match target {
        RetirementTarget::Session { recording_ids, .. } => {
            recording_ids.iter().any(|value| value == recording_id)
        }
        RetirementTarget::RootFile {
            recording_id: value,
            ..
        } => value == recording_id,
    }
}

pub(super) fn target_contains_additional_file(
    target: &RetirementTarget,
    additional_file_id: &str,
) -> bool {
    match target {
        RetirementTarget::Session {
            additional_file_ids,
            ..
        } => additional_file_ids
            .iter()
            .any(|value| value == additional_file_id),
        RetirementTarget::RootFile { .. } => false,
    }
}

pub fn reconcile_legacy_empty_sessions(
    context: &DeletionContext,
    ledger: &mut Ledger,
    trash: &dyn TrashAdapter,
    moved_at: &str,
) -> Result<u64, CoreError> {
    let source_root = fs::canonicalize(&context.source_root).map_err(CoreError::CopyFailed)?;
    let mut sessions: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for recording in ledger.legacy_retired_recordings(&context.source_id)? {
        let mut components = recording.source_relative_path.components();
        let Some(session) = components.next() else {
            continue;
        };
        if components.next().is_none() || components.next().is_some() {
            continue;
        }
        let session = PathBuf::from(session.as_os_str());
        if is_recognized_session_name(&session) {
            sessions
                .entry(session)
                .or_default()
                .push(recording.recording_id);
        }
    }
    let mut moved = 0_u64;
    for (session, recording_ids) in sessions {
        let session_path = source_root.join(&session);
        let metadata = match fs::symlink_metadata(&session_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(CoreError::CopyFailed(error)),
        };
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        let canonical_session = fs::canonicalize(&session_path).map_err(CoreError::CopyFailed)?;
        if canonical_session.parent() != Some(source_root.as_path()) {
            continue;
        }
        if fs::read_dir(&canonical_session)
            .map_err(CoreError::CopyFailed)?
            .next()
            .transpose()
            .map_err(CoreError::CopyFailed)?
            .is_some()
        {
            continue;
        }
        trash.move_to_trash(&canonical_session)?;
        ledger.record_legacy_session_moved_to_trash(&recording_ids, &session, moved_at)?;
        moved = moved.checked_add(1).ok_or(CoreError::InvalidRequest)?;
    }
    Ok(moved)
}
