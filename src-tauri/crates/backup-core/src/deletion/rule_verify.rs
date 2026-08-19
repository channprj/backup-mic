//! Proving a rule's sources are still exactly what was backed up, before anything moves.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::artifact::{ConversionStatus, OutputFormat};
use crate::batch::{BatchPhase, M4A_PROFILE_ID};
use crate::error::CoreError;
use crate::filesystem::{is_safe_additional_relative_path, is_safe_relative_path};
use crate::ledger::Ledger;
use crate::rule::{BackupRule, compile_rule};
use crate::rule_scanner::{SelectedFileKind, scan_rule_once};

use super::evidence::{
    verify_additional_destination, verify_additional_source, verify_destination, verify_source,
};
use super::rule_execute::{RuleRetirementPlan, RuleRetirementTarget};
use super::{
    AdditionalDeletionCandidate, CompleteRuleDeletionSnapshot, DeletionCandidate,
    DeletionFaultPoint, DeletionFaults, DeletionProposal, FrozenRuleDeletionContext,
    RuleDeletionContext,
};

pub(super) fn freeze_rule_context(context: &RuleDeletionContext<'_>) -> FrozenRuleDeletionContext {
    FrozenRuleDeletionContext {
        source_id: context.source_id.clone(),
        rule_id: context.rule_id.clone(),
        rule_updated_at: context.rule_updated_at.to_owned(),
        authority: context.authority.clone(),
        destination_generation: context.destination_generation,
        scan_generation: context.scan_generation,
    }
}

pub(super) fn validate_rule_context(context: &RuleDeletionContext<'_>) -> Result<(), CoreError> {
    if context.source_id != &context.authority.source.id
        || context.rule_id != &context.authority.source.rule_id
        || context.rule_updated_at.trim().is_empty()
        || !context
            .authority
            .source
            .volume_uuid
            .eq_ignore_ascii_case(&context.authority.descriptor.volume_uuid)
        || context
            .authority
            .descriptor
            .mount_root
            .as_os_str()
            .is_empty()
        || !context
            .authority
            .descriptor
            .protocol
            .eq_ignore_ascii_case("USB")
        || context.authority.descriptor.is_internal
        || !context.authority.descriptor.is_removable
        || !context.authority.descriptor.is_writable
        || context.authority.descriptor.mount_generation == 0
        || context.destination_generation == 0
        || context.scan_generation == 0
    {
        return Err(CoreError::DeletionPreflightRefused);
    }
    Ok(())
}

pub(super) fn rule_context_matches(
    frozen: &FrozenRuleDeletionContext,
    current: &RuleDeletionContext<'_>,
) -> bool {
    validate_rule_context(current).is_ok()
        && frozen.source_id == *current.source_id
        && frozen.rule_id == *current.rule_id
        && frozen.rule_updated_at == current.rule_updated_at
        && frozen.authority == *current.authority
        && frozen.destination_generation == current.destination_generation
        && frozen.scan_generation == current.scan_generation
}

pub(super) fn rule_recording_candidates(
    snapshot: &CompleteRuleDeletionSnapshot,
) -> Vec<&DeletionCandidate> {
    snapshot
        .files
        .iter()
        .chain(
            snapshot
                .sessions
                .iter()
                .flat_map(|session| session.files.iter()),
        )
        .collect()
}

pub(super) fn rule_additional_candidates(
    snapshot: &CompleteRuleDeletionSnapshot,
) -> Vec<&AdditionalDeletionCandidate> {
    snapshot
        .additional_files
        .iter()
        .chain(
            snapshot
                .sessions
                .iter()
                .flat_map(|session| session.additional_files.iter()),
        )
        .collect()
}

pub(super) fn validate_rule_snapshot_shape(
    snapshot: &CompleteRuleDeletionSnapshot,
) -> Result<(), CoreError> {
    if snapshot.destination_root.as_os_str().is_empty() || snapshot.m4a_barrier_run_id.is_empty() {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let recordings = rule_recording_candidates(snapshot);
    let additional = rule_additional_candidates(snapshot);
    if recordings.is_empty() && additional.is_empty() {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let recording_ids = recordings
        .iter()
        .map(|candidate| candidate.recording_id.as_str())
        .collect::<BTreeSet<_>>();
    let additional_ids = additional
        .iter()
        .map(|candidate| candidate.additional_file_id.as_str())
        .collect::<BTreeSet<_>>();
    let paths = recordings
        .iter()
        .map(|candidate| candidate.source_relative_path.clone())
        .chain(
            additional
                .iter()
                .map(|candidate| candidate.source_relative_path.clone()),
        )
        .collect::<BTreeSet<_>>();
    let session_paths = snapshot
        .sessions
        .iter()
        .map(|session| session.relative_directory.clone())
        .collect::<BTreeSet<_>>();
    if recording_ids.len() != recordings.len()
        || additional_ids.len() != additional.len()
        || paths.len() != recordings.len() + additional.len()
        || session_paths.len() != snapshot.sessions.len()
    {
        return Err(CoreError::DeletionPreflightRefused);
    }
    for candidate in &recordings {
        if candidate.recording_id.is_empty()
            || !is_safe_relative_path(&candidate.source_relative_path)
            || !is_safe_relative_path(&candidate.destination_relative_path)
        {
            return Err(CoreError::DeletionPreflightRefused);
        }
    }
    for candidate in &additional {
        if candidate.additional_file_id.is_empty()
            || !is_safe_additional_relative_path(&candidate.source_relative_path)
            || !is_safe_additional_relative_path(&candidate.destination_relative_path)
        {
            return Err(CoreError::DeletionPreflightRefused);
        }
    }
    for session in &snapshot.sessions {
        if !is_safe_relative_path(&session.relative_directory)
            || session.files.is_empty() && session.additional_files.is_empty()
            || session.files.iter().any(|candidate| {
                !candidate
                    .source_relative_path
                    .starts_with(&session.relative_directory)
                    || candidate.source_relative_path.parent()
                        != Some(session.relative_directory.as_path())
            })
            || session.additional_files.iter().any(|candidate| {
                !candidate
                    .source_relative_path
                    .starts_with(&session.relative_directory)
                    || candidate.source_relative_path.parent()
                        != Some(session.relative_directory.as_path())
            })
        {
            return Err(CoreError::DeletionPreflightRefused);
        }
    }
    Ok(())
}

pub(super) fn verify_rule_ledger_authority(
    ledger: &Ledger,
    proposal: &DeletionProposal,
) -> Result<(), CoreError> {
    let run = ledger
        .batch_run_evidence(&proposal.snapshot.m4a_barrier_run_id)?
        .ok_or(CoreError::DeletionPreflightRefused)?;
    if run.source_id.as_ref() != Some(&proposal.frozen.source_id)
        || run.phase != BatchPhase::M4aCohortVerified
        || !run.frozen_preferences.m4a_conversion
        || run.m4a_profile_id.as_deref() != Some(M4A_PROFILE_ID)
    {
        return Err(CoreError::DeletionPreflightRefused);
    }
    for candidate in rule_recording_candidates(&proposal.snapshot) {
        let recording = ledger
            .verified_recording(&candidate.recording_id)?
            .ok_or(CoreError::DeletionPreflightRefused)?;
        if recording.source_id != proposal.frozen.source_id
            || recording.backup_run_id != proposal.snapshot.m4a_barrier_run_id
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
    for candidate in rule_additional_candidates(&proposal.snapshot) {
        let file = ledger
            .verified_additional_file(&candidate.additional_file_id)?
            .ok_or(CoreError::DeletionPreflightRefused)?;
        if file.source_id != proposal.frozen.source_id
            || file.backup_run_id != proposal.snapshot.m4a_barrier_run_id
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

pub(super) fn verify_complete_rule_snapshot(
    proposal: &DeletionProposal,
    rule: &BackupRule,
    faults: &dyn DeletionFaults,
) -> Result<RuleRetirementPlan, CoreError> {
    let compiled = compile_rule(rule.clone())?;
    let scan = scan_rule_once(
        &proposal.frozen.authority.descriptor.mount_root,
        &compiled,
        time::UtcOffset::UTC,
    )?;
    if scan.unsafe_session_count > 0 {
        return Err(CoreError::UnsafeSessionEntry);
    }
    let mut expected = BTreeMap::new();
    for candidate in &proposal.snapshot.files {
        expected.insert(
            candidate.source_relative_path.clone(),
            (None, SelectedFileKind::RecordingWav),
        );
    }
    for candidate in &proposal.snapshot.additional_files {
        expected.insert(
            candidate.source_relative_path.clone(),
            (None, SelectedFileKind::Companion),
        );
    }
    for session in &proposal.snapshot.sessions {
        for candidate in &session.files {
            expected.insert(
                candidate.source_relative_path.clone(),
                (
                    Some(session.relative_directory.clone()),
                    SelectedFileKind::RecordingWav,
                ),
            );
        }
        for candidate in &session.additional_files {
            expected.insert(
                candidate.source_relative_path.clone(),
                (
                    Some(session.relative_directory.clone()),
                    SelectedFileKind::Companion,
                ),
            );
        }
    }
    let observed = scan
        .files
        .into_iter()
        .map(|file| (file.relative_path, (file.session_relative_path, file.kind)))
        .collect::<BTreeMap<_, _>>();
    if observed != expected {
        return Err(CoreError::SourceChanged);
    }

    let source_root = fs::canonicalize(&proposal.frozen.authority.descriptor.mount_root)
        .map_err(CoreError::CopyFailed)?;
    let destination_root =
        fs::canonicalize(&proposal.snapshot.destination_root).map_err(CoreError::CopyFailed)?;
    if source_root.starts_with(&destination_root) || destination_root.starts_with(&source_root) {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let recordings = rule_recording_candidates(&proposal.snapshot);
    let additional = rule_additional_candidates(&proposal.snapshot);
    for (index, candidate) in recordings.iter().enumerate() {
        faults.check(DeletionFaultPoint::RevalidateSource(index))?;
        verify_source(&source_root, candidate).map_err(|_| CoreError::SourceChanged)?;
        faults.check(DeletionFaultPoint::RevalidateDestination(index))?;
        verify_destination(&destination_root, candidate)
            .map_err(|_| CoreError::DeletionPreflightRefused)?;
    }
    for (index, candidate) in additional.iter().enumerate() {
        let index = recordings
            .len()
            .checked_add(index)
            .ok_or(CoreError::InvalidRequest)?;
        faults.check(DeletionFaultPoint::RevalidateSource(index))?;
        verify_additional_source(&source_root, candidate).map_err(|_| CoreError::SourceChanged)?;
        faults.check(DeletionFaultPoint::RevalidateDestination(index))?;
        verify_additional_destination(&destination_root, candidate)
            .map_err(|_| CoreError::DeletionPreflightRefused)?;
    }

    let mut targets = Vec::new();
    let mut sessions = proposal.snapshot.sessions.iter().collect::<Vec<_>>();
    sessions.sort_by(|left, right| left.relative_directory.cmp(&right.relative_directory));
    for session in sessions {
        verify_rule_session_inventory(
            &source_root,
            &session.relative_directory,
            &session.files,
            &session.additional_files,
        )?;
        let mut recording_ids = session
            .files
            .iter()
            .map(|candidate| candidate.recording_id.clone())
            .collect::<Vec<_>>();
        recording_ids.sort();
        let mut additional_file_ids = session
            .additional_files
            .iter()
            .map(|candidate| candidate.additional_file_id.clone())
            .collect::<Vec<_>>();
        additional_file_ids.sort();
        targets.push(RuleRetirementTarget::Session {
            relative_directory: session.relative_directory.clone(),
            recording_ids,
            additional_file_ids,
        });
    }
    let mut files = proposal.snapshot.files.iter().collect::<Vec<_>>();
    files.sort_by(|left, right| left.source_relative_path.cmp(&right.source_relative_path));
    targets.extend(
        files
            .into_iter()
            .map(|candidate| RuleRetirementTarget::RecordingFile {
                relative_path: candidate.source_relative_path.clone(),
                recording_id: candidate.recording_id.clone(),
            }),
    );
    let mut additional_files = proposal
        .snapshot
        .additional_files
        .iter()
        .collect::<Vec<_>>();
    additional_files
        .sort_by(|left, right| left.source_relative_path.cmp(&right.source_relative_path));
    targets.extend(additional_files.into_iter().map(|candidate| {
        RuleRetirementTarget::AdditionalFile {
            relative_path: candidate.source_relative_path.clone(),
            additional_file_id: candidate.additional_file_id.clone(),
        }
    }));
    Ok(RuleRetirementPlan {
        targets,
        source_root,
    })
}

pub(super) fn verify_rule_session_inventory(
    source_root: &Path,
    session: &Path,
    candidates: &[DeletionCandidate],
    additional_files: &[AdditionalDeletionCandidate],
) -> Result<(), CoreError> {
    let session_path = source_root.join(session);
    let metadata = fs::symlink_metadata(&session_path).map_err(CoreError::CopyFailed)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(CoreError::UnsafeSessionEntry);
    }
    let canonical_session = fs::canonicalize(&session_path).map_err(CoreError::CopyFailed)?;
    if canonical_session
        .strip_prefix(source_root)
        .ok()
        .filter(|relative| *relative == session)
        .is_none()
    {
        return Err(CoreError::UnsafeSessionEntry);
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
            return Err(CoreError::UnsafeSessionEntry);
        };
        if file_type.is_symlink() || !file_type.is_file() {
            return Err(CoreError::UnsafeSessionEntry);
        }
        observed.insert(session.join(name));
    }
    if observed != expected {
        return Err(CoreError::SourceChanged);
    }
    Ok(())
}
