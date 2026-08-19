//! Admitting one operation at a time, and deciding whether a backup may start.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use backup_core::backup::CancellationToken;
use backup_core::error::CoreError;
use backup_core::events::{ActivityEntry, ActivitySeverity};
use backup_core::source::SourceId;
use backup_core::state::{BackupPhase, DeletionPhase, Progress};
use parking_lot::Mutex;
use tauri::AppHandle;

use crate::dto::{AppSnapshotDto, ProgressDto, SetupStateDto};
use crate::manual_backup::ManualBackupClaim;

use super::devices::canonical_destination_is_separate;
use super::presentation::clear_retirement_authority;
use super::{AppState, publish_locked};

impl AppState {
    pub(crate) fn request_manual_backup(&self) {
        self.manual_backup.request();
    }

    pub(crate) fn mark_manual_backup_waiting(&self) {
        self.manual_backup.wait_for_device();
    }

    pub(crate) fn claim_manual_backup(&self, sources: &[SourceId]) -> Option<ManualBackupClaim> {
        self.manual_backup.claim(sources)
    }

    pub(crate) fn manual_backup_should_start_for(&self, source_id: &SourceId) -> bool {
        self.manual_backup.should_start_for(source_id)
    }

    pub(crate) fn manual_backup_is_waiting(&self) -> bool {
        self.manual_backup.is_waiting()
    }

    pub(crate) fn finish_manual_backup(&self, claim: ManualBackupClaim) {
        self.manual_backup.finish(claim);
    }

    pub(crate) fn interrupt_manual_backup<I>(&self, sources: I)
    where
        I: IntoIterator<Item = SourceId>,
    {
        self.manual_backup.interrupt(sources);
    }

    pub(crate) fn interrupt_manual_backup_claim<I>(
        &self,
        claim: ManualBackupClaim,
        sources: I,
        cancelled: bool,
    ) where
        I: IntoIterator<Item = SourceId>,
    {
        self.manual_backup
            .interrupt_claim(claim, sources, cancelled);
    }

    pub(crate) fn cancel_manual_backup(&self) {
        self.manual_backup.cancel();
    }

    pub fn backup_is_ready(&self) -> bool {
        self.backup_start_preflight().is_ok()
    }

    pub(crate) fn backup_start_preflight(&self) -> Result<(), CoreError> {
        self.backup_configuration_preflight()?;
        if self.runtime.lock().matched.is_empty() {
            return Err(CoreError::DeviceRemoved);
        }
        Ok(())
    }

    pub(crate) fn backup_configuration_preflight(&self) -> Result<(), CoreError> {
        let (destination_configured, setup_state, destination, source_roots) = {
            let runtime = self.runtime.lock();
            (
                runtime.destination_configured,
                runtime.snapshot.setup_state,
                runtime.destination.clone(),
                runtime
                    .matched
                    .values()
                    .map(|matched| matched.authority.descriptor.mount_root.clone())
                    .collect::<Vec<_>>(),
            )
        };

        if !destination_configured || setup_state != SetupStateDto::Ready {
            return Err(CoreError::InvalidRequest);
        }
        if !canonical_destination_is_separate(&destination, &source_roots) {
            return Err(CoreError::InvalidRequest);
        }
        Ok(())
    }

    pub fn operation_is_active(&self) -> bool {
        self.operation_active.load(Ordering::SeqCst)
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

    pub fn cancel_active_operation(&self) -> bool {
        let active = self.operation_is_active();
        self.cancellation.lock().cancel();
        self.cancel_manual_backup();
        active
    }

    pub fn settle_cancelled_operation(&self, app: &AppHandle) -> bool {
        let settled = {
            let mut runtime = self.runtime.lock();
            if !settle_cancelled_snapshot(&mut runtime.snapshot) {
                return false;
            }
            clear_retirement_authority(&mut runtime);
            publish_locked(app, &mut runtime);
            true
        };
        if settled {
            let activity = ActivityEntry {
                occurred_at: time::OffsetDateTime::now_utc()
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned()),
                code: "backup_cancelled".to_owned(),
                source_id: None,
                source_label: None,
                count_value: None,
                byte_value: None,
                severity: ActivitySeverity::Info,
            };
            if let Err(error) = self.record_activity(app, activity) {
                self.report_failure("cancel_backup", "activity_persistence", &error, None, None);
            }
        }
        settled
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

pub(super) fn backup_requirements_met(
    destination_configured: bool,
    setup_state: SetupStateDto,
    mounted_devices: usize,
) -> bool {
    destination_configured && setup_state == SetupStateDto::Ready && mounted_devices > 0
}

pub(super) fn backup_should_schedule(
    automatic_backup: bool,
    manual_backup_pending: bool,
    destination_configured: bool,
    setup_state: SetupStateDto,
    mounted_devices: usize,
) -> bool {
    (automatic_backup || manual_backup_pending)
        && backup_requirements_met(destination_configured, setup_state, mounted_devices)
}

pub(super) fn settle_cancelled_snapshot(snapshot: &mut AppSnapshotDto) -> bool {
    let active = matches!(
        snapshot.phase,
        BackupPhase::Detecting
            | BackupPhase::Scanning
            | BackupPhase::CheckingCapacity
            | BackupPhase::Copying
            | BackupPhase::Verifying
    ) || snapshot.current_stage.is_some();
    if !active {
        return false;
    }

    snapshot.phase = BackupPhase::Idle;
    snapshot.message_code = "operation_cancelled".to_owned();
    snapshot.overall_progress = ProgressDto::from(&Progress::default());
    snapshot.current_stage = None;
    snapshot.failure_stage = None;
    snapshot.setting_applies_next_run = false;
    snapshot.current_item_ordinal = None;
    snapshot.error = None;
    for source in &mut snapshot.sources {
        source.phase = BackupPhase::Idle;
        source.progress = ProgressDto::from(&Progress::default());
        source.retirement_outcome = DeletionPhase::Inactive;
        source.deletion_ready = false;
        source.error = None;
    }
    true
}
