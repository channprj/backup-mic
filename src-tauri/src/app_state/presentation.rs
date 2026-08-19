//! Shaping the snapshot React renders, including the redacted error it may show.

use backup_core::error::{CoreError, PublicError};
use backup_core::initial_setup::InitialSetupMarker;
use backup_core::source::SourceId;
use backup_core::state::{BackupPhase, DeletionPhase, Transmitter};
use tauri::AppHandle;

use crate::dto::{AppSnapshotDto, NotificationStatusDto, SetupStateDto, SourceSnapshotDto};

use super::{AppState, RuntimeState, publish_locked};

impl AppState {
    pub fn set_error(&self, app: &AppHandle, error: CoreError, transmitter: Option<Transmitter>) {
        let public = error.public(transmitter);
        let mut runtime = self.runtime.lock();
        runtime.snapshot.phase = BackupPhase::Error;
        runtime.snapshot.message_code = public.message_code.clone();
        runtime.snapshot.error = Some(public);
        runtime.snapshot.failure_stage = runtime.snapshot.current_stage;
        runtime.snapshot.current_stage = None;
        runtime.snapshot.setting_applies_next_run = false;
        publish_locked(app, &mut runtime);
    }

    pub fn set_waiting_for_device(&self, app: &AppHandle) {
        let mut runtime = self.runtime.lock();
        runtime.snapshot.phase = BackupPhase::Detecting;
        runtime.snapshot.message_code = "waiting_for_device".to_owned();
        runtime.snapshot.error = None;
        runtime.snapshot.failure_stage = None;
        runtime.snapshot.current_stage = None;
        runtime.snapshot.setting_applies_next_run = false;
        runtime.snapshot.current_item_ordinal = None;
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
        runtime.snapshot.error = Some(public.clone());
        runtime.snapshot.failure_stage = runtime.snapshot.current_stage;
        runtime.snapshot.current_stage = None;
        runtime.snapshot.setting_applies_next_run = false;
        update_source(&mut runtime.snapshot, source_id, |snapshot| {
            snapshot.phase = BackupPhase::Error;
            snapshot.retirement_outcome = DeletionPhase::Refused;
            snapshot.deletion_ready = false;
            snapshot.error = Some(public.clone());
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

    pub fn public_error(error: CoreError, transmitter: Option<Transmitter>) -> PublicError {
        error.public(transmitter)
    }

    pub fn awaiting_deletion_source(&self) -> Option<SourceId> {
        self.runtime.lock().awaiting_rule_deletion.clone()
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

pub(super) fn clear_retirement_authority(runtime: &mut RuntimeState) {
    runtime.rule_deletions.clear();
    runtime.rule_scan_generations.clear();
    runtime.awaiting_rule_deletion = None;
    for source in &mut runtime.snapshot.sources {
        source.deletion_ready = false;
        source.retirement_outcome = DeletionPhase::Inactive;
    }
}

pub(super) fn setup_state_for(
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
