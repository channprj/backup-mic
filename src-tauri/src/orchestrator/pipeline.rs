//! Driving one backup over every matched recorder, and reducing the per-source results.

use backup_core::audit_log::{AuditDurability, AuditEvent, AuditLevel, AuditValue};
use backup_core::backup::{CopyFaultPoint, CopyFaults, cleanup_owned_partials, ensure_capacity};
use backup_core::clock::Clock;
use backup_core::deletion::{CompleteRuleDeletionSnapshot, TrashAdapter};
use backup_core::error::{CoreError, PublicError};
use backup_core::layout::migrate_verified_dji_calendar_layout_resilient;
use backup_core::rule::{BackupRule, compile_rule};
use backup_core::rule_scanner::{begin_rule_stable_scan, finish_rule_stable_scan};
use backup_core::scanner::STABILITY_INTERVAL;
use backup_core::source::{MountedSourceAuthority, SourceId};
use backup_core::state::BackupPhase;

use crate::app_state::{AppState, RuleDeletionState};
use crate::clock;
use crate::platform::macos::audio::AudioTools;
use crate::rule_runtime::MatchedSource;

use super::copying::{failed_without_run, process_prepared_source};
use super::planning::prepare_matched_source;

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

pub(super) struct ScopedCopyFaults<'a> {
    pub(super) source_id: &'a SourceId,
    pub(super) faults: &'a dyn SourceCopyFaults,
}

impl CopyFaults for ScopedCopyFaults<'_> {
    fn check(&self, point: CopyFaultPoint) -> Result<(), CoreError> {
        self.faults.check(self.source_id, point)
    }
}

pub fn run_matched_sources_with_adapters(
    state: &AppState,
    matched_sources: &[MatchedSource],
    audio_tools: &dyn AudioTools,
    source_faults: &dyn SourceCopyFaults,
    destination_trash: &dyn TrashAdapter,
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
    let migrations = {
        let mut ledger = state.ledger.lock();
        migrate_verified_dji_calendar_layout_resilient(
            &destination,
            &mut ledger,
            destination_trash,
            cancellation,
            &mut |error| {
                state.report_failure(
                    "dji_calendar_migration",
                    "artifact_migration",
                    error,
                    None,
                    None,
                );
            },
        )?
    };
    for migration in migrations {
        let source = migration.from.to_string_lossy().into_owned();
        let output = migration.to.to_string_lossy().into_owned();
        let fields = [
            ("source", AuditValue::Text(source.as_str())),
            ("output", AuditValue::Text(output.as_str())),
            ("mode", AuditValue::Text("dji_calendar")),
        ];
        if let Err(error) = state.append_audit(
            &AuditEvent {
                occurred_at: clock::local_now(),
                level: AuditLevel::Info,
                code: "archive.migration_complete",
                transmitter: None,
                fields: &fields,
            },
            AuditDurability::Buffered,
        ) {
            state.report_failure("dji_calendar_migration", "audit_log", &error, None, None);
        }
    }
    let preferences = state.frozen_preferences();
    let local_offset = clock::local_offset();
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
            Err(error) => {
                let error = classify_source_failure(
                    source_authority_is_available(state, &matched.authority),
                    error,
                );
                outcomes.push(failed_without_run(matched.authority.source.id, error));
            }
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
            Err(error) => {
                let error = classify_source_failure(
                    source_authority_is_available(state, &matched.authority),
                    error,
                );
                outcomes.push(failed_without_run(matched.authority.source.id, error));
            }
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
            state.free_space_reserve_bytes(),
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
            destination_trash,
            cancellation,
        )?);
    }
    outcomes.sort_by(|left, right| left.source_id.cmp(&right.source_id));
    install_rule_deletion_evidence(state, &outcomes, destination_generation);
    Ok(outcomes)
}

pub(super) fn install_rule_deletion_evidence(
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

pub(super) fn source_failure_is_process_wide(error: &CoreError) -> bool {
    matches!(
        error,
        CoreError::DestinationUnavailable
            | CoreError::LedgerCorrupt
            | CoreError::Ledger(_)
            | CoreError::LedgerIo(_)
            | CoreError::Cancelled
    )
}

pub(super) fn classify_source_failure(authority_is_current: bool, error: CoreError) -> CoreError {
    if !authority_is_current && !matches!(error, CoreError::Cancelled) {
        CoreError::DeviceRemoved
    } else {
        error
    }
}

pub(super) fn source_authority_is_available(
    state: &AppState,
    authority: &MountedSourceAuthority,
) -> bool {
    state.source_authority_is_current(authority) && authority.descriptor.mount_root.is_dir()
}

pub(super) fn revalidate_run_context(
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::super::{run::BackupRunResult, shared::source_failure_stage};
    use super::*;

    #[test]
    fn manual_reconnect_classifies_io_failure_only_after_authority_is_lost() {
        let current = classify_source_failure(
            true,
            CoreError::CopyFailed(std::io::Error::from(std::io::ErrorKind::NotFound)),
        );
        let removed = classify_source_failure(
            false,
            CoreError::CopyFailed(std::io::Error::from(std::io::ErrorKind::NotFound)),
        );
        let cancelled = classify_source_failure(false, CoreError::Cancelled);

        assert_eq!(current.diagnostic_code(), "copy_failed");
        assert!(matches!(removed, CoreError::DeviceRemoved));
        assert!(matches!(cancelled, CoreError::Cancelled));
    }

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

    #[test]
    fn a_real_failure_is_not_hidden_by_a_simultaneous_disconnect() {
        let interrupted_sources = [SourceId::new()].into_iter().collect::<BTreeSet<_>>();

        assert!(
            BackupRunResult {
                interrupted_sources: interrupted_sources.clone(),
                has_real_failure: false,
            }
            .should_resume_after_reconnect()
        );
        assert!(
            !BackupRunResult {
                interrupted_sources,
                has_real_failure: true,
            }
            .should_resume_after_reconnect()
        );
    }
}
