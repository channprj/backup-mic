//! Approving a backup folder and completing first-run setup.

use std::path::PathBuf;

use backup_core::deletion::ProposalInvalidation;
use backup_core::error::CoreError;
use backup_core::state::DeletionPhase;
use tauri::AppHandle;

use crate::dto::{AppSnapshotDto, SetupStateDto, destination_display_for};

use super::{AppState, publish_locked};

impl AppState {
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
}
