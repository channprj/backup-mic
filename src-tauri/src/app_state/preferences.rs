//! Reading and writing the persisted preferences, and republishing what the ledger holds.

use std::time::Duration;

use backup_core::batch::FrozenPreferences;
use backup_core::error::CoreError;
use backup_core::preferences::{BackupPreferences, PreferenceFlag, PreferenceLimit};

use crate::dto::{AppSnapshotDto, ArtifactFormatDto, RetirementModeDto};

use super::AppState;
use super::presentation::clear_retirement_authority;

impl AppState {
    pub fn automatic_backup_enabled(&self) -> bool {
        self.runtime.lock().preferences.automatic_backup
    }

    pub fn m4a_conversion_enabled(&self) -> bool {
        self.runtime.lock().preferences.m4a_conversion
    }

    /// The free space the destination capacity preflight must leave behind.
    pub fn free_space_reserve_bytes(&self) -> u64 {
        self.runtime.lock().preferences.free_space_reserve_bytes()
    }

    /// How long a mounted recorder waits between metadata rescans. Read on every device-monitor
    /// tick so a change in the settings window takes effect without a restart.
    pub fn rescan_interval(&self) -> Duration {
        self.runtime.lock().preferences.rescan_interval()
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

    /// Writes a flag and reads back the preferences the ledger actually holds.
    ///
    /// Synchronous, and it takes the ledger lock, so a caller on the async runtime must run it
    /// in a blocking task. The read-back is the point: the snapshot can then only show a value
    /// that survived the write.
    pub fn persist_flag(
        &self,
        flag: PreferenceFlag,
        enabled: bool,
        occurred_at: &str,
    ) -> Result<BackupPreferences, CoreError> {
        let mut ledger = self.ledger.lock();
        ledger.set_flag(flag, enabled, occurred_at)?;
        ledger.read_preferences()
    }

    /// Writes a whole-number limit and reads back what the ledger holds. Same blocking contract
    /// as [`Self::persist_flag`]; the limit refuses an out-of-range value before any write.
    pub fn persist_limit(
        &self,
        limit: PreferenceLimit,
        value: u32,
        occurred_at: &str,
    ) -> Result<BackupPreferences, CoreError> {
        let mut ledger = self.ledger.lock();
        ledger.set_limit(limit, value, occurred_at)?;
        ledger.read_preferences()
    }

    pub fn apply_persisted_preferences(&self, preferences: BackupPreferences) -> AppSnapshotDto {
        let mut runtime = self.runtime.lock();
        runtime.preferences = preferences;
        runtime.snapshot.settings.automatic_backup = preferences.automatic_backup;
        runtime.snapshot.settings.m4a_conversion = preferences.m4a_conversion;
        runtime.snapshot.settings.automatic_trash = preferences.automatic_trash;
        runtime.snapshot.settings.free_space_reserve_gib = preferences.free_space_reserve_gib;
        runtime.snapshot.settings.rescan_interval_seconds = preferences.rescan_interval_seconds;
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
}
