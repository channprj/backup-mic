use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use backup_core::{
    audit_log::{AuditDurability, AuditEvent, AuditLevel, AuditSink, AuditValue, FileAuditLog},
    backup::CancellationToken,
    deletion::{DeletionCandidate, DeletionProposalStore, ProposalInvalidation},
    device::{DeviceMatch, PairedDevice},
    error::{CoreError, PublicError},
    events::{ActivityEntry, ActivitySeverity},
    ledger::Ledger,
    preferences::{BackupPreferences, PreferenceKey},
    state::{BackupPhase, DeletionPhase, Progress, Transmitter},
};
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};

use crate::{
    dto::{
        AppSnapshotDto, BackupSettingsDto, NotificationStatusDto, ProgressDto, SetupStateDto,
        TransmitterSnapshotDto,
    },
    pairing::{PairingAssignment, PairingManager},
    platform::device_registry::{MountedVolume, VolumeLifecycleEvent},
};

pub const SNAPSHOT_EVENT: &str = "app-snapshot-changed";

pub(crate) struct RuntimeState {
    pub snapshot: AppSnapshotDto,
    pub pairing: PairingManager,
    pub paired: Vec<PairedDevice>,
    pub observed: HashMap<String, MountedVolume>,
    pub mounted: HashMap<Transmitter, MountedVolume>,
    pub destination: PathBuf,
    pub destination_configured: bool,
    pub destination_generation: u64,
    pub scan_generations: HashMap<Transmitter, u64>,
    pub verified: HashMap<Transmitter, Vec<DeletionCandidate>>,
    pub current_source_paths: HashMap<Transmitter, BTreeSet<PathBuf>>,
    pub preferences: BackupPreferences,
}

#[derive(Clone)]
pub struct AppState {
    pub(crate) runtime: Arc<Mutex<RuntimeState>>,
    pub(crate) ledger: Arc<Mutex<Ledger>>,
    pub(crate) proposals: Arc<Mutex<DeletionProposalStore>>,
    operation_active: Arc<AtomicBool>,
    cancellation: Arc<Mutex<CancellationToken>>,
    started: Instant,
}

impl AppState {
    pub fn new(
        ledger: Ledger,
        destination: PathBuf,
        destination_configured: bool,
        autostart_enabled: bool,
    ) -> Result<Self, CoreError> {
        let paired = ledger.paired_devices()?;
        let preferences = ledger.read_preferences()?;
        let setup_state = if !destination_configured {
            SetupStateDto::NeedsDestination
        } else if paired.len() == 2 {
            SetupStateDto::Ready
        } else {
            SetupStateDto::NeedsPairing
        };
        Ok(Self {
            runtime: Arc::new(Mutex::new(RuntimeState {
                snapshot: AppSnapshotDto {
                    revision: 1,
                    phase: BackupPhase::Idle,
                    message_code: "idle".to_owned(),
                    overall_progress: ProgressDto::from(&Progress::default()),
                    transmitters: vec![
                        transmitter_snapshot(Transmitter::Tx01),
                        transmitter_snapshot(Transmitter::Tx02),
                    ],
                    current_stage: None,
                    current_item_ordinal: None,
                    last_success_at: None,
                    autostart_enabled,
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
                mounted: HashMap::new(),
                destination,
                destination_configured,
                destination_generation: 1,
                scan_generations: HashMap::new(),
                verified: HashMap::new(),
                current_source_paths: HashMap::new(),
                preferences,
            })),
            ledger: Arc::new(Mutex::new(ledger)),
            proposals: Arc::new(Mutex::new(DeletionProposalStore::default())),
            operation_active: Arc::new(AtomicBool::new(false)),
            cancellation: Arc::new(Mutex::new(CancellationToken::default())),
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

    pub fn backup_is_ready(&self) -> bool {
        let runtime = self.runtime.lock();
        backup_requirements_met(
            runtime.destination_configured,
            runtime.paired.len(),
            runtime.mounted.len(),
        )
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
            }
            PreferenceKey::AutomaticTrash => {
                runtime.preferences.automatic_trash = enabled;
                runtime.snapshot.settings.automatic_trash = enabled;
            }
        }
        runtime.snapshot.revision = runtime.snapshot.revision.saturating_add(1);
        Ok(runtime.snapshot.clone())
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

    pub fn cancel_active_operation(&self) {
        self.cancellation.lock().cancel();
    }

    pub fn elapsed(&self) -> std::time::Duration {
        self.started.elapsed()
    }

    pub fn begin_operation(&self) -> Result<OperationGuard, CoreError> {
        self.operation_active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| CoreError::Busy)?;
        let cancellation = CancellationToken::default();
        *self.cancellation.lock() = cancellation.clone();
        Ok(OperationGuard {
            active: Arc::clone(&self.operation_active),
            cancellation,
        })
    }

    pub fn handle_lifecycle(
        &self,
        app: &AppHandle,
        event: VolumeLifecycleEvent,
    ) -> Option<Transmitter> {
        match event {
            VolumeLifecycleEvent::Mounted(mounted) => self.handle_mounted(app, mounted),
            VolumeLifecycleEvent::Unmounted {
                volume_uuid,
                mount_generation,
            } => {
                self.cancellation.lock().cancel();
                self.proposals
                    .lock()
                    .invalidate(ProposalInvalidation::DeviceDisappeared);
                let mut runtime = self.runtime.lock();
                runtime.pairing.remove(&volume_uuid, mount_generation);
                runtime.observed.remove(&volume_uuid);
                let removed = runtime
                    .mounted
                    .iter()
                    .find(|(_, mounted)| {
                        mounted
                            .descriptor
                            .volume_uuid
                            .eq_ignore_ascii_case(&volume_uuid)
                            && mounted.descriptor.mount_generation == mount_generation
                    })
                    .map(|(transmitter, _)| *transmitter);
                if let Some(transmitter) = removed {
                    runtime.mounted.remove(&transmitter);
                    runtime.verified.remove(&transmitter);
                    runtime.current_source_paths.remove(&transmitter);
                    update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
                        snapshot.mounted = false;
                        snapshot.phase = BackupPhase::Idle;
                        snapshot.deletion_ready = false;
                        snapshot.deletion_phase = DeletionPhase::Inactive;
                    });
                }
                sync_pairing_snapshot(&mut runtime);
                publish_locked(app, &mut runtime);
                drop(runtime);
                if let Some(transmitter) = removed {
                    let _ = self.record_activity(
                        app,
                        activity_entry("device_removed", transmitter, ActivitySeverity::Warning),
                    );
                    let fields = [("reason", AuditValue::Text("device_removed"))];
                    if self.runtime.lock().destination_configured
                        && let Err(error) = self.append_audit(
                            &AuditEvent {
                                occurred_at: local_now(),
                                level: AuditLevel::Warning,
                                code: "device.removed",
                                transmitter: Some(transmitter),
                                fields: &fields,
                            },
                            AuditDurability::Buffered,
                        )
                    {
                        self.set_error(app, error, Some(transmitter));
                    }
                }
                None
            }
        }
    }

    fn handle_mounted(&self, app: &AppHandle, mounted: MountedVolume) -> Option<Transmitter> {
        let mut runtime = self.runtime.lock();
        let volume_uuid = mounted.descriptor.volume_uuid.clone();
        runtime.observed.insert(volume_uuid, mounted.clone());
        let paired = runtime.paired.clone();
        let result = runtime.pairing.observe(mounted.clone(), &paired);
        let trusted = match result {
            DeviceMatch::Trusted(transmitter) => {
                runtime.mounted.insert(transmitter, mounted);
                update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
                    snapshot.mounted = true;
                    snapshot.phase = BackupPhase::Detecting;
                    snapshot.deletion_ready = false;
                });
                runtime.snapshot.phase = BackupPhase::Detecting;
                runtime.snapshot.message_code = "device_detected".to_owned();
                Some(transmitter)
            }
            DeviceMatch::Rejected(error) => {
                runtime.snapshot.phase = BackupPhase::Error;
                runtime.snapshot.message_code = error.message_code.clone();
                runtime.snapshot.error = Some(error);
                None
            }
            DeviceMatch::UnpairedCandidate | DeviceMatch::Unrelated => None,
        };
        sync_pairing_snapshot(&mut runtime);
        let should_schedule = trusted.filter(|_| {
            runtime.preferences.automatic_backup
                && backup_requirements_met(
                    runtime.destination_configured,
                    runtime.paired.len(),
                    runtime.mounted.len(),
                )
        });
        publish_locked(app, &mut runtime);
        drop(runtime);
        if let Some(transmitter) = trusted {
            let _ = self.record_activity(
                app,
                activity_entry("device_detected", transmitter, ActivitySeverity::Info),
            );
            if self.runtime.lock().destination_configured
                && let Err(error) = self.append_audit(
                    &AuditEvent {
                        occurred_at: local_now(),
                        level: AuditLevel::Info,
                        code: "device.detected",
                        transmitter: Some(transmitter),
                        fields: &[],
                    },
                    AuditDurability::Buffered,
                )
            {
                self.set_error(app, error, Some(transmitter));
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

    pub fn set_destination(&self, app: &AppHandle, destination: PathBuf) {
        self.proposals
            .lock()
            .invalidate(ProposalInvalidation::DestinationChanged);
        let mut runtime = self.runtime.lock();
        runtime.destination = destination;
        runtime.destination_configured = true;
        runtime.destination_generation = runtime.destination_generation.saturating_add(1);
        runtime.verified.clear();
        runtime.current_source_paths.clear();
        for snapshot in &mut runtime.snapshot.transmitters {
            snapshot.deletion_ready = false;
            snapshot.deletion_phase = DeletionPhase::Inactive;
        }
        runtime.snapshot.setup_state = if runtime.paired.len() == 2 {
            SetupStateDto::Ready
        } else {
            SetupStateDto::NeedsPairing
        };
        publish_locked(app, &mut runtime);
    }

    pub fn record_activity(&self, app: &AppHandle, entry: ActivityEntry) -> Result<(), CoreError> {
        let recent_activity = {
            let mut ledger = self.ledger.lock();
            ledger.append_activity(&entry)?;
            ledger.recent_activity(8)?
        };
        let mut runtime = self.runtime.lock();
        runtime.snapshot.recent_activity = recent_activity;
        publish_locked(app, &mut runtime);
        Ok(())
    }

    pub fn set_error(&self, app: &AppHandle, error: CoreError, transmitter: Option<Transmitter>) {
        let public = error.public(transmitter);
        let mut runtime = self.runtime.lock();
        runtime.snapshot.phase = BackupPhase::Error;
        runtime.snapshot.message_code = public.message_code.clone();
        runtime.snapshot.error = Some(public);
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
        update_transmitter(&mut runtime.snapshot, transmitter, |snapshot| {
            snapshot.deletion_phase = DeletionPhase::Refused;
            snapshot.deletion_ready = false;
        });
        publish_locked(app, &mut runtime);
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
        runtime.snapshot.autostart_enabled = enabled;
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
            || runtime.snapshot.transmitters.iter().any(|snapshot| {
                matches!(
                    snapshot.deletion_phase,
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
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.active.store(false, Ordering::SeqCst);
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

fn sync_pairing_snapshot(runtime: &mut RuntimeState) {
    runtime.snapshot.pairing_candidates = runtime.pairing.summaries();
    runtime.snapshot.setup_state = if !runtime.destination_configured {
        SetupStateDto::NeedsDestination
    } else if runtime.paired.len() == 2 {
        SetupStateDto::Ready
    } else {
        SetupStateDto::NeedsPairing
    };
}

fn backup_requirements_met(
    destination_configured: bool,
    paired_devices: usize,
    mounted_devices: usize,
) -> bool {
    destination_configured && paired_devices == 2 && mounted_devices > 0
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

fn activity_entry(
    code: &str,
    transmitter: Transmitter,
    severity: ActivitySeverity,
) -> ActivityEntry {
    ActivityEntry {
        occurred_at: time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned()),
        code: code.to_owned(),
        transmitter: Some(transmitter),
        count_value: None,
        byte_value: None,
        severity,
    }
}

fn local_now() -> time::OffsetDateTime {
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    time::OffsetDateTime::now_utc().to_offset(offset)
}

#[cfg(test)]
mod tests {
    use backup_core::audit_log::{AuditDurability, AuditEvent, AuditLevel, AuditValue};
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
    fn automatic_backup_waits_for_every_setup_requirement() {
        assert!(!backup_requirements_met(false, 2, 2));
        assert!(!backup_requirements_met(true, 1, 1));
        assert!(!backup_requirements_met(true, 2, 0));
        assert!(backup_requirements_met(true, 2, 1));
        assert!(backup_requirements_met(true, 2, 2));
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
        assert!(state.automatic_trash_enabled());
        drop(state);
        let reopened = Ledger::open(ledger_path).unwrap();
        assert!(reopened.read_preferences().unwrap().automatic_trash);
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
