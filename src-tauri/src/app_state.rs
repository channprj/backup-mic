use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use backup_core::{
    audit_log::{AuditDurability, AuditEvent, AuditLevel, AuditSink, AuditValue, FileAuditLog},
    backup::CancellationToken,
    batch::FrozenPreferences,
    deletion::{CompleteRuleDeletionSnapshot, DeletionProposalStore, ProposalInvalidation},
    device::PairedDevice,
    error::{CoreError, PublicError},
    events::{ActivityEntry, ActivitySeverity},
    initial_setup::InitialSetupMarker,
    layout::{
        LayoutMigration, flatten_verified_recording_layout_resilient,
        migrate_legacy_rule_layout_resilient,
    },
    ledger::Ledger,
    preferences::{BackupPreferences, PreferenceKey},
    rule::{
        BackupRule, BackupRuleDraft, DeviceConstraintProfile, FilenameProfile, compile_rule,
        validate_rule,
    },
    rule_scanner::scan_rule_once,
    source::{
        LEGACY_TX01_SOURCE_ID, LEGACY_TX02_SOURCE_ID, MountedSourceAuthority, SourceId,
        SourceRecord,
    },
    state::{BackupPhase, DeletionPhase, Progress, Transmitter},
};
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};

use crate::{
    dto::{
        AppSnapshotDto, ArtifactFormatDto, BackupRuleDto, BackupSettingsDto, NotificationStatusDto,
        ProgressDto, RetirementModeDto, RuleTestResultDto, SetupStateDto, SourceSnapshotDto,
        TransmitterSnapshotDto, destination_display_for,
    },
    failure_reporter::{FailureEvent, FailureReporter, FailureWriteOutcome},
    pairing::{PairingAssignment, PairingManager},
    platform::device_registry::{MountedVolume, VolumeLifecycleEvent},
    rule_runtime::{MatchedSource, RuleVolumeMatch, match_mounted_volume},
};

pub const SNAPSHOT_EVENT: &str = "app-snapshot-changed";

pub(crate) struct RuntimeState {
    pub snapshot: AppSnapshotDto,
    pub pairing: PairingManager,
    pub paired: Vec<PairedDevice>,
    pub observed: HashMap<String, MountedVolume>,
    pub matched: HashMap<SourceId, MatchedSource>,
    pub mounted: HashMap<Transmitter, MountedVolume>,
    pub destination: PathBuf,
    pub destination_configured: bool,
    pub destination_generation: u64,
    pub rule_scan_generations: HashMap<SourceId, u64>,
    pub rule_deletions: HashMap<SourceId, RuleDeletionState>,
    pub awaiting_rule_deletion: Option<SourceId>,
    pub preferences: BackupPreferences,
}

#[derive(Clone)]
pub(crate) struct RuleDeletionState {
    pub authority: MountedSourceAuthority,
    pub rule: BackupRule,
    pub snapshot: CompleteRuleDeletionSnapshot,
    pub destination_generation: u64,
    pub scan_generation: u64,
}

#[derive(Clone)]
pub struct AppState {
    pub(crate) runtime: Arc<Mutex<RuntimeState>>,
    pub(crate) ledger: Arc<Mutex<Ledger>>,
    pub(crate) proposals: Arc<Mutex<DeletionProposalStore>>,
    pub(crate) preference_save: Arc<tokio::sync::Mutex<()>>,
    operation_active: Arc<AtomicBool>,
    operation_reserved: Arc<AtomicBool>,
    cancellation: Arc<Mutex<CancellationToken>>,
    failure_reporter: FailureReporter,
    started: Instant,
}

impl AppState {
    pub fn new(
        ledger: Ledger,
        destination: PathBuf,
        destination_configured: bool,
        autostart_enabled: bool,
    ) -> Result<Self, CoreError> {
        let fallback_root = destination.join("fallback-logs");
        Self::new_with_failure_root(
            ledger,
            destination,
            destination_configured,
            autostart_enabled,
            fallback_root,
        )
    }

    pub fn new_with_failure_root(
        ledger: Ledger,
        destination: PathBuf,
        destination_configured: bool,
        autostart_enabled: bool,
        failure_root: PathBuf,
    ) -> Result<Self, CoreError> {
        let paired = ledger.paired_devices()?;
        let preferences = ledger.read_preferences()?;
        let initial_setup_marker = ledger.initial_setup_marker()?;
        let rules = ledger.backup_rules(true)?;
        let mut sources = Vec::new();
        for rule in &rules {
            for source in ledger.sources_for_rule(&rule.id)? {
                sources.push(SourceSnapshotDto::idle(
                    source.id.as_str().to_owned(),
                    rule.name.clone(),
                    source.display_name,
                    source.legacy_slot,
                ));
            }
        }
        let backup_rules = rules.iter().map(BackupRuleDto::from).collect();
        let setup_state = setup_state_for(destination_configured, initial_setup_marker);
        let home =
            directories::BaseDirs::new().map(|directories| directories.home_dir().to_owned());
        let destination_display =
            destination_display_for(&destination, home.as_deref(), destination_configured);
        Ok(Self {
            runtime: Arc::new(Mutex::new(RuntimeState {
                snapshot: AppSnapshotDto {
                    revision: 1,
                    phase: BackupPhase::Idle,
                    message_code: "idle".to_owned(),
                    overall_progress: ProgressDto::from(&Progress::default()),
                    sources,
                    backup_rules,
                    transmitters: vec![
                        transmitter_snapshot(Transmitter::Tx01),
                        transmitter_snapshot(Transmitter::Tx02),
                    ],
                    current_stage: None,
                    failure_stage: None,
                    setting_applies_next_run: false,
                    current_item_ordinal: None,
                    last_success_at: None,
                    artifact_format: if preferences.m4a_conversion {
                        ArtifactFormatDto::M4a
                    } else {
                        ArtifactFormatDto::Wav
                    },
                    retirement_mode: if preferences.automatic_trash {
                        RetirementModeDto::Automatic
                    } else {
                        RetirementModeDto::Manual
                    },
                    current_log_available: destination_configured,
                    destination_display,
                    settings: BackupSettingsDto {
                        automatic_backup: preferences.automatic_backup,
                        m4a_conversion: preferences.m4a_conversion,
                        automatic_trash: preferences.automatic_trash,
                        autostart: autostart_enabled,
                    },
                    notification_status: NotificationStatusDto::Unknown,
                    setup_state,
                    pairing_candidates: Vec::new(),
                    recent_activity: Vec::new(),
                    error: None,
                },
                pairing: PairingManager::default(),
                paired,
                observed: HashMap::new(),
                matched: HashMap::new(),
                mounted: HashMap::new(),
                destination,
                destination_configured,
                destination_generation: 1,
                rule_scan_generations: HashMap::new(),
                rule_deletions: HashMap::new(),
                awaiting_rule_deletion: None,
                preferences,
            })),
            ledger: Arc::new(Mutex::new(ledger)),
            proposals: Arc::new(Mutex::new(DeletionProposalStore::default())),
            preference_save: Arc::new(tokio::sync::Mutex::new(())),
            operation_active: Arc::new(AtomicBool::new(false)),
            operation_reserved: Arc::new(AtomicBool::new(false)),
            cancellation: Arc::new(Mutex::new(CancellationToken::default())),
            failure_reporter: FailureReporter::new(failure_root),
            started: Instant::now(),
        })
    }

    pub fn snapshot(&self) -> AppSnapshotDto {
        self.runtime.lock().snapshot.clone()
    }

    pub fn snapshot_with_activity(&self) -> Result<AppSnapshotDto, CoreError> {
        let recent_activity = self.ledger.lock().recent_activity(8)?;
        Ok(self.snapshot().with_activity(recent_activity))
    }

    pub fn destination(&self) -> PathBuf {
        self.runtime.lock().destination.clone()
    }

    pub(crate) fn migrate_legacy_layout(
        &self,
        trash: &dyn backup_core::deletion::TrashAdapter,
        cancellation: &CancellationToken,
    ) -> Result<Vec<LayoutMigration>, CoreError> {
        let (destination, configured) = {
            let runtime = self.runtime.lock();
            (runtime.destination.clone(), runtime.destination_configured)
        };
        if !configured || !destination.is_dir() {
            return Ok(Vec::new());
        }

        let mut ledger = self.ledger.lock();
        let mut report_failure = |error: &CoreError| {
            self.report_failure(
                "startup_layout_migration",
                "artifact_relocation",
                error,
                None,
                None,
            );
        };
        let mut migrations = flatten_verified_recording_layout_resilient(
            &destination,
            &mut ledger,
            trash,
            cancellation,
            &mut report_failure,
        )?;
        let dji_rule = ledger.dji_rule()?;
        migrations.extend(migrate_legacy_rule_layout_resilient(
            &destination,
            &mut ledger,
            &dji_rule,
            trash,
            cancellation,
            &mut report_failure,
        )?);
        let backup_rules = ledger
            .backup_rules(true)?
            .iter()
            .map(BackupRuleDto::from)
            .collect();
        drop(ledger);
        let mut runtime = self.runtime.lock();
        runtime.snapshot.backup_rules = backup_rules;
        runtime.snapshot.revision = runtime.snapshot.revision.saturating_add(1);
        Ok(migrations)
    }

    pub fn append_audit(
        &self,
        event: &AuditEvent<'_>,
        durability: AuditDurability,
    ) -> Result<PathBuf, CoreError> {
        let destination = {
            let runtime = self.runtime.lock();
            if !runtime.destination_configured {
                return Err(CoreError::InvalidRequest);
            }
            runtime.destination.clone()
        };
        FileAuditLog::new(destination).append(event, durability)
    }

    pub fn report_failure(
        &self,
        operation: &'static str,
        stage: &'static str,
        error: &CoreError,
        transmitter: Option<Transmitter>,
        item_name: Option<&str>,
    ) -> FailureWriteOutcome {
        let primary_destination = {
            let runtime = self.runtime.lock();
            runtime
                .destination_configured
                .then(|| runtime.destination.clone())
        };
        let public = error.public(transmitter);
        let event = FailureEvent {
            operation,
            stage,
            transmitter,
            item_name: sanitize_item_name(item_name),
            error_code: error.diagnostic_code().to_owned(),
            os_kind: error.diagnostic_io_kind_code().map(str::to_owned),
            retryable: public.retryable,
        };
        self.failure_reporter
            .report(primary_destination.as_deref(), local_now(), &event)
    }

    pub fn report_public_failure(
        &self,
        operation: &'static str,
        stage: &'static str,
        error: &PublicError,
        item_name: Option<&str>,
    ) -> FailureWriteOutcome {
        let primary_destination = {
            let runtime = self.runtime.lock();
            runtime
                .destination_configured
                .then(|| runtime.destination.clone())
        };
        let event = FailureEvent {
            operation,
            stage,
            transmitter: error.transmitter,
            item_name: sanitize_item_name(item_name),
            error_code: error.message_code.clone(),
            os_kind: None,
            retryable: error.retryable,
        };
        self.failure_reporter
            .report(primary_destination.as_deref(), local_now(), &event)
    }

    pub fn backup_is_ready(&self) -> bool {
        let (destination_configured, setup_state, mounted_count, destination, source_roots) = {
            let runtime = self.runtime.lock();
            (
                runtime.destination_configured,
                runtime.snapshot.setup_state,
                runtime.matched.len(),
                runtime.destination.clone(),
                runtime
                    .matched
                    .values()
                    .map(|matched| matched.authority.descriptor.mount_root.clone())
                    .collect::<Vec<_>>(),
            )
        };

        backup_requirements_met(destination_configured, setup_state, mounted_count)
            && canonical_destination_is_separate(&destination, &source_roots)
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

    pub fn automatic_backup_enabled(&self) -> bool {
        self.runtime.lock().preferences.automatic_backup
    }

    pub fn m4a_conversion_enabled(&self) -> bool {
        self.runtime.lock().preferences.m4a_conversion
    }

    pub fn automatic_trash_enabled(&self) -> bool {
        self.runtime.lock().preferences.automatic_trash
    }

    pub fn frozen_preferences(&self) -> FrozenPreferences {
        let preferences = self.runtime.lock().preferences;
        FrozenPreferences {
            automatic_backup: preferences.automatic_backup,
            m4a_conversion: preferences.m4a_conversion,
            automatic_trash: preferences.automatic_trash,
        }
    }

    pub fn set_preference(
        &self,
        key: PreferenceKey,
        enabled: bool,
        occurred_at: &str,
    ) -> Result<AppSnapshotDto, CoreError> {
        self.ledger
            .lock()
            .set_preference(key, enabled, occurred_at)?;

        let mut runtime = self.runtime.lock();
        match key {
            PreferenceKey::AutomaticBackup => {
                runtime.preferences.automatic_backup = enabled;
                runtime.snapshot.settings.automatic_backup = enabled;
            }
            PreferenceKey::M4aConversion => {
                runtime.preferences.m4a_conversion = enabled;
                runtime.snapshot.settings.m4a_conversion = enabled;
                runtime.snapshot.artifact_format = if enabled {
                    ArtifactFormatDto::M4a
                } else {
                    ArtifactFormatDto::Wav
                };
                if !enabled {
                    clear_retirement_authority(&mut runtime);
                }
            }
            PreferenceKey::AutomaticTrash => {
                runtime.preferences.automatic_trash = enabled;
                runtime.snapshot.settings.automatic_trash = enabled;
                runtime.snapshot.retirement_mode = if enabled {
                    RetirementModeDto::Automatic
                } else {
                    RetirementModeDto::Manual
                };
            }
        }
        runtime.snapshot.revision = runtime.snapshot.revision.saturating_add(1);
        Ok(runtime.snapshot.clone())
    }

    pub fn apply_persisted_preferences(&self, preferences: BackupPreferences) -> AppSnapshotDto {
        let mut runtime = self.runtime.lock();
        runtime.preferences = preferences;
        runtime.snapshot.settings.automatic_backup = preferences.automatic_backup;
        runtime.snapshot.settings.m4a_conversion = preferences.m4a_conversion;
        runtime.snapshot.settings.automatic_trash = preferences.automatic_trash;
        runtime.snapshot.artifact_format = if preferences.m4a_conversion {
            ArtifactFormatDto::M4a
        } else {
            ArtifactFormatDto::Wav
        };
        runtime.snapshot.retirement_mode = if preferences.automatic_trash {
            RetirementModeDto::Automatic
        } else {
            RetirementModeDto::Manual
        };
        if !preferences.m4a_conversion {
            clear_retirement_authority(&mut runtime);
        }
        runtime.snapshot.setting_applies_next_run = self.operation_is_active();
        runtime.snapshot.revision = runtime.snapshot.revision.saturating_add(1);
        runtime.snapshot.clone()
    }

    pub fn log_directory(&self, occurred_at: time::OffsetDateTime) -> Result<PathBuf, CoreError> {
        let runtime = self.runtime.lock();
        if !runtime.destination_configured {
            return Err(CoreError::InvalidRequest);
        }
        FileAuditLog::new(&runtime.destination)
            .path_for(occurred_at)
            .parent()
            .map(PathBuf::from)
            .ok_or(CoreError::InvalidRequest)
    }

    pub fn operation_is_active(&self) -> bool {
        self.operation_active.load(Ordering::SeqCst)
    }

    pub fn mounted_roots(&self) -> HashMap<Transmitter, PathBuf> {
        self.runtime
            .lock()
            .mounted
            .iter()
            .map(|(transmitter, mounted)| (*transmitter, mounted.descriptor.mount_root.clone()))
            .collect()
    }

    pub fn matched_sources(&self) -> HashMap<SourceId, MatchedSource> {
        self.runtime.lock().matched.clone()
    }

    pub(crate) fn backup_destination_snapshot(&self) -> (PathBuf, u64) {
        let runtime = self.runtime.lock();
        (runtime.destination.clone(), runtime.destination_generation)
    }

    pub(crate) fn destination_snapshot_is_current(
        &self,
        destination: &Path,
        generation: u64,
    ) -> bool {
        let runtime = self.runtime.lock();
        runtime.destination_generation == generation && runtime.destination == destination
    }

    pub(crate) fn source_authority_is_current(&self, authority: &MountedSourceAuthority) -> bool {
        self.runtime
            .lock()
            .matched
            .get(&authority.source.id)
            .is_some_and(|matched| matched.authority == *authority)
    }

    pub fn save_backup_rule_for_state(
        &self,
        draft: BackupRuleDraft,
        updated_at: &str,
    ) -> Result<BackupRule, CoreError> {
        let rule = self.ledger.lock().save_backup_rule(draft, updated_at)?;
        self.proposals
            .lock()
            .invalidate(ProposalInvalidation::NewScanResults);
        let (rules, source_rule_names) = public_rule_catalog(&self.ledger.lock())?;
        let mut runtime = self.runtime.lock();
        runtime
            .rule_deletions
            .retain(|_, evidence| evidence.rule.id != rule.id);
        runtime.awaiting_rule_deletion = None;
        refresh_rule_catalog(&mut runtime.snapshot, rules, &source_rule_names);
        runtime.snapshot.setting_applies_next_run = self.operation_is_active();
        runtime.snapshot.revision = runtime.snapshot.revision.saturating_add(1);
        Ok(rule)
    }

    pub fn archive_backup_rule_for_state(
        &self,
        rule_id: &str,
        archived_at: &str,
    ) -> Result<(), CoreError> {
        let rule_id = backup_core::rule::RuleId::parse(rule_id)?;
        {
            let mut ledger = self.ledger.lock();
            ledger.archive_backup_rule(&rule_id, archived_at)?;
        }
        self.proposals
            .lock()
            .invalidate(ProposalInvalidation::NewScanResults);
        let (rules, source_rule_names) = public_rule_catalog(&self.ledger.lock())?;
        let mut runtime = self.runtime.lock();
        runtime
            .rule_deletions
            .retain(|_, evidence| evidence.rule.id != rule_id);
        runtime.awaiting_rule_deletion = None;
        refresh_rule_catalog(&mut runtime.snapshot, rules, &source_rule_names);
        runtime.snapshot.setting_applies_next_run = self.operation_is_active();
        runtime.snapshot.revision = runtime.snapshot.revision.saturating_add(1);
        Ok(())
    }

    pub fn restore_dji_rule_for_state(&self, updated_at: &str) -> Result<BackupRule, CoreError> {
        let rule = self.ledger.lock().restore_dji_preset(updated_at)?;
        self.proposals
            .lock()
            .invalidate(ProposalInvalidation::NewScanResults);
        let (rules, source_rule_names) = public_rule_catalog(&self.ledger.lock())?;
        let mut runtime = self.runtime.lock();
        runtime
            .rule_deletions
            .retain(|_, evidence| evidence.rule.id != rule.id);
        runtime.awaiting_rule_deletion = None;
        refresh_rule_catalog(&mut runtime.snapshot, rules, &source_rule_names);
        runtime.snapshot.setting_applies_next_run = self.operation_is_active();
        runtime.snapshot.revision = runtime.snapshot.revision.saturating_add(1);
        Ok(rule)
    }

    pub fn test_backup_rule_for_state(
        &self,
        draft: BackupRuleDraft,
    ) -> Result<RuleTestResultDto, CoreError> {
        let draft = validate_rule(draft)?;
        let test_rule = BackupRule {
            id: draft.id.clone().unwrap_or_default(),
            name: draft.name,
            archive_directory_name: draft.archive_directory_name,
            enabled: draft.enabled,
            volume_name_glob: draft.volume_name_glob,
            required_path_globs: draft.required_path_globs,
            backup_file_globs: draft.backup_file_globs,
            session_directory_globs: draft.session_directory_globs,
            filename_prefix: draft.filename_prefix,
            filename_suffix: draft.filename_suffix,
            filename_profile: FilenameProfile::Preserve,
            device_constraint_profile: DeviceConstraintProfile::GenericExternal,
            preset_kind: None,
            preset_revision: None,
            archive_directory_locked: false,
            archived_at: None,
            created_at: "rule-test".to_owned(),
            updated_at: "rule-test".to_owned(),
        };
        let compiled_test = compile_rule(test_rule.clone())?;
        let existing_rules = self.ledger.lock().backup_rules(false)?;
        let observed = self
            .runtime
            .lock()
            .observed
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let local_offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
        let mut matched_volumes = Vec::new();
        let mut matched_file_count = 0_u64;
        let mut conflicts = BTreeSet::new();
        for mounted in observed {
            let display_name = safe_volume_label(&mounted.display_name);
            if !shared_external_policy(&mounted)
                || !compiled_test.matches_volume_name(&display_name)
            {
                continue;
            }
            let scan =
                scan_rule_once(&mounted.descriptor.mount_root, &compiled_test, local_offset)?;
            if scan.files.is_empty() {
                continue;
            }
            matched_volumes.push(display_name.clone());
            matched_file_count = matched_file_count
                .checked_add(u64::try_from(scan.files.len()).map_err(|_| CoreError::InvalidRule)?)
                .ok_or(CoreError::InvalidRule)?;
            for rule in existing_rules.iter().filter(|rule| {
                rule.enabled && rule.archived_at.is_none() && rule.id != test_rule.id
            }) {
                let compiled = compile_rule(rule.clone())?;
                if compiled.matches_volume_name(&display_name)
                    && rule_constraint_matches(&mounted, rule.device_constraint_profile)
                    && !scan_rule_once(&mounted.descriptor.mount_root, &compiled, local_offset)?
                        .files
                        .is_empty()
                {
                    conflicts.insert(rule.name.clone());
                }
            }
        }
        matched_volumes.sort();
        matched_volumes.dedup();
        Ok(RuleTestResultDto {
            matched_volumes,
            matched_file_count,
            conflict_rule_names: conflicts.into_iter().collect(),
        })
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

    pub fn install_matched_source_for_state(&self, matched: MatchedSource) {
        self.runtime
            .lock()
            .matched
            .insert(matched.authority.source.id.clone(), matched);
    }

    pub fn cancel_active_operation(&self) {
        self.cancellation.lock().cancel();
    }

    pub fn elapsed(&self) -> std::time::Duration {
        self.started.elapsed()
    }

    pub fn begin_operation(&self) -> Result<OperationGuard, CoreError> {
        if self.operation_reserved.load(Ordering::SeqCst) {
            return Err(CoreError::Busy);
        }
        self.operation_active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| CoreError::Busy)?;
        let cancellation = CancellationToken::default();
        *self.cancellation.lock() = cancellation.clone();
        Ok(OperationGuard {
            active: Arc::clone(&self.operation_active),
            cancellation,
            reservation: None,
        })
    }

    pub(crate) fn reserve_operation(&self) -> Result<OperationReservation, CoreError> {
        self.operation_reserved
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| CoreError::Busy)?;
        Ok(OperationReservation {
            active: Arc::clone(&self.operation_active),
            reserved: Some(Arc::clone(&self.operation_reserved)),
            cancellation: Arc::clone(&self.cancellation),
        })
    }

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
                runtime.pairing.remove(&volume_uuid, mount_generation);
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
                if let Some((source_id, matched)) = &removed_source {
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
                    if let Some(transmitter) = legacy_transmitter(&matched.authority.source) {
                        runtime.mounted.remove(&transmitter);
                        update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
                            snapshot.mounted = false;
                            snapshot.phase = BackupPhase::Idle;
                            snapshot.deletion_ready = false;
                            snapshot.deletion_phase = DeletionPhase::Inactive;
                        });
                    }
                }
                sync_pairing_snapshot(&mut runtime);
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
                                occurred_at: local_now(),
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
                time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC),
                &crate::orchestrator::now_string(),
            )
        };
        let mut runtime = self.runtime.lock();
        let mut rejected_error = None;
        let matched_source = match result {
            Ok(RuleVolumeMatch::Matched(matched)) => {
                let matched = *matched;
                let source_id = matched.authority.source.id.clone();
                upsert_source_snapshot(
                    &mut runtime.snapshot,
                    &matched.authority.source,
                    &matched.rule,
                    true,
                    BackupPhase::Detecting,
                );
                if let Some(transmitter) = legacy_transmitter(&matched.authority.source) {
                    runtime.mounted.insert(transmitter, mounted);
                    update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
                        snapshot.mounted = true;
                        snapshot.phase = BackupPhase::Detecting;
                        snapshot.deletion_ready = false;
                    });
                }
                runtime.matched.insert(source_id.clone(), matched);
                runtime.snapshot.phase = BackupPhase::Detecting;
                runtime.snapshot.message_code = "device_detected".to_owned();
                Some(source_id)
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
        sync_pairing_snapshot(&mut runtime);
        let should_schedule = matched_source.clone().filter(|_| {
            runtime.preferences.automatic_backup
                && backup_requirements_met(
                    runtime.destination_configured,
                    runtime.snapshot.setup_state,
                    runtime.matched.len(),
                )
        });
        publish_locked(app, &mut runtime);
        drop(runtime);
        if let Some(error) = rejected_error {
            self.report_public_failure("device_lifecycle", "identity_validation", &error, None);
        }
        if let Some(source_id) = matched_source {
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
                        occurred_at: local_now(),
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

    pub fn pair_devices(
        &self,
        app: &AppHandle,
        assignments: &[PairingAssignment],
        paired_at: &str,
    ) -> Result<Vec<Transmitter>, CoreError> {
        let mut runtime = self.runtime.lock();
        let paired = {
            let mut ledger = self.ledger.lock();
            runtime.pairing.pair(assignments, &mut ledger, paired_at)?;
            ledger.paired_devices()?
        };
        let mut mounted_transmitters = Vec::new();
        for device in &paired {
            let dji_rule = self.ledger.lock().dji_rule()?;
            let source = SourceRecord {
                id: legacy_source_id(device.transmitter),
                rule_id: dji_rule.id,
                volume_uuid: device.expected_uuid.clone(),
                legacy_slot: Some(transmitter_name(device.transmitter).to_owned()),
                display_name: format!("DJI Mic Mini 2S ({})", transmitter_name(device.transmitter)),
            };
            self.ledger.lock().upsert_source(&source, paired_at)?;
            if let Some(mounted) = runtime
                .observed
                .values()
                .find(|mounted| {
                    mounted
                        .descriptor
                        .volume_uuid
                        .eq_ignore_ascii_case(&device.expected_uuid)
                })
                .cloned()
            {
                runtime.mounted.insert(device.transmitter, mounted);
                mounted_transmitters.push(device.transmitter);
                update_transmitter(&mut runtime.snapshot, device.transmitter, |snapshot| {
                    snapshot.mounted = true;
                    snapshot.phase = BackupPhase::Detecting;
                });
            }
        }
        runtime.paired = paired;
        sync_pairing_snapshot(&mut runtime);
        publish_locked(app, &mut runtime);
        Ok(mounted_transmitters)
    }

    pub fn persist_destination_for_state(
        &self,
        app: &AppHandle,
        destination: PathBuf,
        occurred_at: &str,
    ) -> Result<AppSnapshotDto, CoreError> {
        let current_setup = self.runtime.lock().snapshot.setup_state;
        let require_settings_review = current_setup == SetupStateDto::NeedsDestination;
        let encoded = serde_json::to_string(&destination.to_string_lossy())
            .map_err(|_| CoreError::InvalidRequest)?;
        self.ledger
            .lock()
            .persist_destination(&encoded, require_settings_review, occurred_at)?;

        self.proposals
            .lock()
            .invalidate(ProposalInvalidation::DestinationChanged);
        let mut runtime = self.runtime.lock();
        let home =
            directories::BaseDirs::new().map(|directories| directories.home_dir().to_owned());
        runtime.snapshot.destination_display =
            destination_display_for(&destination, home.as_deref(), true);
        runtime.destination = destination;
        runtime.destination_configured = true;
        runtime.snapshot.current_log_available = true;
        runtime.destination_generation = runtime.destination_generation.saturating_add(1);
        runtime.rule_deletions.clear();
        runtime.rule_scan_generations.clear();
        runtime.awaiting_rule_deletion = None;
        for snapshot in &mut runtime.snapshot.transmitters {
            snapshot.deletion_ready = false;
            snapshot.deletion_phase = DeletionPhase::Inactive;
        }
        for source in &mut runtime.snapshot.sources {
            source.deletion_ready = false;
            source.retirement_outcome = DeletionPhase::Inactive;
        }
        runtime.snapshot.setup_state = match current_setup {
            SetupStateDto::NeedsDestination => SetupStateDto::NeedsSettingsReview,
            SetupStateDto::NeedsSettingsReview => SetupStateDto::NeedsSettingsReview,
            SetupStateDto::Ready => SetupStateDto::Ready,
        };
        publish_locked(app, &mut runtime);
        Ok(runtime.snapshot.clone())
    }

    pub fn complete_initial_setup_for_state(
        &self,
        app: &AppHandle,
        occurred_at: &str,
    ) -> Result<AppSnapshotDto, CoreError> {
        let (setup_state, destination) = {
            let runtime = self.runtime.lock();
            (runtime.snapshot.setup_state, runtime.destination.clone())
        };
        if setup_state != SetupStateDto::NeedsSettingsReview || !destination.is_dir() {
            return Err(CoreError::InvalidRequest);
        }
        self.ledger.lock().complete_initial_setup(occurred_at)?;
        let mut runtime = self.runtime.lock();
        runtime.snapshot.setup_state = SetupStateDto::Ready;
        publish_locked(app, &mut runtime);
        Ok(runtime.snapshot.clone())
    }

    pub fn record_activity(&self, app: &AppHandle, entry: ActivityEntry) -> Result<(), CoreError> {
        let recent_activity = {
            let mut ledger = self.ledger.lock();
            ledger.append_activity(&entry)?;
            ledger.recent_activity(8)?
        };
        let mut runtime = self.runtime.lock();
        runtime.snapshot.recent_activity = runtime
            .snapshot
            .clone()
            .with_activity(recent_activity)
            .recent_activity;
        publish_locked(app, &mut runtime);
        Ok(())
    }

    pub fn set_error(&self, app: &AppHandle, error: CoreError, transmitter: Option<Transmitter>) {
        let public = error.public(transmitter);
        let mut runtime = self.runtime.lock();
        runtime.snapshot.phase = BackupPhase::Error;
        runtime.snapshot.message_code = public.message_code.clone();
        runtime.snapshot.error = Some(public);
        runtime.snapshot.failure_stage = runtime.snapshot.current_stage;
        runtime.snapshot.current_stage = None;
        runtime.snapshot.setting_applies_next_run = false;
        if let Some(transmitter) = transmitter {
            update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
                snapshot.phase = BackupPhase::Error;
                snapshot.deletion_ready = false;
            });
        }
        publish_locked(app, &mut runtime);
    }

    pub fn set_deletion_error(&self, app: &AppHandle, error: &CoreError, transmitter: Transmitter) {
        let public = error.public(Some(transmitter));
        let mut runtime = self.runtime.lock();
        runtime.snapshot.phase = BackupPhase::Error;
        runtime.snapshot.message_code = public.message_code.clone();
        runtime.snapshot.error = Some(public);
        runtime.snapshot.failure_stage = runtime.snapshot.current_stage;
        runtime.snapshot.current_stage = None;
        runtime.snapshot.setting_applies_next_run = false;
        update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
            snapshot.deletion_phase = DeletionPhase::Refused;
            snapshot.deletion_ready = false;
        });
        publish_locked(app, &mut runtime);
    }

    pub fn set_source_deletion_error(
        &self,
        app: &AppHandle,
        error: &CoreError,
        source_id: &SourceId,
    ) {
        let public = self.public_error_for_source(error.public(None), source_id);
        let mut runtime = self.runtime.lock();
        runtime.snapshot.phase = BackupPhase::Error;
        runtime.snapshot.message_code = public.message_code.clone();
        runtime.snapshot.error = Some(public);
        runtime.snapshot.failure_stage = runtime.snapshot.current_stage;
        runtime.snapshot.current_stage = None;
        runtime.snapshot.setting_applies_next_run = false;
        update_source(&mut runtime.snapshot, source_id, |snapshot| {
            snapshot.phase = BackupPhase::Error;
            snapshot.retirement_outcome = DeletionPhase::Refused;
            snapshot.deletion_ready = false;
        });
        publish_locked(app, &mut runtime);
    }

    pub fn public_error_for_source(&self, error: PublicError, source_id: &SourceId) -> PublicError {
        let label = self
            .runtime
            .lock()
            .snapshot
            .sources
            .iter()
            .find(|source| source.source_id == source_id.as_str())
            .map(|source| source.volume_name.clone())
            .unwrap_or_else(|| "External Recorder".to_owned());
        error.with_source(source_id.clone(), label)
    }

    pub fn awaiting_deletion_source(&self) -> Option<SourceId> {
        self.runtime.lock().awaiting_rule_deletion.clone()
    }

    pub fn awaiting_deletion_transmitter(&self) -> Option<Transmitter> {
        self.runtime
            .lock()
            .snapshot
            .transmitters
            .iter()
            .find(|snapshot| snapshot.deletion_phase == DeletionPhase::AwaitingConfirmation)
            .map(|snapshot| snapshot.transmitter)
    }

    pub fn set_autostart(&self, app: &AppHandle, enabled: bool) {
        let mut runtime = self.runtime.lock();
        runtime.snapshot.settings.autostart = enabled;
        publish_locked(app, &mut runtime);
    }

    pub fn set_notification_status(&self, app: &AppHandle, status: NotificationStatusDto) {
        let mut runtime = self.runtime.lock();
        runtime.snapshot.notification_status = status;
        publish_locked(app, &mut runtime);
    }

    pub fn should_keep_window_open(&self) -> bool {
        let runtime = self.runtime.lock();
        runtime.snapshot.setup_state != SetupStateDto::Ready
            || runtime.snapshot.sources.iter().any(|snapshot| {
                matches!(
                    snapshot.retirement_outcome,
                    DeletionPhase::Preparing
                        | DeletionPhase::AwaitingConfirmation
                        | DeletionPhase::Revalidating
                        | DeletionPhase::Deleting
                )
            })
    }

    pub fn public_error(error: CoreError, transmitter: Option<Transmitter>) -> PublicError {
        error.public(transmitter)
    }
}

pub struct OperationGuard {
    active: Arc<AtomicBool>,
    pub cancellation: CancellationToken,
    reservation: Option<Arc<AtomicBool>>,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.active.store(false, Ordering::SeqCst);
        if let Some(reservation) = self.reservation.take() {
            reservation.store(false, Ordering::SeqCst);
        }
    }
}

pub(crate) struct OperationReservation {
    active: Arc<AtomicBool>,
    reserved: Option<Arc<AtomicBool>>,
    cancellation: Arc<Mutex<CancellationToken>>,
}

impl OperationReservation {
    pub(crate) async fn acquire(mut self) -> OperationGuard {
        loop {
            if self
                .active
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                let cancellation = CancellationToken::default();
                *self.cancellation.lock() = cancellation.clone();
                return OperationGuard {
                    active: Arc::clone(&self.active),
                    cancellation,
                    reservation: self.reserved.take(),
                };
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}

impl Drop for OperationReservation {
    fn drop(&mut self) {
        if let Some(reservation) = self.reserved.take() {
            reservation.store(false, Ordering::SeqCst);
        }
    }
}

pub(crate) fn publish_locked(app: &AppHandle, runtime: &mut RuntimeState) {
    runtime.snapshot.revision = runtime.snapshot.revision.saturating_add(1);
    let _ = app.emit(SNAPSHOT_EVENT, runtime.snapshot.clone());
}

pub(crate) fn update_transmitter(
    snapshot: &mut AppSnapshotDto,
    transmitter: Transmitter,
    update: impl FnOnce(&mut TransmitterSnapshotDto),
) {
    if let Some(transmitter_snapshot) = snapshot
        .transmitters
        .iter_mut()
        .find(|snapshot| snapshot.transmitter == transmitter)
    {
        update(transmitter_snapshot);
    }
}

pub(crate) fn update_source(
    snapshot: &mut AppSnapshotDto,
    source_id: &SourceId,
    update: impl FnOnce(&mut SourceSnapshotDto),
) {
    if let Some(source) = snapshot
        .sources
        .iter_mut()
        .find(|source| source.source_id == source_id.as_str())
    {
        update(source);
    }
}

fn upsert_source_snapshot(
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

fn public_rule_catalog(
    ledger: &Ledger,
) -> Result<(Vec<BackupRule>, HashMap<String, String>), CoreError> {
    let rules = ledger.backup_rules(true)?;
    let mut source_rule_names = HashMap::new();
    for rule in &rules {
        for source in ledger.sources_for_rule(&rule.id)? {
            source_rule_names.insert(source.id.as_str().to_owned(), rule.name.clone());
        }
    }
    Ok((rules, source_rule_names))
}

fn refresh_rule_catalog(
    snapshot: &mut AppSnapshotDto,
    rules: Vec<BackupRule>,
    source_rule_names: &HashMap<String, String>,
) {
    snapshot.backup_rules = rules.iter().map(BackupRuleDto::from).collect();
    for source in &mut snapshot.sources {
        if let Some(rule_name) = source_rule_names.get(&source.source_id) {
            source.rule_name.clone_from(rule_name);
        }
    }
}

fn sync_pairing_snapshot(runtime: &mut RuntimeState) {
    runtime.snapshot.pairing_candidates = runtime.pairing.summaries();
}

fn setup_state_for(
    destination_configured: bool,
    marker: Option<InitialSetupMarker>,
) -> SetupStateDto {
    if !destination_configured {
        return SetupStateDto::NeedsDestination;
    }
    match marker {
        Some(InitialSetupMarker::SettingsReviewPending) => SetupStateDto::NeedsSettingsReview,
        None | Some(InitialSetupMarker::Complete) => SetupStateDto::Ready,
    }
}

fn backup_requirements_met(
    destination_configured: bool,
    setup_state: SetupStateDto,
    mounted_devices: usize,
) -> bool {
    destination_configured && setup_state == SetupStateDto::Ready && mounted_devices > 0
}

fn transmitter_snapshot(transmitter: Transmitter) -> TransmitterSnapshotDto {
    TransmitterSnapshotDto {
        transmitter,
        mounted: false,
        phase: BackupPhase::Idle,
        progress: ProgressDto::from(&Progress::default()),
        deletion_phase: DeletionPhase::Inactive,
        deletion_ready: false,
    }
}

fn activity_entry(code: &str, source_id: SourceId, severity: ActivitySeverity) -> ActivityEntry {
    ActivityEntry {
        occurred_at: time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned()),
        code: code.to_owned(),
        source_id: Some(source_id),
        source_label: None,
        count_value: None,
        byte_value: None,
        severity,
    }
}

fn legacy_transmitter(source: &SourceRecord) -> Option<Transmitter> {
    match source.legacy_slot.as_deref() {
        Some("TX01") => Some(Transmitter::Tx01),
        Some("TX02") => Some(Transmitter::Tx02),
        _ => None,
    }
}

fn runtime_legacy_transmitter(state: &AppState, source_id: &SourceId) -> Option<Transmitter> {
    state
        .runtime
        .lock()
        .matched
        .get(source_id)
        .and_then(|matched| legacy_transmitter(&matched.authority.source))
}

fn legacy_source_id(transmitter: Transmitter) -> SourceId {
    SourceId::parse(match transmitter {
        Transmitter::Tx01 => LEGACY_TX01_SOURCE_ID,
        Transmitter::Tx02 => LEGACY_TX02_SOURCE_ID,
    })
    .expect("legacy source IDs are canonical UUID literals")
}

fn transmitter_name(transmitter: Transmitter) -> &'static str {
    match transmitter {
        Transmitter::Tx01 => "TX01",
        Transmitter::Tx02 => "TX02",
    }
}

fn local_now() -> time::OffsetDateTime {
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    time::OffsetDateTime::now_utc().to_offset(offset)
}

fn sanitize_item_name(item_name: Option<&str>) -> Option<String> {
    item_name
        .and_then(|item_name| std::path::Path::new(item_name).file_name())
        .and_then(|item_name| item_name.to_str())
        .map(|item_name| item_name.chars().take(180).collect())
        .filter(|item_name: &String| !item_name.is_empty())
}

fn safe_volume_label(value: &str) -> String {
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

fn shared_external_policy(mounted: &MountedVolume) -> bool {
    mounted.descriptor.protocol.eq_ignore_ascii_case("USB")
        && !mounted.descriptor.is_internal
        && mounted.descriptor.is_removable
        && mounted.descriptor.is_writable
}

fn rule_constraint_matches(mounted: &MountedVolume, profile: DeviceConstraintProfile) -> bool {
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

fn clear_retirement_authority(runtime: &mut RuntimeState) {
    runtime.rule_deletions.clear();
    runtime.rule_scan_generations.clear();
    runtime.awaiting_rule_deletion = None;
    for transmitter in &mut runtime.snapshot.transmitters {
        transmitter.deletion_ready = false;
        transmitter.deletion_phase = DeletionPhase::Inactive;
    }
    for source in &mut runtime.snapshot.sources {
        source.deletion_ready = false;
        source.retirement_outcome = DeletionPhase::Inactive;
    }
}

fn canonical_destination_is_separate(destination: &Path, source_roots: &[PathBuf]) -> bool {
    let Ok(destination) = std::fs::canonicalize(destination) else {
        return false;
    };

    source_roots.iter().all(|source_root| {
        std::fs::canonicalize(source_root).is_ok_and(|source_root| {
            !destination.starts_with(&source_root) && !source_root.starts_with(&destination)
        })
    })
}

#[cfg(test)]
mod tests {
    use backup_core::audit_log::{AuditDurability, AuditEvent, AuditLevel, AuditValue};
    use backup_core::initial_setup::InitialSetupMarker;
    use backup_core::preferences::PreferenceKey;
    use tempfile::tempdir;
    use time::macros::datetime;

    use super::*;

    #[test]
    fn first_run_requires_a_user_confirmed_destination() {
        let state_directory = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let ledger = Ledger::open(state_directory.path().join("ledger.sqlite3")).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), false, false).unwrap();

        assert_eq!(
            state.snapshot().setup_state,
            SetupStateDto::NeedsDestination
        );
        assert!(!state.backup_is_ready());
    }

    #[test]
    fn setup_state_preserves_existing_destinations_and_resumes_pending_review() {
        let state_directory = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let legacy_ledger = Ledger::open(state_directory.path().join("legacy.sqlite3")).unwrap();
        let legacy =
            AppState::new(legacy_ledger, destination.path().to_path_buf(), true, false).unwrap();
        assert_eq!(legacy.snapshot().setup_state, SetupStateDto::Ready);

        let pending_path = state_directory.path().join("pending.sqlite3");
        let mut pending_ledger = Ledger::open(&pending_path).unwrap();
        pending_ledger
            .persist_destination(r#""/tmp/Pending Backup""#, true, "2026-08-11T00:00:00Z")
            .unwrap();
        assert_eq!(
            pending_ledger.initial_setup_marker().unwrap(),
            Some(InitialSetupMarker::SettingsReviewPending)
        );
        let pending = AppState::new(
            pending_ledger,
            destination.path().to_path_buf(),
            true,
            false,
        )
        .unwrap();
        assert_eq!(
            pending.snapshot().setup_state,
            SetupStateDto::NeedsSettingsReview
        );

        let mut completed_ledger = Ledger::open(&pending_path).unwrap();
        completed_ledger
            .complete_initial_setup("2026-08-11T00:01:00Z")
            .unwrap();
        let completed = AppState::new(
            completed_ledger,
            destination.path().to_path_buf(),
            true,
            false,
        )
        .unwrap();
        assert_eq!(completed.snapshot().setup_state, SetupStateDto::Ready);
    }

    #[test]
    fn automatic_backup_waits_for_every_setup_requirement() {
        assert!(!backup_requirements_met(
            false,
            SetupStateDto::NeedsDestination,
            2
        ));
        assert!(!backup_requirements_met(
            true,
            SetupStateDto::NeedsSettingsReview,
            2
        ));
        assert!(!backup_requirements_met(true, SetupStateDto::Ready, 0));
        assert!(backup_requirements_met(true, SetupStateDto::Ready, 1));
        assert!(backup_requirements_met(true, SetupStateDto::Ready, 2));
    }

    #[test]
    fn destination_must_stay_outside_every_mounted_source() {
        let fixture = tempdir().unwrap();
        let source = fixture.path().join("DJI-MIC-1");
        let nested_destination = source.join("backups");
        let separate_destination = fixture.path().join("990EVO-backups");
        std::fs::create_dir_all(&nested_destination).unwrap();
        std::fs::create_dir_all(&separate_destination).unwrap();

        assert!(!canonical_destination_is_separate(
            &nested_destination,
            std::slice::from_ref(&source)
        ));
        assert!(!canonical_destination_is_separate(
            fixture.path(),
            std::slice::from_ref(&source)
        ));
        assert!(canonical_destination_is_separate(
            &separate_destination,
            &[source]
        ));
    }

    #[test]
    fn snapshot_reads_always_return_the_latest_revision() {
        let state_directory = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let ledger = Ledger::open(state_directory.path().join("ledger.sqlite3")).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();
        let first = state.snapshot();
        {
            let mut runtime = state.runtime.lock();
            runtime.snapshot.revision += 1;
            runtime.snapshot.message_code = "newer".to_owned();
        }
        let repaired = state.snapshot();
        assert!(repaired.revision > first.revision);
        assert_eq!(repaired.message_code, "newer");
    }

    #[test]
    fn only_one_backup_or_deletion_operation_can_be_active() {
        let state_directory = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let ledger = Ledger::open(state_directory.path().join("ledger.sqlite3")).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();
        let first = state.begin_operation().unwrap();
        assert!(matches!(state.begin_operation(), Err(CoreError::Busy)));
        drop(first);
        assert!(state.begin_operation().is_ok());
    }

    #[tokio::test]
    async fn reserved_operation_waits_for_the_active_run_and_blocks_new_starters() {
        let state_directory = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let ledger = Ledger::open(state_directory.path().join("ledger.sqlite3")).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();

        let active = state.begin_operation().unwrap();
        let reservation = state.reserve_operation().unwrap();

        assert!(matches!(state.begin_operation(), Err(CoreError::Busy)));

        drop(active);
        let reserved =
            tokio::time::timeout(std::time::Duration::from_secs(1), reservation.acquire())
                .await
                .expect("the reserved operation should acquire the next slot");

        assert!(matches!(state.begin_operation(), Err(CoreError::Busy)));
        drop(reserved);
        assert!(state.begin_operation().is_ok());
    }

    #[test]
    fn audit_events_follow_the_current_configured_destination() {
        let state_directory = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let ledger = Ledger::open(state_directory.path().join("ledger.sqlite3")).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();
        let fields = [("count", AuditValue::Unsigned(2))];

        let path = state
            .append_audit(
                &AuditEvent {
                    occurred_at: datetime!(2026-08-09 20:01:42.613 +09:00),
                    level: AuditLevel::Info,
                    code: "scan.complete",
                    transmitter: None,
                    fields: &fields,
                },
                AuditDurability::SyncData,
            )
            .unwrap();

        assert_eq!(
            path,
            destination
                .path()
                .join("logs/2026/08/260809-backup-mic.log")
        );
    }

    #[test]
    fn audit_events_do_not_touch_an_unconfirmed_default_destination() {
        let state_directory = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let ledger = Ledger::open(state_directory.path().join("ledger.sqlite3")).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), false, false).unwrap();

        let result = state.append_audit(
            &AuditEvent {
                occurred_at: datetime!(2026-08-09 20:01:42.613 +09:00),
                level: AuditLevel::Info,
                code: "device.detected",
                transmitter: Some(Transmitter::Tx01),
                fields: &[],
            },
            AuditDurability::Buffered,
        );

        assert!(matches!(result, Err(CoreError::InvalidRequest)));
        assert!(!destination.path().join("logs").exists());
    }

    #[test]
    fn automatic_backup_uses_the_persisted_typed_preference() {
        let state_directory = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let mut ledger = Ledger::open(state_directory.path().join("ledger.sqlite3")).unwrap();
        ledger
            .set_preference(
                PreferenceKey::AutomaticBackup,
                false,
                "2026-08-09T00:00:00Z",
            )
            .unwrap();

        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();

        assert!(!state.automatic_backup_enabled());
        assert!(state.m4a_conversion_enabled());
        assert!(!state.automatic_trash_enabled());
        assert_eq!(
            state.snapshot().settings,
            BackupSettingsDto {
                automatic_backup: false,
                m4a_conversion: true,
                automatic_trash: false,
                autostart: false,
            }
        );
    }

    #[test]
    fn setting_a_preference_persists_before_updating_the_snapshot() {
        let state_directory = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let ledger_path = state_directory.path().join("ledger.sqlite3");
        let ledger = Ledger::open(&ledger_path).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();

        let snapshot = state
            .set_preference(PreferenceKey::AutomaticTrash, true, "2026-08-09T00:00:00Z")
            .unwrap();

        assert!(snapshot.settings.automatic_trash);
        assert_eq!(snapshot.retirement_mode, RetirementModeDto::Automatic);
        assert!(state.automatic_trash_enabled());
        drop(state);
        let reopened = Ledger::open(ledger_path).unwrap();
        assert!(reopened.read_preferences().unwrap().automatic_trash);
    }

    #[test]
    fn setting_changed_during_an_operation_is_marked_for_the_next_run() {
        let state_directory = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let ledger = Ledger::open(state_directory.path().join("ledger.sqlite3")).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();
        let _operation = state.begin_operation().unwrap();

        let snapshot = state.apply_persisted_preferences(BackupPreferences {
            automatic_backup: false,
            ..BackupPreferences::default()
        });

        assert!(snapshot.setting_applies_next_run);
        assert!(!snapshot.settings.automatic_backup);
    }

    #[test]
    fn log_directory_is_scoped_to_the_configured_destination_and_local_month() {
        let state_directory = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let ledger = Ledger::open(state_directory.path().join("ledger.sqlite3")).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();

        assert_eq!(
            state
                .log_directory(datetime!(2026-08-09 20:01:42.613 +09:00))
                .unwrap(),
            destination.path().join("logs/2026/08")
        );
    }

    #[test]
    fn log_directory_refuses_an_unconfirmed_default_destination() {
        let state_directory = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let ledger = Ledger::open(state_directory.path().join("ledger.sqlite3")).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), false, false).unwrap();

        assert!(matches!(
            state.log_directory(datetime!(2026-08-09 20:01:42.613 +09:00)),
            Err(CoreError::InvalidRequest)
        ));
        assert!(!destination.path().join("logs").exists());
    }
}
