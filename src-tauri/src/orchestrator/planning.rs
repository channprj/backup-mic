//! Turning a mounted recorder into the exact set of copies a run will attempt.

use std::collections::BTreeSet;

use backup_core::artifact::OutputFormat;
use backup_core::batch::{BatchItemKey, CopyBarrier};
use backup_core::destination::{
    AdditionalFilePlan, DestinationDisposition, DestinationPlan, RuleDestinationPlan,
    plan_rule_file_from_metadata,
};
use backup_core::error::CoreError;
use backup_core::ledger::VerifiedRecording;
use backup_core::recording::{
    AdditionalFileObservation, ParsedRecordingName, RecordingObservation,
};
use backup_core::rule::BackupRule;
use backup_core::rule_scanner::{RuleScanResult, SelectedFileKind};
use backup_core::source::{MountedSourceAuthority, SourceId};

use crate::app_state::AppState;
use crate::rule_runtime::MatchedSource;

pub(super) struct PreparedRuleRecording {
    pub(super) plan: DestinationPlan,
    pub(super) existing: Option<VerifiedRecording>,
    pub(super) replace_existing_m4a: bool,
}

pub(super) struct PreparedSource {
    pub(super) source_id: SourceId,
    pub(super) authority: MountedSourceAuthority,
    pub(super) rule: BackupRule,
    pub(super) source_root: std::path::PathBuf,
    pub(super) scan: RuleScanResult,
    pub(super) recording_plans: Vec<PreparedRuleRecording>,
    pub(super) companion_plans: Vec<AdditionalFilePlan>,
    pub(super) copy_barrier: CopyBarrier,
    pub(super) required_copy_bytes: u64,
    pub(super) required_capacity_bytes: u64,
}

pub(super) fn prepare_matched_source(
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

pub(super) fn destination_plan_from_rule(plan: RuleDestinationPlan) -> DestinationPlan {
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

pub(super) fn additional_plan_from_rule(plan: RuleDestinationPlan) -> AdditionalFilePlan {
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
