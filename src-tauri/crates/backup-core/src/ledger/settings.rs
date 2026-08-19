//! The key-value settings table: destination, first-run marker, and preferences.

use rusqlite::{OptionalExtension, params};

use crate::error::CoreError;
use crate::initial_setup::DESTINATION_SETTING;
use crate::initial_setup::INITIAL_SETUP_SETTING;
use crate::initial_setup::InitialSetupMarker;
use crate::preferences::BackupPreferences;
use crate::preferences::PreferenceFlag;
use crate::preferences::PreferenceLimit;
use crate::preferences::decode_bool;
use crate::preferences::decode_limit;

use super::Ledger;

impl Ledger {
    pub fn set_setting(
        &mut self,
        key: &str,
        value_json: &str,
        updated_at: &str,
    ) -> Result<(), CoreError> {
        self.connection
            .execute(
                r#"INSERT INTO settings(key, value_json, updated_at) VALUES (?1, ?2, ?3)
                   ON CONFLICT(key) DO UPDATE SET
                     value_json = excluded.value_json,
                     updated_at = excluded.updated_at"#,
                params![key, value_json, updated_at],
            )
            .map_err(CoreError::Ledger)?;
        Ok(())
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>, CoreError> {
        self.connection
            .query_row(
                "SELECT value_json FROM settings WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()
            .map_err(CoreError::Ledger)
    }

    pub fn initial_setup_marker(&self) -> Result<Option<InitialSetupMarker>, CoreError> {
        InitialSetupMarker::decode(self.setting(INITIAL_SETUP_SETTING)?)
    }

    pub fn persist_destination(
        &mut self,
        destination_json: &str,
        require_settings_review: bool,
        updated_at: &str,
    ) -> Result<(), CoreError> {
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        transaction
            .execute(
                r#"INSERT INTO settings(key, value_json, updated_at) VALUES (?1, ?2, ?3)
                   ON CONFLICT(key) DO UPDATE SET
                     value_json = excluded.value_json,
                     updated_at = excluded.updated_at"#,
                params![DESTINATION_SETTING, destination_json, updated_at],
            )
            .map_err(CoreError::Ledger)?;
        if require_settings_review {
            transaction
                .execute(
                    r#"INSERT INTO settings(key, value_json, updated_at) VALUES (?1, ?2, ?3)
                       ON CONFLICT(key) DO UPDATE SET
                         value_json = excluded.value_json,
                         updated_at = excluded.updated_at"#,
                    params![
                        INITIAL_SETUP_SETTING,
                        InitialSetupMarker::SettingsReviewPending.encode(),
                        updated_at
                    ],
                )
                .map_err(CoreError::Ledger)?;
        }
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn complete_initial_setup(&mut self, updated_at: &str) -> Result<(), CoreError> {
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        let changed = transaction
            .execute(
                r#"UPDATE settings SET value_json = ?1, updated_at = ?2
                   WHERE key = ?3 AND value_json = ?4"#,
                params![
                    InitialSetupMarker::Complete.encode(),
                    updated_at,
                    INITIAL_SETUP_SETTING,
                    InitialSetupMarker::SettingsReviewPending.encode()
                ],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::InvalidRequest);
        }
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn read_preferences(&self) -> Result<BackupPreferences, CoreError> {
        let defaults = BackupPreferences::default();
        Ok(BackupPreferences {
            automatic_backup: decode_bool(
                self.setting(PreferenceFlag::AutomaticBackup.storage_key())?,
                defaults.automatic_backup,
            )?,
            m4a_conversion: decode_bool(
                self.setting(PreferenceFlag::M4aConversion.storage_key())?,
                defaults.m4a_conversion,
            )?,
            automatic_trash: decode_bool(
                self.setting(PreferenceFlag::AutomaticTrash.storage_key())?,
                defaults.automatic_trash,
            )?,
            free_space_reserve_gib: self.limit(PreferenceLimit::FreeSpaceReserveGib)?,
            rescan_interval_seconds: self.limit(PreferenceLimit::RescanIntervalSeconds)?,
        })
    }

    fn limit(&self, limit: PreferenceLimit) -> Result<u32, CoreError> {
        decode_limit(self.setting(limit.storage_key())?, limit)
    }

    pub fn set_flag(
        &mut self,
        flag: PreferenceFlag,
        value: bool,
        updated_at: &str,
    ) -> Result<(), CoreError> {
        self.set_setting(
            flag.storage_key(),
            if value { "true" } else { "false" },
            updated_at,
        )
    }

    /// Stores a whole-number limit after the limit itself has accepted the value, so an
    /// out-of-range request never reaches the settings table.
    pub fn set_limit(
        &mut self,
        limit: PreferenceLimit,
        value: u32,
        updated_at: &str,
    ) -> Result<(), CoreError> {
        let accepted = limit.accept(value)?;
        self.set_setting(limit.storage_key(), &accepted.to_string(), updated_at)
    }
}
