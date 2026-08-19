//! The one canonical application state, and the snapshot React renders from it.
//!
//! Everything the UI shows comes from `RuntimeState` behind a single lock. Blocking work never
//! runs while that lock is held, so the submodules here are grouped by which part of the state
//! they own rather than by which command calls them.

mod destination;
mod devices;
mod operations;
mod preferences;
mod presentation;
mod reporting;
mod rules;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use backup_core::backup::CancellationToken;
use backup_core::deletion::{CompleteRuleDeletionSnapshot, DeletionProposalStore};
use backup_core::error::CoreError;
use backup_core::ledger::Ledger;
use backup_core::preferences::BackupPreferences;
use backup_core::rule::BackupRule;
use backup_core::source::{MountedSourceAuthority, SourceId};
use backup_core::state::{BackupPhase, Progress};
use parking_lot::Mutex;
use presentation::setup_state_for;
use tauri::{AppHandle, Emitter};

use crate::dto::{
    AppSnapshotDto, ArtifactFormatDto, BackupRuleDto, BackupSettingsDto, NotificationStatusDto,
    ProgressDto, RetirementModeDto, SourceSnapshotDto, destination_display_for,
};
use crate::failure_reporter::FailureReporter;
use crate::manual_backup::ManualBackupIntent;
use crate::platform::device_registry::MountedVolume;
use crate::rule_runtime::MatchedSource;

pub use operations::OperationGuard;
pub(crate) use presentation::update_source;

pub const SNAPSHOT_EVENT: &str = "app-snapshot-changed";

pub(crate) struct RuntimeState {
    pub snapshot: AppSnapshotDto,
    pub observed: HashMap<String, MountedVolume>,
    pub matched: HashMap<SourceId, MatchedSource>,
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
    pub(super) operation_active: Arc<AtomicBool>,
    pub(super) operation_reserved: Arc<AtomicBool>,
    pub(super) cancellation: Arc<Mutex<CancellationToken>>,
    pub(super) manual_backup: Arc<ManualBackupIntent>,
    pub(super) failure_reporter: FailureReporter,
    pub(super) started: Instant,
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
                        free_space_reserve_gib: preferences.free_space_reserve_gib,
                        rescan_interval_seconds: preferences.rescan_interval_seconds,
                    },
                    notification_status: NotificationStatusDto::Unknown,
                    setup_state,
                    recent_activity: Vec::new(),
                    error: None,
                },
                observed: HashMap::new(),
                matched: HashMap::new(),
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
            manual_backup: Arc::new(ManualBackupIntent::default()),
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

    pub fn elapsed(&self) -> std::time::Duration {
        self.started.elapsed()
    }
}

pub(crate) fn publish_locked(app: &AppHandle, runtime: &mut RuntimeState) {
    runtime.snapshot.revision = runtime.snapshot.revision.saturating_add(1);
    let _ = app.emit(SNAPSHOT_EVENT, runtime.snapshot.clone());
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use backup_core::audit_log::{AuditDurability, AuditEvent, AuditLevel, AuditValue};
    use backup_core::initial_setup::InitialSetupMarker;
    use backup_core::preferences::PreferenceFlag;
    use backup_core::state::{DeletionPhase, Transmitter};
    use tempfile::tempdir;
    use time::macros::datetime;

    use crate::dto::SetupStateDto;

    use super::devices::{canonical_destination_is_separate, mounted_source_phase};
    use super::operations::{
        backup_requirements_met, backup_should_schedule, settle_cancelled_snapshot,
    };
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
    fn manual_reconnect_schedules_even_when_automatic_backup_is_disabled() {
        assert!(backup_should_schedule(
            false,
            true,
            true,
            SetupStateDto::Ready,
            1
        ));
        assert!(!backup_should_schedule(
            false,
            false,
            true,
            SetupStateDto::Ready,
            1
        ));
    }

    #[test]
    fn manual_backup_can_wait_for_a_missing_recorder() {
        let state_directory = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let ledger = Ledger::open(state_directory.path().join("ledger.sqlite3")).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();

        assert!(state.backup_configuration_preflight().is_ok());
        assert!(matches!(
            state.backup_start_preflight(),
            Err(CoreError::DeviceRemoved)
        ));
        state.request_manual_backup();
        state.mark_manual_backup_waiting();
        assert!(state.manual_backup_is_waiting());

        state.cancel_active_operation();

        assert!(!state.manual_backup_is_waiting());
    }

    #[test]
    fn matched_mount_phase_tracks_whether_backup_is_really_scheduled() {
        assert_eq!(mounted_source_phase(false), BackupPhase::Idle);
        assert_eq!(mounted_source_phase(true), BackupPhase::Detecting);
    }

    #[test]
    fn cancelled_snapshot_returns_every_source_to_safe_idle_state() {
        let state_directory = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let ledger = Ledger::open(state_directory.path().join("ledger.sqlite3")).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();
        let mut snapshot = state.snapshot();
        snapshot.phase = BackupPhase::Scanning;
        snapshot.message_code = "scanning".to_owned();
        snapshot.current_stage = Some(backup_core::state::CurrentStage::Copy);
        snapshot.failure_stage = Some(backup_core::state::CurrentStage::Copy);
        snapshot.setting_applies_next_run = true;
        snapshot.current_item_ordinal = Some(3);
        snapshot.error = Some(CoreError::InvalidRequest.public(None));
        snapshot.sources.push(SourceSnapshotDto {
            source_id: SourceId::new().as_str().to_owned(),
            rule_name: "Zoom".to_owned(),
            volume_name: "ZOOM".to_owned(),
            legacy_slot: None,
            mounted: true,
            phase: BackupPhase::Copying,
            progress: ProgressDto {
                percent: 42,
                copied_bytes: 42,
                bytes_requiring_copy: 100,
                verified_files: 1,
                total_files: 2,
            },
            retirement_outcome: DeletionPhase::Preparing,
            deletion_ready: true,
            error: Some(CoreError::InvalidRequest.public(None)),
        });

        assert!(settle_cancelled_snapshot(&mut snapshot));
        assert_eq!(snapshot.phase, BackupPhase::Idle);
        assert_eq!(snapshot.message_code, "operation_cancelled");
        assert_eq!(
            snapshot.overall_progress,
            ProgressDto::from(&Progress::default())
        );
        assert_eq!(snapshot.current_stage, None);
        assert_eq!(snapshot.failure_stage, None);
        assert!(!snapshot.setting_applies_next_run);
        assert_eq!(snapshot.current_item_ordinal, None);
        assert_eq!(snapshot.error, None);
        assert!(snapshot.sources.iter().all(|source| {
            source.phase == BackupPhase::Idle
                && source.progress == ProgressDto::from(&Progress::default())
                && !source.deletion_ready
                && source.retirement_outcome == DeletionPhase::Inactive
                && source.error.is_none()
        }));
        assert!(!settle_cancelled_snapshot(&mut snapshot));
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
            .set_flag(
                PreferenceFlag::AutomaticBackup,
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
                ..BackupSettingsDto::default()
            }
        );
    }

    #[test]
    fn applying_persisted_preferences_republishes_every_derived_field() {
        let state_directory = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let ledger = Ledger::open(state_directory.path().join("ledger.sqlite3")).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();
        let before = state.snapshot().revision;

        let snapshot = state.apply_persisted_preferences(BackupPreferences {
            automatic_backup: false,
            m4a_conversion: false,
            automatic_trash: true,
            free_space_reserve_gib: 25,
            rescan_interval_seconds: 120,
        });

        assert!(!snapshot.settings.automatic_backup);
        assert!(!snapshot.settings.m4a_conversion);
        assert!(snapshot.settings.automatic_trash);
        assert_eq!(snapshot.settings.free_space_reserve_gib, 25);
        assert_eq!(snapshot.settings.rescan_interval_seconds, 120);
        assert_eq!(snapshot.artifact_format, ArtifactFormatDto::Wav);
        assert_eq!(snapshot.retirement_mode, RetirementModeDto::Automatic);
        assert!(snapshot.revision > before, "React needs a new revision");
        assert_eq!(state.free_space_reserve_bytes(), 25 * 1024 * 1024 * 1024);
        assert_eq!(state.rescan_interval(), Duration::from_secs(120));
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
