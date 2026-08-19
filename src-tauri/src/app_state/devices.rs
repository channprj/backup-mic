//! Matching a mounted volume to a rule, and tracking which recorders are present.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use backup_core::audit_log::{AuditDurability, AuditEvent, AuditLevel, AuditValue};
use backup_core::deletion::ProposalInvalidation;
use backup_core::error::CoreError;
use backup_core::events::ActivitySeverity;
use backup_core::rule::{BackupRule, DeviceConstraintProfile};
use backup_core::source::{MountedSourceAuthority, SourceId, SourceRecord};
use backup_core::state::{BackupPhase, DeletionPhase, Transmitter};
use tauri::AppHandle;

use crate::dto::{AppSnapshotDto, SourceSnapshotDto};
use crate::platform::device_registry::{MountedVolume, VolumeLifecycleEvent};
use crate::rule_runtime::{MatchedSource, RuleVolumeMatch, match_mounted_volume};

use super::operations::backup_should_schedule;
use super::presentation::update_source;
use super::reporting::activity_entry;
use super::{AppState, publish_locked};

impl AppState {
    pub fn handle_lifecycle(
        &self,
        app: &AppHandle,
        event: VolumeLifecycleEvent,
    ) -> Option<SourceId> {
        match event {
            VolumeLifecycleEvent::Mounted(mounted) => self.handle_mounted(app, mounted),
            VolumeLifecycleEvent::Unmounted {
                volume_uuid,
                mount_generation,
            } => {
                self.proposals
                    .lock()
                    .invalidate(ProposalInvalidation::DeviceDisappeared);
                let mut runtime = self.runtime.lock();
                runtime.observed.remove(&volume_uuid);
                let removed_source = runtime
                    .matched
                    .iter()
                    .find(|(_, matched)| {
                        matched
                            .authority
                            .descriptor
                            .volume_uuid
                            .eq_ignore_ascii_case(&volume_uuid)
                            && matched.authority.descriptor.mount_generation == mount_generation
                    })
                    .map(|(source_id, matched)| (source_id.clone(), matched.clone()));
                if let Some((source_id, _)) = &removed_source {
                    runtime.matched.remove(source_id);
                    runtime.rule_deletions.remove(source_id);
                    runtime.rule_scan_generations.remove(source_id);
                    if runtime.awaiting_rule_deletion.as_ref() == Some(source_id) {
                        runtime.awaiting_rule_deletion = None;
                    }
                    update_source(&mut runtime.snapshot, source_id, |snapshot| {
                        snapshot.mounted = false;
                        snapshot.phase = BackupPhase::Idle;
                        snapshot.deletion_ready = false;
                        snapshot.retirement_outcome = DeletionPhase::Inactive;
                    });
                }
                publish_locked(app, &mut runtime);
                drop(runtime);
                if let Some((source_id, matched)) = removed_source {
                    let transmitter = legacy_transmitter(&matched.authority.source);
                    if let Err(error) = self.record_activity(
                        app,
                        activity_entry("device_removed", source_id, ActivitySeverity::Warning),
                    ) {
                        self.report_failure(
                            "device_lifecycle",
                            "activity_persistence",
                            &error,
                            transmitter,
                            None,
                        );
                    }
                    let fields = [("reason", AuditValue::Text("device_removed"))];
                    if self.runtime.lock().destination_configured
                        && let Err(error) = self.append_audit(
                            &AuditEvent {
                                occurred_at: crate::clock::local_now(),
                                level: AuditLevel::Warning,
                                code: "device.removed",
                                transmitter,
                                fields: &fields,
                            },
                            AuditDurability::Buffered,
                        )
                    {
                        self.report_failure(
                            "device_lifecycle",
                            "audit_log",
                            &error,
                            transmitter,
                            None,
                        );
                        self.set_error(app, error, transmitter);
                    }
                }
                None
            }
        }
    }

    fn handle_mounted(&self, app: &AppHandle, mounted: MountedVolume) -> Option<SourceId> {
        let volume_uuid = mounted.descriptor.volume_uuid.clone();
        self.runtime
            .lock()
            .observed
            .insert(volume_uuid, mounted.clone());
        let result = {
            let mut ledger = self.ledger.lock();
            let rules = match ledger.backup_rules(false) {
                Ok(rules) => rules,
                Err(error) => {
                    drop(ledger);
                    self.set_error(app, error, None);
                    return None;
                }
            };
            match_mounted_volume(
                &mut ledger,
                &mounted,
                &rules,
                crate::clock::local_offset(),
                &crate::clock::now_string(),
            )
        };
        let mut runtime = self.runtime.lock();
        let mut rejected_error = None;
        let matched_source = match result {
            Ok(RuleVolumeMatch::Matched(matched)) => {
                let matched = *matched;
                let source_id = matched.authority.source.id.clone();
                let source = matched.authority.source.clone();
                let rule = matched.rule.clone();
                runtime.matched.insert(source_id.clone(), matched);
                let should_schedule = backup_should_schedule(
                    runtime.preferences.automatic_backup,
                    self.manual_backup_should_start_for(&source_id),
                    runtime.destination_configured,
                    runtime.snapshot.setup_state,
                    runtime.matched.len(),
                );
                let source_phase = mounted_source_phase(should_schedule);
                upsert_source_snapshot(&mut runtime.snapshot, &source, &rule, true, source_phase);
                runtime.snapshot.phase = source_phase;
                runtime.snapshot.message_code = "device_detected".to_owned();
                runtime.snapshot.error = None;
                runtime.snapshot.failure_stage = None;
                Some((source_id, should_schedule))
            }
            Ok(RuleVolumeMatch::Conflict { .. }) => {
                let mut error = CoreError::InvalidRule.public(None);
                error.message_code = "rule_conflict".to_owned();
                rejected_error = Some(error.clone());
                runtime.snapshot.phase = BackupPhase::Error;
                runtime.snapshot.message_code = error.message_code.clone();
                runtime.snapshot.error = Some(error);
                None
            }
            Ok(RuleVolumeMatch::Rejected(error)) => {
                rejected_error = Some(error.clone());
                runtime.snapshot.phase = BackupPhase::Error;
                runtime.snapshot.message_code = error.message_code.clone();
                runtime.snapshot.error = Some(error);
                None
            }
            Ok(RuleVolumeMatch::Unrelated) => None,
            Err(error) => {
                let public = error.public(None);
                rejected_error = Some(public.clone());
                runtime.snapshot.phase = BackupPhase::Error;
                runtime.snapshot.message_code = public.message_code.clone();
                runtime.snapshot.error = Some(public);
                None
            }
        };
        let should_schedule = matched_source
            .as_ref()
            .and_then(|(source_id, should_schedule)| should_schedule.then(|| source_id.clone()));
        publish_locked(app, &mut runtime);
        drop(runtime);
        if let Some(error) = rejected_error {
            self.report_public_failure("device_lifecycle", "identity_validation", &error, None);
        }
        if let Some((source_id, _)) = matched_source {
            let transmitter = runtime_legacy_transmitter(self, &source_id);
            if let Err(error) = self.record_activity(
                app,
                activity_entry("device_detected", source_id.clone(), ActivitySeverity::Info),
            ) {
                self.report_failure(
                    "device_lifecycle",
                    "activity_persistence",
                    &error,
                    transmitter,
                    None,
                );
            }
            if self.runtime.lock().destination_configured
                && let Err(error) = self.append_audit(
                    &AuditEvent {
                        occurred_at: crate::clock::local_now(),
                        level: AuditLevel::Info,
                        code: "device.detected",
                        transmitter,
                        fields: &[],
                    },
                    AuditDurability::Buffered,
                )
            {
                self.report_failure("device_lifecycle", "audit_log", &error, transmitter, None);
                self.set_error(app, error, transmitter);
            }
        }
        should_schedule
    }

    pub fn matched_sources(&self) -> HashMap<SourceId, MatchedSource> {
        self.runtime.lock().matched.clone()
    }

    pub(crate) fn source_authority_is_current(&self, authority: &MountedSourceAuthority) -> bool {
        self.runtime
            .lock()
            .matched
            .get(&authority.source.id)
            .is_some_and(|matched| matched.authority == *authority)
    }

    pub fn install_matched_source_for_state(&self, matched: MatchedSource) {
        self.runtime
            .lock()
            .matched
            .insert(matched.authority.source.id.clone(), matched);
    }

    pub fn upsert_source_for_state(
        &self,
        source: &SourceRecord,
        seen_at: &str,
    ) -> Result<(), CoreError> {
        let rule = {
            let mut ledger = self.ledger.lock();
            ledger.upsert_source(source, seen_at)?;
            ledger
                .backup_rule(&source.rule_id)?
                .ok_or(CoreError::LedgerCorrupt)?
        };
        let mut runtime = self.runtime.lock();
        upsert_source_snapshot(
            &mut runtime.snapshot,
            source,
            &rule,
            false,
            BackupPhase::Idle,
        );
        runtime.snapshot.revision = runtime.snapshot.revision.saturating_add(1);
        Ok(())
    }

    pub(crate) fn destination_is_separate_from_mounted_sources(&self, destination: &Path) -> bool {
        let source_roots = self
            .runtime
            .lock()
            .matched
            .values()
            .map(|matched| matched.authority.descriptor.mount_root.clone())
            .collect::<Vec<_>>();
        canonical_destination_is_separate(destination, &source_roots)
    }
}

pub(super) fn upsert_source_snapshot(
    snapshot: &mut AppSnapshotDto,
    source: &SourceRecord,
    rule: &BackupRule,
    mounted: bool,
    phase: BackupPhase,
) {
    if let Some(existing) = snapshot
        .sources
        .iter_mut()
        .find(|existing| existing.source_id == source.id.as_str())
    {
        existing.rule_name = rule.name.clone();
        existing.volume_name = crate::dto::safe_label(&source.display_name);
        existing.legacy_slot = source.legacy_slot.clone();
        existing.mounted = mounted;
        existing.phase = phase;
        return;
    }
    let mut item = SourceSnapshotDto::idle(
        source.id.as_str().to_owned(),
        rule.name.clone(),
        source.display_name.clone(),
        source.legacy_slot.clone(),
    );
    item.mounted = mounted;
    item.phase = phase;
    snapshot.sources.push(item);
}

pub(super) fn mounted_source_phase(should_schedule: bool) -> BackupPhase {
    if should_schedule {
        BackupPhase::Detecting
    } else {
        BackupPhase::Idle
    }
}

pub(super) fn legacy_transmitter(source: &SourceRecord) -> Option<Transmitter> {
    match source.legacy_slot.as_deref() {
        Some("TX01") => Some(Transmitter::Tx01),
        Some("TX02") => Some(Transmitter::Tx02),
        _ => None,
    }
}

pub(super) fn runtime_legacy_transmitter(
    state: &AppState,
    source_id: &SourceId,
) -> Option<Transmitter> {
    state
        .runtime
        .lock()
        .matched
        .get(source_id)
        .and_then(|matched| legacy_transmitter(&matched.authority.source))
}

pub(super) fn safe_volume_label(value: &str) -> String {
    let value = value
        .chars()
        .filter(|character| !character.is_control() && !matches!(character, '/' | '\\'))
        .take(128)
        .collect::<String>();
    if value.trim().is_empty() {
        "External Recorder".to_owned()
    } else {
        value
    }
}

pub(super) fn shared_external_policy(mounted: &MountedVolume) -> bool {
    mounted.descriptor.protocol.eq_ignore_ascii_case("USB")
        && !mounted.descriptor.is_internal
        && mounted.descriptor.is_removable
        && mounted.descriptor.is_writable
}

pub(super) fn rule_constraint_matches(
    mounted: &MountedVolume,
    profile: DeviceConstraintProfile,
) -> bool {
    match profile {
        DeviceConstraintProfile::GenericExternal => true,
        DeviceConstraintProfile::DjiMicMini2s => {
            matches!(
                mounted.descriptor.media_name.to_ascii_lowercase().as_str(),
                "mic tx" | "wireless mic tx media"
            ) && (12_000_000_000..=20_000_000_000).contains(&mounted.descriptor.nominal_capacity)
        }
    }
}

pub(super) fn canonical_destination_is_separate(
    destination: &Path,
    source_roots: &[PathBuf],
) -> bool {
    let Ok(destination) = std::fs::canonicalize(destination) else {
        return false;
    };

    source_roots.iter().all(|source_root| {
        std::fs::canonicalize(source_root).is_ok_and(|source_root| {
            !destination.starts_with(&source_root) && !source_root.starts_with(&destination)
        })
    })
}
