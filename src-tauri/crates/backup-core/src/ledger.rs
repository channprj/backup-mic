use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::{
    additional_file::{AdditionalFileClass, VerifiedAdditionalFile},
    artifact::{
        ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact, VerifiedAudioProperties,
    },
    batch::{BatchPhase, BatchRunEvidence, FrozenPreferences},
    device::PairedDevice,
    error::CoreError,
    events::{ActivityEntry, ActivitySeverity},
    initial_setup::{DESTINATION_SETTING, INITIAL_SETUP_SETTING, InitialSetupMarker},
    preferences::{BackupPreferences, PreferenceKey, decode_bool},
    preset::{DJI_PRESET_KIND, DJI_PRESET_REVISION, dji_mic_mini_2s_preset},
    recovery::DELETION_DISABLED_REINDEX_REQUIRED,
    rule::{
        BackupRule, BackupRuleDraft, DateFolderLayout, DeviceConstraintProfile, FilenameProfile,
        RuleId, normalized_rule_name, validate_rule,
    },
    source::{SourceId, SourceRecord, validate_source},
    state::Transmitter,
};

pub const MAX_ACTIVITY_ENTRIES: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRecording {
    pub id: String,
    pub source_id: SourceId,
    pub source_relative_path: PathBuf,
    pub source_size: u64,
    pub source_mtime_ns: i128,
    pub source_sha256: String,
    pub artifact: VerifiedArtifact,
    pub conversion_status: ConversionStatus,
    pub conversion_error_code: Option<String>,
    pub retirement_status: RetirementStatus,
    pub retired_session_relative_path: Option<PathBuf>,
    pub verified_at: String,
    pub backup_run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersededWavEvidence {
    pub recording_id: String,
    pub source_id: SourceId,
    pub relative_path: PathBuf,
    pub byte_count: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDeletionItem {
    pub recording_id: String,
    pub source_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAdditionalDeletionItem {
    pub additional_file_id: String,
    pub source_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyRetiredRecording {
    pub recording_id: String,
    pub source_relative_path: PathBuf,
}

pub struct Ledger {
    connection: Connection,
    path: PathBuf,
    deletion_disabled: bool,
}

impl Ledger {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, CoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(CoreError::LedgerIo)?;
        }
        let mut quarantined = false;
        let connection = if path.exists() && !has_sqlite_header(&path)? {
            quarantine_existing(&path)?;
            quarantined = true;
            open_validated(&path)?
        } else {
            match open_validated(&path) {
                Ok(connection) => connection,
                Err(_) if path.exists() => {
                    quarantine_existing(&path)?;
                    quarantined = true;
                    open_validated(&path)?
                }
                Err(error) => return Err(error),
            }
        };
        migrate(&connection)?;
        if quarantined {
            connection
                .execute(
                    r#"INSERT INTO settings(key, value_json, updated_at)
                       VALUES (?1, ?2, ?3)
                       ON CONFLICT(key) DO UPDATE SET
                         value_json = excluded.value_json,
                         updated_at = excluded.updated_at"#,
                    params![
                        "deletion_disabled_reason",
                        format!("\"{DELETION_DISABLED_REINDEX_REQUIRED}\""),
                        now_timestamp()
                    ],
                )
                .map_err(CoreError::Ledger)?;
        }
        let deletion_disabled = quarantined
            || connection
                .query_row(
                    "SELECT value_json FROM settings WHERE key = 'deletion_disabled_reason'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(CoreError::Ledger)?
                .is_some();
        Ok(Self {
            connection,
            path,
            deletion_disabled,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn deletion_disabled(&self) -> bool {
        self.deletion_disabled
    }

    pub fn backup_rules(&self, include_archived: bool) -> Result<Vec<BackupRule>, CoreError> {
        let query = format!(
            "SELECT {RULE_COLUMNS} FROM backup_rules
             WHERE ?1 OR archived_at IS NULL
             ORDER BY CASE WHEN preset_kind IS NULL THEN 1 ELSE 0 END,
                      normalized_name, id"
        );
        let mut statement = self.connection.prepare(&query).map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([include_archived], row_to_stored_rule)
            .map_err(CoreError::Ledger)?;
        let stored = rows
            .map(|row| row.map_err(CoreError::Ledger))
            .collect::<Result<Vec<_>, _>>()?;

        stored
            .into_iter()
            .map(|row| hydrate_rule(&self.connection, row))
            .collect()
    }

    pub fn backup_rule(&self, id: &RuleId) -> Result<Option<BackupRule>, CoreError> {
        self.backup_rule_by_id(id)
    }

    pub fn lock_rule_archive_directory(
        &mut self,
        id: &RuleId,
        expected_archive_directory: &str,
    ) -> Result<(), CoreError> {
        let changed = self
            .connection
            .execute(
                r#"UPDATE backup_rules SET archive_directory_locked = 1
                   WHERE id = ?1 AND archive_directory_name = ?2"#,
                params![id.as_str(), expected_archive_directory],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::InvalidRule);
        }
        Ok(())
    }

    pub fn save_backup_rule(
        &mut self,
        mut draft: BackupRuleDraft,
        updated_at: &str,
    ) -> Result<BackupRule, CoreError> {
        if updated_at.trim().is_empty() {
            return Err(CoreError::InvalidRule);
        }
        draft.id = draft
            .id
            .as_ref()
            .map(|id| RuleId::parse(id.as_str()))
            .transpose()?;
        let draft = validate_rule(draft)?;
        let existing = draft
            .id
            .as_ref()
            .map(|id| self.backup_rule_by_id(id))
            .transpose()?
            .flatten();
        if draft.id.is_some() && existing.is_none() {
            return Err(CoreError::InvalidRule);
        }
        let id = draft.id.clone().unwrap_or_default();
        let archived_at = existing
            .as_ref()
            .and_then(|rule| rule.archived_at.as_deref());
        let normalized_name = normalized_rule_name(&draft.name);
        if archived_at.is_none() && self.active_rule_name_conflicts(&normalized_name, Some(&id))? {
            return Err(CoreError::InvalidRule);
        }

        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        if let Some(rule) = &existing {
            transaction
                .execute(
                    r#"UPDATE backup_rules SET
                         name = ?1, normalized_name = ?2, archive_directory_name = ?3,
                         enabled = ?4, volume_name_glob = ?5, filename_prefix = ?6,
                         filename_suffix = ?7, date_folder_layout = ?8, updated_at = ?9
                       WHERE id = ?10"#,
                    params![
                        draft.name,
                        normalized_name,
                        draft.archive_directory_name,
                        draft.enabled,
                        draft.volume_name_glob,
                        draft.filename_prefix,
                        draft.filename_suffix,
                        draft.date_folder_layout.storage_name(),
                        updated_at,
                        id.as_str(),
                    ],
                )
                .map_err(rule_write_error)?;
            debug_assert_eq!(rule.id, id);
        } else {
            transaction
                .execute(
                    r#"INSERT INTO backup_rules(
                         id, name, normalized_name, archive_directory_name, enabled,
                         volume_name_glob, filename_prefix, filename_suffix, date_folder_layout,
                         filename_profile, device_constraint_profile, preset_kind,
                         preset_revision, archive_directory_locked, archived_at,
                         created_at, updated_at
                       ) VALUES (
                         ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                         NULL, NULL, 0, NULL, ?12, ?12
                       )"#,
                    params![
                        id.as_str(),
                        draft.name,
                        normalized_name,
                        draft.archive_directory_name,
                        draft.enabled,
                        draft.volume_name_glob,
                        draft.filename_prefix,
                        draft.filename_suffix,
                        draft.date_folder_layout.storage_name(),
                        FilenameProfile::Preserve.storage_name(),
                        DeviceConstraintProfile::GenericExternal.storage_name(),
                        updated_at,
                    ],
                )
                .map_err(rule_write_error)?;
        }
        replace_rule_patterns(&transaction, &id, &draft)?;
        transaction.commit().map_err(CoreError::Ledger)?;

        self.backup_rule_by_id(&id)?.ok_or(CoreError::LedgerCorrupt)
    }

    pub fn archive_backup_rule(&mut self, id: &RuleId, archived_at: &str) -> Result<(), CoreError> {
        if archived_at.trim().is_empty() {
            return Err(CoreError::InvalidRule);
        }
        let reference_state = self
            .connection
            .query_row(
                r#"SELECT preset_kind IS NOT NULL,
                          EXISTS(
                            SELECT 1 FROM rule_device_bindings WHERE rule_id = backup_rules.id
                          ),
                          EXISTS(SELECT 1 FROM sources WHERE rule_id = backup_rules.id)
                   FROM backup_rules WHERE id = ?1"#,
                [id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, bool>(0)?,
                        row.get::<_, bool>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(CoreError::Ledger)?
            .ok_or(CoreError::InvalidRule)?;

        let changed = if reference_state.0 || reference_state.1 || reference_state.2 {
            self.connection
                .execute(
                    r#"UPDATE backup_rules
                       SET enabled = 0, archived_at = ?1, updated_at = ?1
                       WHERE id = ?2"#,
                    params![archived_at, id.as_str()],
                )
                .map_err(CoreError::Ledger)?
        } else {
            self.connection
                .execute("DELETE FROM backup_rules WHERE id = ?1", [id.as_str()])
                .map_err(CoreError::Ledger)?
        };
        if changed != 1 {
            return Err(CoreError::LedgerCorrupt);
        }
        Ok(())
    }

    pub fn restore_dji_preset(&mut self, updated_at: &str) -> Result<BackupRule, CoreError> {
        if updated_at.trim().is_empty() {
            return Err(CoreError::InvalidRule);
        }
        let current = self.dji_rule()?;
        let mut defaults = validate_rule(dji_mic_mini_2s_preset())?;
        defaults.id = Some(current.id.clone());
        defaults.archive_directory_name = current.archive_directory_name;
        let normalized_name = normalized_rule_name(&defaults.name);
        if self.active_rule_name_conflicts(&normalized_name, Some(&current.id))? {
            return Err(CoreError::InvalidRule);
        }

        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        let changed = transaction
            .execute(
                r#"UPDATE backup_rules SET
                     name = ?1, normalized_name = ?2, archive_directory_name = ?3,
                     enabled = 1, volume_name_glob = ?4, filename_prefix = ?5,
                     filename_suffix = ?6, date_folder_layout = ?7, filename_profile = ?8,
                     device_constraint_profile = ?9, preset_revision = ?10,
                     archived_at = NULL, updated_at = ?11
                   WHERE id = ?12 AND preset_kind = ?13"#,
                params![
                    defaults.name,
                    normalized_name,
                    defaults.archive_directory_name,
                    defaults.volume_name_glob,
                    defaults.filename_prefix,
                    defaults.filename_suffix,
                    defaults.date_folder_layout.storage_name(),
                    FilenameProfile::DjiTxShort.storage_name(),
                    DeviceConstraintProfile::DjiMicMini2s.storage_name(),
                    i64::from(DJI_PRESET_REVISION),
                    updated_at,
                    current.id.as_str(),
                    DJI_PRESET_KIND,
                ],
            )
            .map_err(rule_write_error)?;
        if changed != 1 {
            return Err(CoreError::LedgerCorrupt);
        }
        replace_rule_patterns(&transaction, &current.id, &defaults)?;
        transaction.commit().map_err(CoreError::Ledger)?;

        self.dji_rule()
    }

    pub fn dji_rule(&self) -> Result<BackupRule, CoreError> {
        let query = format!("SELECT {RULE_COLUMNS} FROM backup_rules WHERE preset_kind = ?1");
        let stored = self
            .connection
            .query_row(&query, [DJI_PRESET_KIND], row_to_stored_rule)
            .optional()
            .map_err(CoreError::Ledger)?
            .ok_or(CoreError::LedgerCorrupt)?;
        hydrate_rule(&self.connection, stored)
    }

    pub fn upsert_source(&mut self, source: &SourceRecord, seen_at: &str) -> Result<(), CoreError> {
        validate_source(source)?;
        if seen_at.trim().is_empty() {
            return Err(CoreError::InvalidRequest);
        }
        let rule_exists = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM backup_rules WHERE id = ?1)",
                [source.rule_id.as_str()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(CoreError::Ledger)?;
        if !rule_exists {
            return Err(CoreError::InvalidRequest);
        }
        let normalized_uuid = source.volume_uuid.to_ascii_lowercase();
        if let Some(existing) = self.source_by_id(&source.id)? {
            if existing.rule_id != source.rule_id
                || !existing
                    .volume_uuid
                    .eq_ignore_ascii_case(&source.volume_uuid)
                || existing.legacy_slot != source.legacy_slot
            {
                return Err(CoreError::InvalidRequest);
            }
            let changed = self
                .connection
                .execute(
                    r#"UPDATE sources
                       SET volume_uuid = ?1, display_name = ?2, last_seen_at = ?3
                       WHERE id = ?4 AND rule_id = ?5 AND volume_uuid_normalized = ?6"#,
                    params![
                        source.volume_uuid,
                        source.display_name,
                        seen_at,
                        source.id.as_str(),
                        source.rule_id.as_str(),
                        normalized_uuid,
                    ],
                )
                .map_err(source_write_error)?;
            if changed != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
            return Ok(());
        }

        self.connection
            .execute(
                r#"INSERT INTO sources(
                     id, rule_id, volume_uuid, volume_uuid_normalized, legacy_slot,
                     display_name, created_at, last_seen_at
                   ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)"#,
                params![
                    source.id.as_str(),
                    source.rule_id.as_str(),
                    source.volume_uuid,
                    normalized_uuid,
                    source.legacy_slot,
                    source.display_name,
                    seen_at,
                ],
            )
            .map_err(source_write_error)?;
        Ok(())
    }

    pub fn source(&self, id: &SourceId) -> Result<SourceRecord, CoreError> {
        self.source_by_id(id)?.ok_or(CoreError::InvalidRequest)
    }

    pub fn sources_for_rule(&self, rule: &RuleId) -> Result<Vec<SourceRecord>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT id, rule_id, volume_uuid, legacy_slot, display_name
                   FROM sources WHERE rule_id = ?1 ORDER BY legacy_slot, id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([rule.as_str()], row_to_source_record)
            .map_err(CoreError::Ledger)?;
        rows.map(|row| row.map_err(CoreError::Ledger)).collect()
    }

    fn source_by_id(&self, id: &SourceId) -> Result<Option<SourceRecord>, CoreError> {
        self.connection
            .query_row(
                r#"SELECT id, rule_id, volume_uuid, legacy_slot, display_name
                   FROM sources WHERE id = ?1"#,
                [id.as_str()],
                row_to_source_record,
            )
            .optional()
            .map_err(CoreError::Ledger)
    }

    fn backup_rule_by_id(&self, id: &RuleId) -> Result<Option<BackupRule>, CoreError> {
        let query = format!("SELECT {RULE_COLUMNS} FROM backup_rules WHERE id = ?1");
        let stored = self
            .connection
            .query_row(&query, [id.as_str()], row_to_stored_rule)
            .optional()
            .map_err(CoreError::Ledger)?;
        stored
            .map(|row| hydrate_rule(&self.connection, row))
            .transpose()
    }

    fn active_rule_name_conflicts(
        &self,
        normalized_name: &str,
        excluding: Option<&RuleId>,
    ) -> Result<bool, CoreError> {
        self.connection
            .query_row(
                r#"SELECT EXISTS(
                     SELECT 1 FROM backup_rules
                     WHERE normalized_name = ?1 AND archived_at IS NULL
                       AND (?2 IS NULL OR id <> ?2)
                   )"#,
                params![normalized_name, excluding.map(RuleId::as_str)],
                |row| row.get(0),
            )
            .map_err(CoreError::Ledger)
    }

    pub fn pair_device(&mut self, device: &PairedDevice, paired_at: &str) -> Result<(), CoreError> {
        self.pair_devices(std::slice::from_ref(device), paired_at)
    }

    pub fn pair_devices(
        &mut self,
        devices: &[PairedDevice],
        paired_at: &str,
    ) -> Result<(), CoreError> {
        let transmitters = devices
            .iter()
            .map(|device| device.transmitter)
            .collect::<std::collections::HashSet<_>>();
        let uuids = devices
            .iter()
            .map(|device| device.expected_uuid.to_ascii_lowercase())
            .collect::<std::collections::HashSet<_>>();
        if transmitters.len() != devices.len() || uuids.len() != devices.len() {
            return Err(CoreError::InvalidRequest);
        }
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        for device in devices {
            transaction
                .execute(
                    r#"INSERT INTO paired_devices(
                     transmitter, volume_uuid, protocol, media_name, nominal_capacity, paired_at
                   ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                   ON CONFLICT(transmitter) DO UPDATE SET
                     volume_uuid = excluded.volume_uuid,
                     protocol = excluded.protocol,
                     media_name = excluded.media_name,
                     nominal_capacity = excluded.nominal_capacity,
                     paired_at = excluded.paired_at"#,
                    params![
                        transmitter_name(device.transmitter),
                        device.expected_uuid,
                        device.expected_protocol,
                        device.expected_media_name,
                        to_i64(device.expected_capacity)?,
                        paired_at,
                    ],
                )
                .map_err(CoreError::Ledger)?;
        }
        transaction.commit().map_err(CoreError::Ledger)?;
        Ok(())
    }

    pub fn paired_device_count(&self) -> Result<u64, CoreError> {
        let count = self
            .connection
            .query_row("SELECT COUNT(*) FROM paired_devices", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(CoreError::Ledger)?;
        u64::try_from(count).map_err(|_| CoreError::LedgerCorrupt)
    }

    pub fn paired_devices(&self) -> Result<Vec<PairedDevice>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT transmitter, volume_uuid, protocol, media_name, nominal_capacity
                   FROM paired_devices ORDER BY transmitter"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(CoreError::Ledger)?;
        rows.map(|row| {
            let (transmitter, expected_uuid, expected_protocol, expected_media_name, capacity) =
                row.map_err(CoreError::Ledger)?;
            Ok(PairedDevice {
                transmitter: parse_transmitter(&transmitter)?,
                expected_uuid,
                expected_protocol,
                expected_media_name,
                expected_capacity: u64::try_from(capacity).map_err(|_| CoreError::LedgerCorrupt)?,
            })
        })
        .collect()
    }

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
                self.setting(PreferenceKey::AutomaticBackup.storage_key())?,
                defaults.automatic_backup,
            )?,
            m4a_conversion: decode_bool(
                self.setting(PreferenceKey::M4aConversion.storage_key())?,
                defaults.m4a_conversion,
            )?,
            automatic_trash: decode_bool(
                self.setting(PreferenceKey::AutomaticTrash.storage_key())?,
                defaults.automatic_trash,
            )?,
        })
    }

    pub fn set_preference(
        &mut self,
        key: PreferenceKey,
        value: bool,
        updated_at: &str,
    ) -> Result<(), CoreError> {
        self.set_setting(
            key.storage_key(),
            if value { "true" } else { "false" },
            updated_at,
        )
    }

    pub fn append_activity(&mut self, entry: &ActivityEntry) -> Result<(), CoreError> {
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        transaction
            .execute(
                r#"INSERT INTO activity(
                     occurred_at, code, source_id, count_value, byte_value, severity
                   ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
                params![
                    entry.occurred_at,
                    entry.code,
                    entry.source_id.as_ref().map(SourceId::as_str),
                    optional_i64(entry.count_value)?,
                    optional_i64(entry.byte_value)?,
                    severity_name(entry.severity),
                ],
            )
            .map_err(CoreError::Ledger)?;
        transaction
            .execute(
                r#"DELETE FROM activity WHERE id NOT IN (
                     SELECT id FROM activity ORDER BY id DESC LIMIT ?1
                   )"#,
                [i64::try_from(MAX_ACTIVITY_ENTRIES).map_err(|_| CoreError::LedgerCorrupt)?],
            )
            .map_err(CoreError::Ledger)?;
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn recent_activity(&self, limit: usize) -> Result<Vec<ActivityEntry>, CoreError> {
        let bounded = limit.min(MAX_ACTIVITY_ENTRIES);
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT occurred_at, code, source_id, count_value, byte_value, severity
                   FROM activity ORDER BY id DESC LIMIT ?1"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map(
                [i64::try_from(bounded).map_err(|_| CoreError::LedgerCorrupt)?],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .map_err(CoreError::Ledger)?;
        rows.map(|row| {
            let (occurred_at, code, source_id, count_value, byte_value, severity) =
                row.map_err(CoreError::Ledger)?;
            Ok(ActivityEntry {
                occurred_at,
                code,
                source_id: source_id.map(|value| SourceId::parse(&value)).transpose()?,
                source_label: None,
                count_value: optional_u64(count_value)?,
                byte_value: optional_u64(byte_value)?,
                severity: parse_severity(&severity)?,
            })
        })
        .collect()
    }

    pub fn begin_backup_run(
        &mut self,
        id: &str,
        source_id: &SourceId,
        started_at: &str,
        required_copy_bytes: u64,
    ) -> Result<(), CoreError> {
        self.connection
            .execute(
                r#"INSERT INTO backup_runs(
                     id, source_id, started_at, outcome, required_copy_bytes
                   ) VALUES (?1, ?2, ?3, 'running', ?4)"#,
                params![
                    id,
                    source_id.as_str(),
                    started_at,
                    to_i64(required_copy_bytes)?
                ],
            )
            .map_err(CoreError::Ledger)?;
        Ok(())
    }

    pub fn begin_batch_run(
        &mut self,
        id: &str,
        source_id: &SourceId,
        started_at: &str,
        required_copy_bytes: u64,
        preferences: FrozenPreferences,
    ) -> Result<(), CoreError> {
        self.connection
            .execute(
                r#"INSERT INTO backup_runs(
                     id, source_id, started_at, outcome, required_copy_bytes, batch_phase,
                     frozen_automatic_backup, frozen_m4a_conversion, frozen_automatic_trash
                   ) VALUES (?1, ?2, ?3, 'running', ?4, 'inventory', ?5, ?6, ?7)"#,
                params![
                    id,
                    source_id.as_str(),
                    started_at,
                    to_i64(required_copy_bytes)?,
                    preferences.automatic_backup,
                    preferences.m4a_conversion,
                    preferences.automatic_trash,
                ],
            )
            .map_err(CoreError::Ledger)?;
        Ok(())
    }

    pub fn advance_batch_phase(&mut self, id: &str, next: BatchPhase) -> Result<(), CoreError> {
        let predecessor = next.predecessor().ok_or(CoreError::InvalidRequest)?;
        let changed = self
            .connection
            .execute(
                r#"UPDATE backup_runs SET batch_phase = ?1
                   WHERE id = ?2 AND outcome = 'running' AND batch_phase = ?3"#,
                params![next.storage_name(), id, predecessor.storage_name()],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::InvalidRequest);
        }
        Ok(())
    }

    pub fn batch_run_evidence(&self, id: &str) -> Result<Option<BatchRunEvidence>, CoreError> {
        let row = self
            .connection
            .query_row(
                r#"SELECT source_id, batch_phase, frozen_automatic_backup, frozen_m4a_conversion,
                          frozen_automatic_trash, m4a_profile_id
                   FROM backup_runs WHERE id = ?1"#,
                [id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                        row.get::<_, bool>(3)?,
                        row.get::<_, bool>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(CoreError::Ledger)?;
        row.map(
            |(
                source_id,
                phase,
                automatic_backup,
                m4a_conversion,
                automatic_trash,
                m4a_profile_id,
            )| {
                Ok(BatchRunEvidence {
                    source_id: source_id
                        .map(|source_id| {
                            SourceId::parse(&source_id).map_err(|_| CoreError::LedgerCorrupt)
                        })
                        .transpose()?,
                    phase: BatchPhase::parse(&phase)?,
                    frozen_preferences: FrozenPreferences {
                        automatic_backup,
                        m4a_conversion,
                        automatic_trash,
                    },
                    m4a_profile_id,
                })
            },
        )
        .transpose()
    }

    pub fn begin_conversion_cohort(
        &mut self,
        backup_run_id: &str,
        recording_ids: &[String],
        profile_id: &str,
    ) -> Result<(), CoreError> {
        if recording_ids.is_empty()
            || profile_id != crate::batch::M4A_PROFILE_ID
            || recording_ids
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != recording_ids.len()
        {
            return Err(CoreError::InvalidRequest);
        }
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        let changed = transaction
            .execute(
                r#"UPDATE backup_runs
                   SET batch_phase = 'converting', m4a_profile_id = ?1
                   WHERE id = ?2 AND outcome = 'running' AND batch_phase = 'copies_verified'
                     AND m4a_profile_id IS NULL"#,
                params![profile_id, backup_run_id],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::InvalidRequest);
        }
        for recording_id in recording_ids {
            let inserted = transaction
                .execute(
                    r#"INSERT INTO conversion_cohort_items(
                         backup_run_id, recording_id, source_id, profile_id, status
                       )
                       SELECT ?1, id, source_id, ?3, 'pending'
                       FROM recordings
                       WHERE id = ?2 AND source_id = (
                         SELECT source_id FROM backup_runs WHERE id = ?1
                       )"#,
                    params![backup_run_id, recording_id, profile_id],
                )
                .map_err(CoreError::Ledger)?;
            if inserted != 1 {
                return Err(CoreError::InvalidRequest);
            }
        }
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn mark_conversion_item_verified(
        &mut self,
        backup_run_id: &str,
        recording_id: &str,
    ) -> Result<(), CoreError> {
        let changed = self
            .connection
            .execute(
                r#"UPDATE conversion_cohort_items
                   SET status = 'verified_m4a', error_code = NULL
                   WHERE backup_run_id = ?1 AND recording_id = ?2 AND status = 'pending'
                     AND profile_id = (
                       SELECT m4a_profile_id FROM backup_runs
                       WHERE id = ?1 AND batch_phase = 'converting'
                     )
                     AND EXISTS (
                       SELECT 1 FROM recordings
                       WHERE id = ?2 AND artifact_format = 'm4a'
                         AND conversion_status = 'complete'
                         AND artifact_codec = 'aac'
                         AND artifact_sample_rate_hz > 0
                         AND artifact_channel_count > 0
                         AND artifact_valid_frames > 0
                         AND artifact_duration_micros > 0
                     )"#,
                params![backup_run_id, recording_id],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::InvalidRequest);
        }
        Ok(())
    }

    pub fn commit_m4a_barrier(&mut self, backup_run_id: &str) -> Result<(), CoreError> {
        let changed = self
            .connection
            .execute(
                r#"UPDATE backup_runs SET batch_phase = 'm4a_cohort_verified'
                   WHERE id = ?1 AND outcome = 'running' AND batch_phase = 'converting'
                     AND m4a_profile_id IS NOT NULL
                     AND EXISTS (
                       SELECT 1 FROM conversion_cohort_items WHERE backup_run_id = ?1
                     )
                     AND NOT EXISTS (
                       SELECT 1 FROM conversion_cohort_items
                       WHERE backup_run_id = ?1 AND status != 'verified_m4a'
                     )"#,
                [backup_run_id],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::InvalidRequest);
        }
        Ok(())
    }

    pub fn commit_empty_m4a_barrier(&mut self, backup_run_id: &str) -> Result<(), CoreError> {
        let changed = self
            .connection
            .execute(
                r#"UPDATE backup_runs
                   SET batch_phase = 'm4a_cohort_verified', m4a_profile_id = ?1
                   WHERE id = ?2 AND outcome = 'running' AND batch_phase = 'copies_verified'
                     AND source_id IS NOT NULL
                     AND NOT EXISTS (
                       SELECT 1 FROM recordings
                       WHERE backup_run_id = ?2 AND artifact_format = 'wav'
                     )"#,
                params![crate::batch::M4A_PROFILE_ID, backup_run_id],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::InvalidRequest);
        }
        Ok(())
    }

    pub fn mark_interrupted_runs(&mut self, finished_at: &str) -> Result<usize, CoreError> {
        self.connection
            .execute(
                r#"UPDATE backup_runs
                   SET outcome = 'interrupted', finished_at = ?1, error_code = 'interrupted'
                   WHERE outcome = 'running'"#,
                [finished_at],
            )
            .map_err(CoreError::Ledger)
    }

    pub fn backup_run_outcome(&self, id: &str) -> Result<Option<String>, CoreError> {
        self.connection
            .query_row(
                "SELECT outcome FROM backup_runs WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(CoreError::Ledger)
    }

    pub fn commit_verified_recording(
        &mut self,
        recording: &VerifiedRecording,
    ) -> Result<String, CoreError> {
        validate_verified_recording(recording)?;
        let source_relative_path = path_text(&recording.source_relative_path)?;
        let destination_relative_path = path_text(&recording.artifact.relative_path)?;
        let retired_session_relative_path = recording
            .retired_session_relative_path
            .as_deref()
            .map(path_text)
            .transpose()?;
        let audio = recording.artifact.audio.as_ref();
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        let existing = transaction
            .query_row(
                r#"SELECT id, destination_relative_path, destination_size, destination_sha256,
                          artifact_format, artifact_codec, artifact_sample_rate_hz,
                          artifact_channel_count, artifact_valid_frames,
                          artifact_duration_micros, conversion_status,
                          conversion_error_code, retirement_status,
                          retired_session_relative_path
                   FROM recordings
                   WHERE source_id = ?1 AND source_relative_path = ?2
                     AND source_size = ?3 AND source_mtime_ns = ?4 AND source_sha256 = ?5"#,
                params![
                    recording.source_id.as_str(),
                    source_relative_path,
                    to_i64(recording.source_size)?,
                    recording.source_mtime_ns.to_string(),
                    recording.source_sha256,
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                        row.get::<_, Option<i64>>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, Option<String>>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, Option<String>>(13)?,
                    ))
                },
            )
            .optional()
            .map_err(CoreError::Ledger)?;
        let id = if let Some((
            id,
            existing_path,
            existing_size,
            existing_hash,
            existing_format,
            existing_codec,
            existing_sample_rate,
            existing_channels,
            existing_valid_frames,
            existing_duration,
            existing_conversion,
            existing_conversion_error,
            existing_retirement,
            existing_retired_session,
        )) = existing
        {
            if existing_path != destination_relative_path
                || optional_u64(Some(existing_size))? != Some(recording.artifact.byte_count)
                || existing_hash != recording.artifact.sha256
                || parse_output_format(&existing_format)? != recording.artifact.format
                || existing_codec.as_deref() != audio.map(|value| value.codec.as_str())
                || optional_u32(existing_sample_rate)? != audio.map(|value| value.sample_rate_hz)
                || optional_u16(existing_channels)? != audio.map(|value| value.channel_count)
                || optional_u64(existing_valid_frames)? != audio.map(|value| value.valid_frames)
                || optional_u64(existing_duration)? != audio.map(|value| value.duration_micros)
                || parse_conversion_status(&existing_conversion)? != recording.conversion_status
                || existing_conversion_error != recording.conversion_error_code
                || parse_retirement_status(&existing_retirement)? != recording.retirement_status
                || existing_retired_session.as_deref() != retired_session_relative_path
            {
                return Err(CoreError::LedgerCorrupt);
            }
            transaction
                .execute(
                    r#"UPDATE recordings
                       SET verified_at = ?1, backup_run_id = ?2
                       WHERE id = ?3"#,
                    params![recording.verified_at, recording.backup_run_id, id],
                )
                .map_err(CoreError::Ledger)?;
            id
        } else {
            transaction
                .execute(
                    r#"INSERT INTO recordings(
                         id, source_id, source_relative_path, source_size, source_mtime_ns,
                         source_sha256, destination_relative_path, destination_size,
                         destination_sha256, verified_at, backup_run_id, artifact_format,
                         artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
                         artifact_valid_frames, artifact_duration_micros, conversion_status,
                         conversion_error_code, retirement_status, retired_session_relative_path
                       ) VALUES (
                         ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                         ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
                       )"#,
                    params![
                        recording.id,
                        recording.source_id.as_str(),
                        source_relative_path,
                        to_i64(recording.source_size)?,
                        recording.source_mtime_ns.to_string(),
                        recording.source_sha256,
                        destination_relative_path,
                        to_i64(recording.artifact.byte_count)?,
                        recording.artifact.sha256,
                        recording.verified_at,
                        recording.backup_run_id,
                        output_format_name(recording.artifact.format),
                        audio.map(|value| value.codec.as_str()),
                        audio.map(|value| i64::from(value.sample_rate_hz)),
                        audio.map(|value| i64::from(value.channel_count)),
                        optional_i64(audio.map(|value| value.valid_frames))?,
                        optional_i64(audio.map(|value| value.duration_micros))?,
                        conversion_status_name(recording.conversion_status),
                        recording.conversion_error_code,
                        retirement_status_name(recording.retirement_status),
                        retired_session_relative_path,
                    ],
                )
                .map_err(CoreError::Ledger)?;
            recording.id.clone()
        };
        transaction.commit().map_err(CoreError::Ledger)?;
        Ok(id)
    }

    pub fn verified_recording(&self, id: &str) -> Result<Option<VerifiedRecording>, CoreError> {
        self.connection
            .query_row(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, destination_relative_path, destination_size,
                          destination_sha256, verified_at, backup_run_id, artifact_format,
                          artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
                          artifact_valid_frames, artifact_duration_micros, conversion_status,
                          conversion_error_code, retirement_status,
                          retired_session_relative_path
                   FROM recordings WHERE id = ?1"#,
                [id],
                row_to_verified_recording,
            )
            .optional()
            .map_err(CoreError::Ledger)
    }

    pub fn verified_recordings(&self) -> Result<Vec<VerifiedRecording>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, destination_relative_path, destination_size,
                          destination_sha256, verified_at, backup_run_id, artifact_format,
                          artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
                          artifact_valid_frames, artifact_duration_micros, conversion_status,
                          conversion_error_code, retirement_status,
                          retired_session_relative_path
                   FROM recordings
                   ORDER BY destination_relative_path, id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([], row_to_verified_recording)
            .map_err(CoreError::Ledger)?;
        rows.map(|row| {
            let recording = row.map_err(CoreError::Ledger)?;
            validate_verified_recording(&recording).map_err(|_| CoreError::LedgerCorrupt)?;
            Ok(recording)
        })
        .collect()
    }

    pub fn verified_recordings_for_backup_run(
        &self,
        source_id: &SourceId,
        backup_run_id: &str,
    ) -> Result<Vec<VerifiedRecording>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, destination_relative_path, destination_size,
                          destination_sha256, verified_at, backup_run_id, artifact_format,
                          artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
                          artifact_valid_frames, artifact_duration_micros, conversion_status,
                          conversion_error_code, retirement_status,
                          retired_session_relative_path
                   FROM recordings
                   WHERE source_id = ?1 AND backup_run_id = ?2
                   ORDER BY source_relative_path, id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map(
                params![source_id.as_str(), backup_run_id],
                row_to_verified_recording,
            )
            .map_err(CoreError::Ledger)?;
        rows.map(|row| {
            let recording = row.map_err(CoreError::Ledger)?;
            validate_verified_recording(&recording).map_err(|_| CoreError::LedgerCorrupt)?;
            Ok(recording)
        })
        .collect()
    }

    pub fn relocate_verified_artifact(
        &mut self,
        recording_id: &str,
        from_relative_path: &Path,
        to_relative_path: &Path,
        byte_count: u64,
        sha256: &str,
    ) -> Result<(), CoreError> {
        if recording_id.is_empty()
            || from_relative_path == to_relative_path
            || !crate::filesystem::is_safe_relative_path(from_relative_path)
            || !crate::filesystem::is_safe_relative_path(to_relative_path)
            || byte_count == 0
            || sha256.len() != 64
            || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(CoreError::InvalidRequest);
        }
        let changed = self
            .connection
            .execute(
                r#"UPDATE recordings
                   SET destination_relative_path = ?1
                   WHERE id = ?2 AND destination_relative_path = ?3
                     AND destination_size = ?4 AND destination_sha256 = ?5"#,
                params![
                    path_text(to_relative_path)?,
                    recording_id,
                    path_text(from_relative_path)?,
                    to_i64(byte_count)?,
                    sha256,
                ],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::LedgerCorrupt);
        }
        Ok(())
    }

    pub fn verified_recording_for_source(
        &self,
        source_id: &SourceId,
        source_relative_path: &Path,
        source_size: u64,
        source_mtime_ns: i128,
        source_sha256: &str,
    ) -> Result<Option<VerifiedRecording>, CoreError> {
        if !crate::filesystem::is_safe_relative_path(source_relative_path) {
            return Err(CoreError::InvalidRequest);
        }
        self.connection
            .query_row(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, destination_relative_path, destination_size,
                          destination_sha256, verified_at, backup_run_id, artifact_format,
                          artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
                          artifact_valid_frames, artifact_duration_micros, conversion_status,
                          conversion_error_code, retirement_status,
                          retired_session_relative_path
                   FROM recordings
                   WHERE source_id = ?1 AND source_relative_path = ?2
                     AND source_size = ?3 AND source_mtime_ns = ?4
                     AND source_sha256 = ?5"#,
                params![
                    source_id.as_str(),
                    path_text(source_relative_path)?,
                    to_i64(source_size)?,
                    source_mtime_ns.to_string(),
                    source_sha256,
                ],
                row_to_verified_recording,
            )
            .optional()
            .map_err(CoreError::Ledger)
    }

    pub fn replace_verified_artifact(
        &mut self,
        recording: &VerifiedRecording,
    ) -> Result<(), CoreError> {
        self.replace_verified_artifact_evidence(recording, None)
    }

    pub fn replace_verified_recording_from_fresh_copy(
        &mut self,
        recording: &VerifiedRecording,
    ) -> Result<String, CoreError> {
        validate_verified_recording(recording)?;
        if recording.artifact.format != OutputFormat::Wav
            || recording.artifact.audio.is_some()
            || recording.conversion_status != ConversionStatus::NotRequired
            || recording.conversion_error_code.is_some()
            || recording.retirement_status != RetirementStatus::Present
            || recording.retired_session_relative_path.is_some()
        {
            return Err(CoreError::InvalidRequest);
        }

        let source_relative_path = path_text(&recording.source_relative_path)?;
        let destination_relative_path = path_text(&recording.artifact.relative_path)?;
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        let id = transaction
            .query_row(
                r#"SELECT id FROM recordings
                   WHERE source_id = ?1 AND source_relative_path = ?2
                     AND source_size = ?3 AND source_mtime_ns = ?4
                     AND source_sha256 = ?5"#,
                params![
                    recording.source_id.as_str(),
                    source_relative_path,
                    to_i64(recording.source_size)?,
                    recording.source_mtime_ns.to_string(),
                    recording.source_sha256,
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(CoreError::Ledger)?
            .ok_or(CoreError::LedgerCorrupt)?;
        let changed = transaction
            .execute(
                r#"UPDATE recordings
                   SET destination_relative_path = ?1, destination_size = ?2,
                       destination_sha256 = ?3, verified_at = ?4, backup_run_id = ?5,
                       artifact_format = 'wav', artifact_codec = NULL,
                       artifact_sample_rate_hz = NULL, artifact_channel_count = NULL,
                       artifact_valid_frames = NULL, artifact_duration_micros = NULL,
                       conversion_status = 'not_required', conversion_error_code = NULL,
                       retirement_status = 'present', retired_session_relative_path = NULL,
                       superseded_wav_relative_path = NULL, superseded_wav_size = NULL,
                       superseded_wav_sha256 = NULL, superseded_wav_retirement_status = 'none'
                   WHERE id = ?6 AND source_id = ?7 AND source_relative_path = ?8
                     AND source_size = ?9 AND source_mtime_ns = ?10
                     AND source_sha256 = ?11"#,
                params![
                    destination_relative_path,
                    to_i64(recording.artifact.byte_count)?,
                    recording.artifact.sha256,
                    recording.verified_at,
                    recording.backup_run_id,
                    id,
                    recording.source_id.as_str(),
                    source_relative_path,
                    to_i64(recording.source_size)?,
                    recording.source_mtime_ns.to_string(),
                    recording.source_sha256,
                ],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::LedgerCorrupt);
        }
        transaction.commit().map_err(CoreError::Ledger)?;
        Ok(id)
    }

    pub fn replace_verified_artifact_with_superseded_wav(
        &mut self,
        recording: &VerifiedRecording,
        superseded_relative_path: &Path,
        superseded_size: u64,
        superseded_sha256: &str,
    ) -> Result<(), CoreError> {
        if !crate::filesystem::is_safe_relative_path(superseded_relative_path)
            || superseded_size == 0
            || superseded_sha256.len() != 64
            || !superseded_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(CoreError::InvalidRequest);
        }
        self.replace_verified_artifact_evidence(
            recording,
            Some((superseded_relative_path, superseded_size, superseded_sha256)),
        )
    }

    fn replace_verified_artifact_evidence(
        &mut self,
        recording: &VerifiedRecording,
        superseded: Option<(&Path, u64, &str)>,
    ) -> Result<(), CoreError> {
        validate_verified_recording(recording)?;
        let destination_relative_path = path_text(&recording.artifact.relative_path)?;
        let audio = recording
            .artifact
            .audio
            .as_ref()
            .ok_or(CoreError::InvalidRequest)?;
        let superseded_path = superseded.map(|value| path_text(value.0)).transpose()?;
        let superseded_size = superseded.map(|value| to_i64(value.1)).transpose()?;
        let superseded_sha256 = superseded.map(|value| value.2);
        let changed = self
            .connection
            .execute(
                r#"UPDATE recordings
                   SET destination_relative_path = ?1, destination_size = ?2,
                       destination_sha256 = ?3, artifact_format = ?4,
                       artifact_codec = ?5, artifact_sample_rate_hz = ?6,
                       artifact_channel_count = ?7, artifact_valid_frames = ?8,
                       artifact_duration_micros = ?9, conversion_status = ?10,
                       conversion_error_code = ?11, verified_at = ?12,
                       backup_run_id = ?13,
                       superseded_wav_relative_path = COALESCE(?14, superseded_wav_relative_path),
                       superseded_wav_size = COALESCE(?15, superseded_wav_size),
                       superseded_wav_sha256 = COALESCE(?16, superseded_wav_sha256),
                       superseded_wav_retirement_status = CASE
                           WHEN ?14 IS NOT NULL THEN 'pending'
                           ELSE superseded_wav_retirement_status
                       END
                   WHERE id = ?17 AND source_id = ?18 AND source_relative_path = ?19
                     AND source_size = ?20 AND source_mtime_ns = ?21
                     AND source_sha256 = ?22"#,
                params![
                    destination_relative_path,
                    to_i64(recording.artifact.byte_count)?,
                    recording.artifact.sha256,
                    output_format_name(recording.artifact.format),
                    audio.codec,
                    i64::from(audio.sample_rate_hz),
                    i64::from(audio.channel_count),
                    to_i64(audio.valid_frames)?,
                    to_i64(audio.duration_micros)?,
                    conversion_status_name(recording.conversion_status),
                    recording.conversion_error_code,
                    recording.verified_at,
                    recording.backup_run_id,
                    superseded_path,
                    superseded_size,
                    superseded_sha256,
                    recording.id,
                    recording.source_id.as_str(),
                    path_text(&recording.source_relative_path)?,
                    to_i64(recording.source_size)?,
                    recording.source_mtime_ns.to_string(),
                    recording.source_sha256,
                ],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::LedgerCorrupt);
        }
        Ok(())
    }

    pub fn superseded_wav_evidence(
        &self,
        recording_id: &str,
    ) -> Result<Option<(PathBuf, u64, String)>, CoreError> {
        let row = self
            .connection
            .query_row(
                r#"SELECT superseded_wav_relative_path, superseded_wav_size,
                          superseded_wav_sha256
                   FROM recordings WHERE id = ?1"#,
                [recording_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(CoreError::Ledger)?;
        let Some((path, size, sha256)) = row else {
            return Ok(None);
        };
        match (path, size, sha256) {
            (Some(path), Some(size), Some(sha256)) => Ok(Some((
                PathBuf::from(path),
                u64::try_from(size).map_err(|_| CoreError::LedgerCorrupt)?,
                sha256,
            ))),
            (None, None, None) => Ok(None),
            _ => Err(CoreError::LedgerCorrupt),
        }
    }

    pub fn pending_superseded_wavs(&self) -> Result<Vec<SupersededWavEvidence>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT id, source_id, superseded_wav_relative_path,
                          superseded_wav_size, superseded_wav_sha256
                   FROM recordings
                   WHERE superseded_wav_retirement_status = 'pending'
                   ORDER BY superseded_wav_relative_path, id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(CoreError::Ledger)?;
        rows.map(|row| {
            let (recording_id, source_id, path, size, sha256) = row.map_err(CoreError::Ledger)?;
            let (Some(path), Some(size), Some(sha256)) = (path, size, sha256) else {
                return Err(CoreError::LedgerCorrupt);
            };
            let relative_path = PathBuf::from(path);
            let byte_count = u64::try_from(size).map_err(|_| CoreError::LedgerCorrupt)?;
            if !crate::filesystem::is_safe_relative_path(&relative_path)
                || byte_count == 0
                || sha256.len() != 64
            {
                return Err(CoreError::LedgerCorrupt);
            }
            Ok(SupersededWavEvidence {
                recording_id,
                source_id: SourceId::parse(&source_id).map_err(|_| CoreError::LedgerCorrupt)?,
                relative_path,
                byte_count,
                sha256,
            })
        })
        .collect()
    }

    pub fn mark_superseded_wav_moved_to_trash(
        &mut self,
        recording_id: &str,
    ) -> Result<(), CoreError> {
        self.mark_superseded_wav_retired(recording_id, "moved_to_trash")
    }

    pub fn mark_superseded_wav_absent_after_conversion(
        &mut self,
        recording_id: &str,
    ) -> Result<(), CoreError> {
        self.mark_superseded_wav_retired(recording_id, "absent_after_conversion")
    }

    fn mark_superseded_wav_retired(
        &mut self,
        recording_id: &str,
        status: &str,
    ) -> Result<(), CoreError> {
        let changed = self
            .connection
            .execute(
                r#"UPDATE recordings
                   SET superseded_wav_retirement_status = ?2
                   WHERE id = ?1 AND superseded_wav_retirement_status = 'pending'
                     AND superseded_wav_relative_path IS NOT NULL
                     AND superseded_wav_size IS NOT NULL
                     AND superseded_wav_sha256 IS NOT NULL"#,
                params![recording_id, status],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::InvalidRequest);
        }
        Ok(())
    }

    pub fn verified_recording_count(&self) -> Result<u64, CoreError> {
        let count = self
            .connection
            .query_row("SELECT COUNT(*) FROM recordings", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(CoreError::Ledger)?;
        u64::try_from(count).map_err(|_| CoreError::LedgerCorrupt)
    }

    pub fn commit_verified_additional_file(
        &mut self,
        file: &VerifiedAdditionalFile,
    ) -> Result<String, CoreError> {
        validate_verified_additional_file(file)?;
        if let Some(existing) = self.verified_additional_file(&file.id)?
            && !additional_file_evidence_matches(&existing, file)
        {
            return Err(CoreError::LedgerCorrupt);
        }
        if let Some(existing) = self.verified_additional_file_for_source(
            &file.source_id,
            &file.source_relative_path,
            file.source_size,
            file.source_mtime_ns,
            &file.source_sha256,
        )? {
            if !additional_file_evidence_matches(&existing, file) {
                return Err(CoreError::LedgerCorrupt);
            }
            self.connection
                .execute(
                    "UPDATE additional_files SET backup_run_id = ?1 WHERE id = ?2",
                    params![file.backup_run_id, existing.id],
                )
                .map_err(CoreError::Ledger)?;
            return Ok(existing.id);
        }
        self.connection
            .execute(
                r#"INSERT INTO additional_files(
                     id, source_id, source_relative_path, source_size, source_mtime_ns,
                     source_sha256, artifact_relative_path, artifact_size, artifact_sha256,
                     classification, backup_run_id
                   ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
                params![
                    file.id,
                    file.source_id.as_str(),
                    path_text(&file.source_relative_path)?,
                    to_i64(file.source_size)?,
                    file.source_mtime_ns.to_string(),
                    file.source_sha256,
                    path_text(&file.artifact_relative_path)?,
                    to_i64(file.artifact_size)?,
                    file.artifact_sha256,
                    file.classification.storage_name(),
                    file.backup_run_id,
                ],
            )
            .map_err(CoreError::Ledger)?;
        Ok(file.id.clone())
    }

    pub fn verified_additional_file(
        &self,
        id: &str,
    ) -> Result<Option<VerifiedAdditionalFile>, CoreError> {
        let row = self
            .connection
            .query_row(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, artifact_relative_path, artifact_size,
                          artifact_sha256, classification, backup_run_id
                   FROM additional_files WHERE id = ?1"#,
                [id],
                row_to_verified_additional_file,
            )
            .optional()
            .map_err(CoreError::Ledger)?;
        if let Some(file) = &row {
            validate_verified_additional_file(file).map_err(|_| CoreError::LedgerCorrupt)?;
        }
        Ok(row)
    }

    pub fn verified_additional_files(&self) -> Result<Vec<VerifiedAdditionalFile>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, artifact_relative_path, artifact_size,
                          artifact_sha256, classification, backup_run_id
                   FROM additional_files
                   ORDER BY artifact_relative_path, id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([], row_to_verified_additional_file)
            .map_err(CoreError::Ledger)?;
        rows.map(|row| {
            let file = row.map_err(CoreError::Ledger)?;
            validate_verified_additional_file(&file).map_err(|_| CoreError::LedgerCorrupt)?;
            Ok(file)
        })
        .collect()
    }

    pub fn relocate_verified_additional_artifact(
        &mut self,
        file_id: &str,
        from_relative_path: &Path,
        to_relative_path: &Path,
        byte_count: u64,
        sha256: &str,
    ) -> Result<(), CoreError> {
        if file_id.is_empty()
            || from_relative_path == to_relative_path
            || !crate::filesystem::is_safe_additional_relative_path(from_relative_path)
            || !crate::filesystem::is_safe_additional_relative_path(to_relative_path)
            || byte_count == 0
            || sha256.len() != 64
            || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(CoreError::InvalidRequest);
        }
        let changed = self
            .connection
            .execute(
                r#"UPDATE additional_files
                   SET artifact_relative_path = ?1
                   WHERE id = ?2 AND artifact_relative_path = ?3
                     AND artifact_size = ?4 AND artifact_sha256 = ?5"#,
                params![
                    path_text(to_relative_path)?,
                    file_id,
                    path_text(from_relative_path)?,
                    to_i64(byte_count)?,
                    sha256,
                ],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::LedgerCorrupt);
        }
        Ok(())
    }

    pub fn verified_additional_file_for_source(
        &self,
        source_id: &SourceId,
        source_relative_path: &Path,
        source_size: u64,
        source_mtime_ns: i128,
        source_sha256: &str,
    ) -> Result<Option<VerifiedAdditionalFile>, CoreError> {
        if !crate::filesystem::is_safe_additional_relative_path(source_relative_path) {
            return Err(CoreError::InvalidRequest);
        }
        let row = self
            .connection
            .query_row(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, artifact_relative_path, artifact_size,
                          artifact_sha256, classification, backup_run_id
                   FROM additional_files
                   WHERE source_id = ?1 AND source_relative_path = ?2
                     AND source_size = ?3 AND source_mtime_ns = ?4 AND source_sha256 = ?5"#,
                params![
                    source_id.as_str(),
                    path_text(source_relative_path)?,
                    to_i64(source_size)?,
                    source_mtime_ns.to_string(),
                    source_sha256,
                ],
                row_to_verified_additional_file,
            )
            .optional()
            .map_err(CoreError::Ledger)?;
        if let Some(file) = &row {
            validate_verified_additional_file(file).map_err(|_| CoreError::LedgerCorrupt)?;
        }
        Ok(row)
    }

    pub fn verified_additional_files_for_backup_run(
        &self,
        source_id: &SourceId,
        backup_run_id: &str,
    ) -> Result<Vec<VerifiedAdditionalFile>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, artifact_relative_path, artifact_size,
                          artifact_sha256, classification, backup_run_id
                   FROM additional_files
                   WHERE source_id = ?1 AND backup_run_id = ?2
                   ORDER BY source_relative_path, id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map(
                params![source_id.as_str(), backup_run_id],
                row_to_verified_additional_file,
            )
            .map_err(CoreError::Ledger)?;
        rows.map(|row| {
            let file = row.map_err(CoreError::Ledger)?;
            validate_verified_additional_file(&file).map_err(|_| CoreError::LedgerCorrupt)?;
            Ok(file)
        })
        .collect()
    }

    pub fn historical_wav_recordings(&self) -> Result<Vec<VerifiedRecording>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, destination_relative_path, destination_size,
                          destination_sha256, verified_at, backup_run_id, artifact_format,
                          artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
                          artifact_valid_frames, artifact_duration_micros, conversion_status,
                          conversion_error_code, retirement_status,
                          retired_session_relative_path
                   FROM recordings
                   WHERE artifact_format = 'wav'
                   ORDER BY destination_relative_path, id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([], row_to_verified_recording)
            .map_err(CoreError::Ledger)?;
        rows.map(|row| row.map_err(CoreError::Ledger)).collect()
    }

    pub fn historical_wav_recordings_for_source(
        &self,
        source_id: &SourceId,
    ) -> Result<Vec<VerifiedRecording>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, destination_relative_path, destination_size,
                          destination_sha256, verified_at, backup_run_id, artifact_format,
                          artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
                          artifact_valid_frames, artifact_duration_micros, conversion_status,
                          conversion_error_code, retirement_status,
                          retired_session_relative_path
                   FROM recordings
                   WHERE source_id = ?1 AND artifact_format = 'wav'
                   ORDER BY destination_relative_path, id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([source_id.as_str()], row_to_verified_recording)
            .map_err(CoreError::Ledger)?;
        rows.map(|row| row.map_err(CoreError::Ledger)).collect()
    }

    pub fn legacy_retired_recordings(
        &self,
        source_id: &SourceId,
    ) -> Result<Vec<LegacyRetiredRecording>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT id, source_relative_path
                   FROM recordings
                   WHERE source_id = ?1 AND retirement_status = 'legacy_deleted'
                   ORDER BY source_relative_path"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([source_id.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(CoreError::Ledger)?;
        rows.map(|row| {
            let (recording_id, source_relative_path) = row.map_err(CoreError::Ledger)?;
            let source_relative_path = PathBuf::from(source_relative_path);
            if !crate::filesystem::is_safe_relative_path(&source_relative_path) {
                return Err(CoreError::LedgerCorrupt);
            }
            Ok(LegacyRetiredRecording {
                recording_id,
                source_relative_path,
            })
        })
        .collect()
    }

    pub fn record_legacy_session_moved_to_trash(
        &mut self,
        recording_ids: &[String],
        session_relative_path: &Path,
        moved_at: &str,
    ) -> Result<(), CoreError> {
        if recording_ids.is_empty()
            || !crate::filesystem::is_safe_relative_path(session_relative_path)
        {
            return Err(CoreError::InvalidRequest);
        }
        let session = path_text(session_relative_path)?;
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        for recording_id in recording_ids {
            let changed = transaction
                .execute(
                    r#"UPDATE recordings
                       SET source_deleted_at = COALESCE(source_deleted_at, ?1),
                           deletion_error_code = NULL,
                           retirement_status = 'moved_to_trash',
                           retired_session_relative_path = ?2
                       WHERE id = ?3 AND retirement_status = 'legacy_deleted'"#,
                    params![moved_at, session, recording_id],
                )
                .map_err(CoreError::Ledger)?;
            if changed != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
        }
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn finish_backup_run(
        &mut self,
        id: &str,
        finished_at: &str,
        outcome: &str,
        error_code: Option<&str>,
    ) -> Result<(), CoreError> {
        let changed = self
            .connection
            .execute(
                r#"UPDATE backup_runs
                   SET finished_at = ?1, outcome = ?2, error_code = ?3
                   WHERE id = ?4 AND outcome = 'running'"#,
                params![finished_at, outcome, error_code, id],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::InvalidRequest);
        }
        Ok(())
    }

    pub fn begin_deletion_run(
        &mut self,
        id: &str,
        source_id: &SourceId,
        started_at: &str,
        items: &[PendingDeletionItem],
    ) -> Result<(), CoreError> {
        self.begin_deletion_run_with_additional(id, source_id, started_at, items, &[])
    }

    pub fn begin_deletion_run_with_additional(
        &mut self,
        id: &str,
        source_id: &SourceId,
        started_at: &str,
        items: &[PendingDeletionItem],
        additional_items: &[PendingAdditionalDeletionItem],
    ) -> Result<(), CoreError> {
        let proposed_bytes = items.iter().try_fold(0_u64, |total, item| {
            total
                .checked_add(item.source_size)
                .ok_or(CoreError::InvalidRequest)
        })?;
        let proposed_bytes = additional_items
            .iter()
            .try_fold(proposed_bytes, |total, item| {
                total
                    .checked_add(item.source_size)
                    .ok_or(CoreError::InvalidRequest)
            })?;
        let proposed_count = items
            .len()
            .checked_add(additional_items.len())
            .ok_or(CoreError::InvalidRequest)?;
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        transaction
            .execute(
                r#"INSERT INTO deletion_runs(
                     id, source_id, started_at, outcome, proposed_file_count, proposed_bytes
                   ) VALUES (?1, ?2, ?3, 'running', ?4, ?5)"#,
                params![
                    id,
                    source_id.as_str(),
                    started_at,
                    to_i64(u64::try_from(proposed_count).map_err(|_| CoreError::InvalidRequest)?)?,
                    to_i64(proposed_bytes)?,
                ],
            )
            .map_err(CoreError::Ledger)?;
        for item in items {
            let inserted = transaction
                .execute(
                    r#"INSERT INTO deletion_items(
                         deletion_run_id, recording_id, source_id, outcome
                       )
                       SELECT ?1, id, source_id, 'pending' FROM recordings
                       WHERE id = ?2 AND source_id = ?3"#,
                    params![id, item.recording_id, source_id.as_str()],
                )
                .map_err(CoreError::Ledger)?;
            if inserted != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
            let changed = transaction
                .execute(
                    r#"UPDATE recordings
                       SET retirement_status = 'trash_pending', deletion_error_code = NULL
                       WHERE id = ?1 AND source_deleted_at IS NULL
                         AND retirement_status IN ('present', 'failed')"#,
                    [&item.recording_id],
                )
                .map_err(CoreError::Ledger)?;
            if changed != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
        }
        for item in additional_items {
            let inserted = transaction
                .execute(
                    r#"INSERT INTO additional_deletion_items(
                         deletion_run_id, additional_file_id, source_id, outcome
                       )
                       SELECT ?1, id, source_id, 'pending' FROM additional_files
                       WHERE id = ?2 AND source_id = ?3"#,
                    params![id, item.additional_file_id, source_id.as_str()],
                )
                .map_err(CoreError::Ledger)?;
            if inserted != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
        }
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn record_deletion_target_success(
        &mut self,
        deletion_run_id: &str,
        recording_ids: &[&str],
        removed_at: &str,
        retired_session_relative_path: Option<&Path>,
    ) -> Result<(), CoreError> {
        self.record_deletion_target_success_with_additional(
            deletion_run_id,
            recording_ids,
            &[],
            removed_at,
            retired_session_relative_path,
        )
    }

    pub fn record_deletion_target_success_with_additional(
        &mut self,
        deletion_run_id: &str,
        recording_ids: &[&str],
        additional_file_ids: &[&str],
        removed_at: &str,
        retired_session_relative_path: Option<&Path>,
    ) -> Result<(), CoreError> {
        if recording_ids.is_empty() && additional_file_ids.is_empty() {
            return Err(CoreError::InvalidRequest);
        }
        let retired_session = retired_session_relative_path
            .map(|path| path.to_str().ok_or(CoreError::InvalidRequest))
            .transpose()?;
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        for recording_id in recording_ids {
            let item_changed = transaction
                .execute(
                    r#"UPDATE deletion_items
                       SET outcome = 'moved_to_trash', removed_at = ?1, error_code = NULL
                       WHERE deletion_run_id = ?2 AND recording_id = ?3 AND outcome = 'pending'"#,
                    params![removed_at, deletion_run_id, recording_id],
                )
                .map_err(CoreError::Ledger)?;
            let recording_changed = transaction
                .execute(
                    r#"UPDATE recordings
                       SET source_deleted_at = ?1, deletion_error_code = NULL,
                           retirement_status = 'moved_to_trash',
                           retired_session_relative_path = ?2
                       WHERE id = ?3 AND source_deleted_at IS NULL
                         AND retirement_status = 'trash_pending'"#,
                    params![removed_at, retired_session, recording_id],
                )
                .map_err(CoreError::Ledger)?;
            if item_changed != 1 || recording_changed != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
        }
        for additional_file_id in additional_file_ids {
            let item_changed = transaction
                .execute(
                    r#"UPDATE additional_deletion_items
                       SET outcome = 'moved_to_trash', removed_at = ?1, error_code = NULL
                       WHERE deletion_run_id = ?2 AND additional_file_id = ?3
                         AND outcome = 'pending'"#,
                    params![removed_at, deletion_run_id, additional_file_id],
                )
                .map_err(CoreError::Ledger)?;
            if item_changed != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
        }
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn record_deletion_target_failure(
        &mut self,
        deletion_run_id: &str,
        recording_ids: &[&str],
        error_code: &str,
    ) -> Result<(), CoreError> {
        self.record_deletion_target_failure_with_additional(
            deletion_run_id,
            recording_ids,
            &[],
            error_code,
        )
    }

    pub fn record_deletion_target_failure_with_additional(
        &mut self,
        deletion_run_id: &str,
        recording_ids: &[&str],
        additional_file_ids: &[&str],
        error_code: &str,
    ) -> Result<(), CoreError> {
        if recording_ids.is_empty() && additional_file_ids.is_empty() {
            return Err(CoreError::InvalidRequest);
        }
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        for recording_id in recording_ids {
            let item_changed = transaction
                .execute(
                    r#"UPDATE deletion_items SET outcome = 'failed', error_code = ?1
                       WHERE deletion_run_id = ?2 AND recording_id = ?3 AND outcome = 'pending'"#,
                    params![error_code, deletion_run_id, recording_id],
                )
                .map_err(CoreError::Ledger)?;
            let recording_changed = transaction
                .execute(
                    r#"UPDATE recordings
                       SET deletion_error_code = ?1, retirement_status = 'failed'
                       WHERE id = ?2 AND retirement_status = 'trash_pending'"#,
                    params![error_code, recording_id],
                )
                .map_err(CoreError::Ledger)?;
            if item_changed != 1 || recording_changed != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
        }
        for additional_file_id in additional_file_ids {
            let item_changed = transaction
                .execute(
                    r#"UPDATE additional_deletion_items
                       SET outcome = 'failed', error_code = ?1
                       WHERE deletion_run_id = ?2 AND additional_file_id = ?3
                         AND outcome = 'pending'"#,
                    params![error_code, deletion_run_id, additional_file_id],
                )
                .map_err(CoreError::Ledger)?;
            if item_changed != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
        }
        transaction
            .execute(
                r#"UPDATE recordings
                   SET retirement_status = 'present'
                   WHERE retirement_status = 'trash_pending'
                     AND id IN (
                       SELECT recording_id FROM deletion_items
                       WHERE deletion_run_id = ?1 AND outcome = 'pending'
                     )"#,
                [deletion_run_id],
            )
            .map_err(CoreError::Ledger)?;
        transaction
            .execute(
                r#"UPDATE deletion_items SET outcome = 'not_attempted'
                   WHERE deletion_run_id = ?1 AND outcome = 'pending'"#,
                [deletion_run_id],
            )
            .map_err(CoreError::Ledger)?;
        transaction
            .execute(
                r#"UPDATE additional_deletion_items SET outcome = 'not_attempted'
                   WHERE deletion_run_id = ?1 AND outcome = 'pending'"#,
                [deletion_run_id],
            )
            .map_err(CoreError::Ledger)?;
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn finish_deletion_run(
        &mut self,
        id: &str,
        finished_at: &str,
        outcome: &str,
        error_code: Option<&str>,
    ) -> Result<(), CoreError> {
        let changed = self
            .connection
            .execute(
                r#"UPDATE deletion_runs SET finished_at = ?1, outcome = ?2, error_code = ?3
                   WHERE id = ?4 AND outcome = 'running'"#,
                params![finished_at, outcome, error_code, id],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::LedgerCorrupt);
        }
        Ok(())
    }

    pub fn additional_deletion_outcomes(
        &self,
        deletion_run_id: &str,
    ) -> Result<Vec<(String, String)>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT additional_file_id, outcome
                   FROM additional_deletion_items
                   WHERE deletion_run_id = ?1
                   ORDER BY additional_file_id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([deletion_run_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(CoreError::Ledger)?;
        rows.map(|row| row.map_err(CoreError::Ledger)).collect()
    }

    pub fn deletion_item_outcomes(&self, id: &str) -> Result<Vec<String>, CoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT outcome FROM deletion_items WHERE deletion_run_id = ?1 ORDER BY rowid")
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([id], |row| row.get::<_, String>(0))
            .map_err(CoreError::Ledger)?;
        rows.map(|row| row.map_err(CoreError::Ledger)).collect()
    }
}

const RULE_COLUMNS: &str = r#"id, name, archive_directory_name, enabled,
    volume_name_glob, filename_prefix, filename_suffix, date_folder_layout, filename_profile,
    device_constraint_profile, preset_kind, preset_revision,
    archive_directory_locked, archived_at, created_at, updated_at"#;

struct StoredRuleRow {
    id: String,
    name: String,
    archive_directory_name: String,
    enabled: bool,
    volume_name_glob: String,
    filename_prefix: String,
    filename_suffix: String,
    date_folder_layout: String,
    filename_profile: String,
    device_constraint_profile: String,
    preset_kind: Option<String>,
    preset_revision: Option<i64>,
    archive_directory_locked: bool,
    archived_at: Option<String>,
    created_at: String,
    updated_at: String,
}

fn row_to_stored_rule(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredRuleRow> {
    Ok(StoredRuleRow {
        id: row.get(0)?,
        name: row.get(1)?,
        archive_directory_name: row.get(2)?,
        enabled: row.get(3)?,
        volume_name_glob: row.get(4)?,
        filename_prefix: row.get(5)?,
        filename_suffix: row.get(6)?,
        date_folder_layout: row.get(7)?,
        filename_profile: row.get(8)?,
        device_constraint_profile: row.get(9)?,
        preset_kind: row.get(10)?,
        preset_revision: row.get(11)?,
        archive_directory_locked: row.get(12)?,
        archived_at: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

fn hydrate_rule(connection: &Connection, stored: StoredRuleRow) -> Result<BackupRule, CoreError> {
    let mut rule = BackupRule {
        id: RuleId::parse(&stored.id).map_err(|_| CoreError::LedgerCorrupt)?,
        name: stored.name,
        archive_directory_name: stored.archive_directory_name,
        enabled: stored.enabled,
        volume_name_glob: stored.volume_name_glob,
        required_path_globs: Vec::new(),
        backup_file_globs: Vec::new(),
        session_directory_globs: Vec::new(),
        filename_prefix: stored.filename_prefix,
        filename_suffix: stored.filename_suffix,
        date_folder_layout: DateFolderLayout::parse_storage(&stored.date_folder_layout)?,
        filename_profile: FilenameProfile::parse_storage(&stored.filename_profile)?,
        device_constraint_profile: DeviceConstraintProfile::parse_storage(
            &stored.device_constraint_profile,
        )?,
        preset_kind: stored.preset_kind,
        preset_revision: optional_u32(stored.preset_revision)?,
        archive_directory_locked: stored.archive_directory_locked,
        archived_at: stored.archived_at,
        created_at: stored.created_at,
        updated_at: stored.updated_at,
    };

    let mut statement = connection
        .prepare(
            r#"SELECT kind, ordinal, pattern FROM backup_rule_patterns
               WHERE rule_id = ?1 ORDER BY kind, ordinal"#,
        )
        .map_err(CoreError::Ledger)?;
    let rows = statement
        .query_map([rule.id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(CoreError::Ledger)?;
    for row in rows {
        let (kind, ordinal, pattern) = row.map_err(CoreError::Ledger)?;
        let patterns = match kind.as_str() {
            "required_path" => &mut rule.required_path_globs,
            "backup_file" => &mut rule.backup_file_globs,
            "session_directory" => &mut rule.session_directory_globs,
            _ => return Err(CoreError::LedgerCorrupt),
        };
        if usize::try_from(ordinal).map_err(|_| CoreError::LedgerCorrupt)? != patterns.len() {
            return Err(CoreError::LedgerCorrupt);
        }
        patterns.push(pattern);
    }

    validate_rule(BackupRuleDraft {
        id: Some(rule.id.clone()),
        name: rule.name.clone(),
        archive_directory_name: rule.archive_directory_name.clone(),
        enabled: rule.enabled,
        volume_name_glob: rule.volume_name_glob.clone(),
        required_path_globs: rule.required_path_globs.clone(),
        backup_file_globs: rule.backup_file_globs.clone(),
        session_directory_globs: rule.session_directory_globs.clone(),
        filename_prefix: rule.filename_prefix.clone(),
        filename_suffix: rule.filename_suffix.clone(),
        date_folder_layout: rule.date_folder_layout,
    })
    .map_err(|_| CoreError::LedgerCorrupt)?;

    Ok(rule)
}

fn replace_rule_patterns(
    transaction: &Transaction<'_>,
    id: &RuleId,
    draft: &BackupRuleDraft,
) -> Result<(), CoreError> {
    transaction
        .execute(
            "DELETE FROM backup_rule_patterns WHERE rule_id = ?1",
            [id.as_str()],
        )
        .map_err(CoreError::Ledger)?;
    for (kind, patterns) in [
        ("required_path", &draft.required_path_globs),
        ("backup_file", &draft.backup_file_globs),
        ("session_directory", &draft.session_directory_globs),
    ] {
        for (ordinal, pattern) in patterns.iter().enumerate() {
            transaction
                .execute(
                    r#"INSERT INTO backup_rule_patterns(rule_id, kind, ordinal, pattern)
                       VALUES (?1, ?2, ?3, ?4)"#,
                    params![
                        id.as_str(),
                        kind,
                        i64::try_from(ordinal).map_err(|_| CoreError::LedgerCorrupt)?,
                        pattern,
                    ],
                )
                .map_err(rule_write_error)?;
        }
    }
    Ok(())
}

fn rule_write_error(error: rusqlite::Error) -> CoreError {
    match &error {
        rusqlite::Error::SqliteFailure(details, _)
            if details.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            CoreError::InvalidRule
        }
        _ => CoreError::Ledger(error),
    }
}

fn row_to_source_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<SourceRecord> {
    let id = row.get::<_, String>(0)?;
    let rule_id = row.get::<_, String>(1)?;
    Ok(SourceRecord {
        id: SourceId::parse(&id).map_err(|_| invalid_text_column(0, "invalid source id"))?,
        rule_id: RuleId::parse(&rule_id)
            .map_err(|_| invalid_text_column(1, "invalid source rule id"))?,
        volume_uuid: row.get(2)?,
        legacy_slot: row.get(3)?,
        display_name: row.get(4)?,
    })
}

fn source_write_error(error: rusqlite::Error) -> CoreError {
    match &error {
        rusqlite::Error::SqliteFailure(details, _)
            if details.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            CoreError::InvalidRequest
        }
        _ => CoreError::Ledger(error),
    }
}

fn invalid_text_column(column: usize, message: &'static str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        )),
    )
}

fn open_validated(path: &Path) -> Result<Connection, CoreError> {
    let connection = Connection::open(path).map_err(CoreError::Ledger)?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(CoreError::Ledger)?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(CoreError::Ledger)?;
    let result: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(CoreError::Ledger)?;
    if result != "ok" {
        return Err(CoreError::LedgerCorrupt);
    }
    Ok(connection)
}

fn has_sqlite_header(path: &Path) -> Result<bool, CoreError> {
    use std::io::Read;

    let mut file = fs::File::open(path).map_err(CoreError::LedgerIo)?;
    let mut header = [0_u8; 16];
    if file.read_exact(&mut header).is_err() {
        return Ok(false);
    }
    Ok(&header == b"SQLite format 3\0")
}

fn migrate(connection: &Connection) -> Result<(), CoreError> {
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(CoreError::Ledger)?;
    connection
        .execute_batch(include_str!("../migrations/0001_initial.sql"))
        .map_err(CoreError::Ledger)?;
    let version_two_applied = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = 2)",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(CoreError::Ledger)?;
    if !version_two_applied {
        connection
            .execute_batch(include_str!(
                "../migrations/0002_artifacts_and_preferences.sql"
            ))
            .map_err(CoreError::Ledger)?;
    }
    let version_three_applied = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = 3)",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(CoreError::Ledger)?;
    if !version_three_applied {
        connection
            .execute_batch(include_str!("../migrations/0003_batch_manifests.sql"))
            .map_err(CoreError::Ledger)?;
    }
    let version_four_applied = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = 4)",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(CoreError::Ledger)?;
    if !version_four_applied {
        connection
            .execute_batch(include_str!(
                "../migrations/0004_durable_superseded_wav_evidence.sql"
            ))
            .map_err(CoreError::Ledger)?;
    }
    let version_five_applied = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = 5)",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(CoreError::Ledger)?;
    if !version_five_applied {
        connection
            .execute_batch(include_str!("../migrations/0005_backup_rules.sql"))
            .map_err(CoreError::Ledger)?;
    }
    let version_six_applied = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = 6)",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(CoreError::Ledger)?;
    if !version_six_applied {
        connection
            .execute_batch(include_str!("../migrations/0006_dynamic_sources.sql"))
            .map_err(CoreError::Ledger)?;
    }
    let version_seven_applied = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = 7)",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(CoreError::Ledger)?;
    if !version_seven_applied {
        connection
            .execute_batch(include_str!(
                "../migrations/0007_rule_date_folder_layout.sql"
            ))
            .map_err(CoreError::Ledger)?;
    }
    Ok(())
}

fn quarantine_existing(path: &Path) -> Result<(), CoreError> {
    let parent = path.parent().ok_or_else(|| CoreError::LedgerCorrupt)?;
    let quarantine = parent.join("quarantine").join(now_timestamp());
    fs::create_dir_all(&quarantine).map_err(CoreError::LedgerIo)?;
    let file_name = path.file_name().ok_or_else(|| CoreError::LedgerCorrupt)?;
    for source in [
        path.to_path_buf(),
        path.with_file_name(format!("{}-wal", file_name.to_string_lossy())),
        path.with_file_name(format!("{}-shm", file_name.to_string_lossy())),
    ] {
        if source.exists() {
            let destination =
                quarantine.join(source.file_name().ok_or_else(|| CoreError::LedgerCorrupt)?);
            fs::rename(source, destination).map_err(CoreError::LedgerIo)?;
        }
    }
    Ok(())
}

fn now_timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
}

fn transmitter_name(transmitter: Transmitter) -> &'static str {
    match transmitter {
        Transmitter::Tx01 => "TX01",
        Transmitter::Tx02 => "TX02",
    }
}

fn parse_transmitter(value: &str) -> Result<Transmitter, CoreError> {
    match value {
        "TX01" => Ok(Transmitter::Tx01),
        "TX02" => Ok(Transmitter::Tx02),
        _ => Err(CoreError::LedgerCorrupt),
    }
}

fn output_format_name(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Wav => "wav",
        OutputFormat::M4a => "m4a",
    }
}

fn parse_output_format(value: &str) -> Result<OutputFormat, CoreError> {
    match value {
        "wav" => Ok(OutputFormat::Wav),
        "m4a" => Ok(OutputFormat::M4a),
        _ => Err(CoreError::LedgerCorrupt),
    }
}

fn conversion_status_name(status: ConversionStatus) -> &'static str {
    match status {
        ConversionStatus::NotRequired => "not_required",
        ConversionStatus::Pending => "pending",
        ConversionStatus::Complete => "complete",
        ConversionStatus::Failed => "failed",
    }
}

fn parse_conversion_status(value: &str) -> Result<ConversionStatus, CoreError> {
    match value {
        "not_required" => Ok(ConversionStatus::NotRequired),
        "pending" => Ok(ConversionStatus::Pending),
        "complete" => Ok(ConversionStatus::Complete),
        "failed" => Ok(ConversionStatus::Failed),
        _ => Err(CoreError::LedgerCorrupt),
    }
}

fn retirement_status_name(status: RetirementStatus) -> &'static str {
    match status {
        RetirementStatus::Present => "present",
        RetirementStatus::TrashPending => "trash_pending",
        RetirementStatus::MovedToTrash => "moved_to_trash",
        RetirementStatus::LegacyDeleted => "legacy_deleted",
        RetirementStatus::Failed => "failed",
    }
}

fn parse_retirement_status(value: &str) -> Result<RetirementStatus, CoreError> {
    match value {
        "present" => Ok(RetirementStatus::Present),
        "trash_pending" => Ok(RetirementStatus::TrashPending),
        "moved_to_trash" => Ok(RetirementStatus::MovedToTrash),
        "legacy_deleted" => Ok(RetirementStatus::LegacyDeleted),
        "failed" => Ok(RetirementStatus::Failed),
        _ => Err(CoreError::LedgerCorrupt),
    }
}

fn validate_verified_recording(recording: &VerifiedRecording) -> Result<(), CoreError> {
    let invalid_audio = recording.artifact.audio.as_ref().is_some_and(|audio| {
        audio.codec.is_empty()
            || audio.sample_rate_hz == 0
            || audio.channel_count == 0
            || audio.valid_frames == 0
            || audio.duration_micros == 0
    });
    if !crate::filesystem::is_safe_relative_path(&recording.source_relative_path)
        || !crate::filesystem::is_safe_relative_path(&recording.artifact.relative_path)
        || recording.source_sha256.len() != 64
        || recording.artifact.sha256.len() != 64
        || recording.artifact.byte_count == 0
        || invalid_audio
        || recording
            .retired_session_relative_path
            .as_deref()
            .is_some_and(|path| !crate::filesystem::is_safe_relative_path(path))
    {
        return Err(CoreError::InvalidRequest);
    }
    match recording.artifact.format {
        OutputFormat::Wav
            if recording.source_size == recording.artifact.byte_count
                && recording.source_sha256 == recording.artifact.sha256
                && recording.artifact.audio.is_none()
                && recording.conversion_status == ConversionStatus::NotRequired =>
        {
            Ok(())
        }
        OutputFormat::M4a
            if recording.artifact.audio.is_some()
                && recording.conversion_status == ConversionStatus::Complete =>
        {
            Ok(())
        }
        OutputFormat::Wav | OutputFormat::M4a => Err(CoreError::InvalidRequest),
    }
}

fn validate_verified_additional_file(file: &VerifiedAdditionalFile) -> Result<(), CoreError> {
    if file.id.is_empty()
        || !crate::filesystem::is_safe_additional_relative_path(&file.source_relative_path)
        || !crate::filesystem::is_safe_additional_relative_path(&file.artifact_relative_path)
        || file.source_size == 0
        || file.source_size != file.artifact_size
        || file.source_sha256.len() != 64
        || file.source_sha256 != file.artifact_sha256
        || file.backup_run_id.is_empty()
    {
        return Err(CoreError::InvalidRequest);
    }
    Ok(())
}

fn additional_file_evidence_matches(
    existing: &VerifiedAdditionalFile,
    candidate: &VerifiedAdditionalFile,
) -> bool {
    existing.source_id == candidate.source_id
        && existing.source_relative_path == candidate.source_relative_path
        && existing.source_size == candidate.source_size
        && existing.source_mtime_ns == candidate.source_mtime_ns
        && existing.source_sha256 == candidate.source_sha256
        && existing.artifact_relative_path == candidate.artifact_relative_path
        && existing.artifact_size == candidate.artifact_size
        && existing.artifact_sha256 == candidate.artifact_sha256
        && existing.classification == candidate.classification
}

fn row_to_verified_recording(row: &rusqlite::Row<'_>) -> rusqlite::Result<VerifiedRecording> {
    let source_size = u64::try_from(row.get::<_, i64>(3)?)
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(3, 0))?;
    let source_mtime_text = row.get::<_, String>(4)?;
    let source_mtime_ns = source_mtime_text.parse::<i128>().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid source mtime",
            )),
        )
    })?;
    let artifact_size = u64::try_from(row.get::<_, i64>(7)?)
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(7, 0))?;
    let format_text = row.get::<_, String>(11)?;
    let format = parse_output_format(&format_text).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            11,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid artifact format",
            )),
        )
    })?;
    let codec = row.get::<_, Option<String>>(12)?;
    let sample_rate = optional_u32_sql(row.get::<_, Option<i64>>(13)?, 13)?;
    let channels = optional_u16_sql(row.get::<_, Option<i64>>(14)?, 14)?;
    let valid_frames = optional_u64_sql(row.get::<_, Option<i64>>(15)?, 15)?;
    let duration_micros = optional_u64_sql(row.get::<_, Option<i64>>(16)?, 16)?;
    let audio = match (codec, sample_rate, channels, valid_frames, duration_micros) {
        (None, None, None, None, None) => None,
        (
            Some(codec),
            Some(sample_rate_hz),
            Some(channel_count),
            Some(valid_frames),
            Some(duration_micros),
        ) => Some(VerifiedAudioProperties {
            codec,
            sample_rate_hz,
            channel_count,
            valid_frames,
            duration_micros,
        }),
        _ => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                12,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "incomplete artifact audio properties",
                )),
            ));
        }
    };
    let conversion_text = row.get::<_, String>(17)?;
    let conversion_status = parse_conversion_status(&conversion_text).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            17,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid conversion status",
            )),
        )
    })?;
    let retirement_text = row.get::<_, String>(19)?;
    let retirement_status = parse_retirement_status(&retirement_text).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            19,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid retirement status",
            )),
        )
    })?;
    Ok(VerifiedRecording {
        id: row.get(0)?,
        source_id: parse_source_id_sql(&row.get::<_, String>(1)?, 1)?,
        source_relative_path: PathBuf::from(row.get::<_, String>(2)?),
        source_size,
        source_mtime_ns,
        source_sha256: row.get(5)?,
        artifact: VerifiedArtifact {
            relative_path: PathBuf::from(row.get::<_, String>(6)?),
            format,
            byte_count: artifact_size,
            sha256: row.get(8)?,
            audio,
        },
        verified_at: row.get(9)?,
        backup_run_id: row.get(10)?,
        conversion_status,
        conversion_error_code: row.get(18)?,
        retirement_status,
        retired_session_relative_path: row.get::<_, Option<String>>(20)?.map(PathBuf::from),
    })
}

fn row_to_verified_additional_file(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<VerifiedAdditionalFile> {
    let source_size = u64::try_from(row.get::<_, i64>(3)?)
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(3, 0))?;
    let source_mtime_ns = row.get::<_, String>(4)?.parse::<i128>().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid source mtime",
            )),
        )
    })?;
    let artifact_size = u64::try_from(row.get::<_, i64>(7)?)
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(7, 0))?;
    let classification = AdditionalFileClass::parse(&row.get::<_, String>(9)?).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            9,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid additional file classification",
            )),
        )
    })?;
    Ok(VerifiedAdditionalFile {
        id: row.get(0)?,
        source_id: parse_source_id_sql(&row.get::<_, String>(1)?, 1)?,
        source_relative_path: PathBuf::from(row.get::<_, String>(2)?),
        source_size,
        source_mtime_ns,
        source_sha256: row.get(5)?,
        artifact_relative_path: PathBuf::from(row.get::<_, String>(6)?),
        artifact_size,
        artifact_sha256: row.get(8)?,
        classification,
        backup_run_id: row.get(10)?,
    })
}

fn parse_source_id_sql(value: &str, column: usize) -> rusqlite::Result<SourceId> {
    SourceId::parse(value).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid source id",
            )),
        )
    })
}

fn severity_name(severity: ActivitySeverity) -> &'static str {
    match severity {
        ActivitySeverity::Info => "info",
        ActivitySeverity::Success => "success",
        ActivitySeverity::Warning => "warning",
        ActivitySeverity::Error => "error",
    }
}

fn parse_severity(value: &str) -> Result<ActivitySeverity, CoreError> {
    match value {
        "info" => Ok(ActivitySeverity::Info),
        "success" => Ok(ActivitySeverity::Success),
        "warning" => Ok(ActivitySeverity::Warning),
        "error" => Ok(ActivitySeverity::Error),
        _ => Err(CoreError::LedgerCorrupt),
    }
}

fn to_i64(value: u64) -> Result<i64, CoreError> {
    i64::try_from(value).map_err(|_| CoreError::LedgerCorrupt)
}

fn optional_i64(value: Option<u64>) -> Result<Option<i64>, CoreError> {
    value.map(to_i64).transpose()
}

fn optional_u64(value: Option<i64>) -> Result<Option<u64>, CoreError> {
    value
        .map(|value| u64::try_from(value).map_err(|_| CoreError::LedgerCorrupt))
        .transpose()
}

fn optional_u32(value: Option<i64>) -> Result<Option<u32>, CoreError> {
    value
        .map(|value| u32::try_from(value).map_err(|_| CoreError::LedgerCorrupt))
        .transpose()
}

fn optional_u16(value: Option<i64>) -> Result<Option<u16>, CoreError> {
    value
        .map(|value| u16::try_from(value).map_err(|_| CoreError::LedgerCorrupt))
        .transpose()
}

fn optional_u64_sql(value: Option<i64>, column: usize) -> rusqlite::Result<Option<u64>> {
    value
        .map(|value| {
            u64::try_from(value)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
        })
        .transpose()
}

fn optional_u32_sql(value: Option<i64>, column: usize) -> rusqlite::Result<Option<u32>> {
    value
        .map(|value| {
            u32::try_from(value)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
        })
        .transpose()
}

fn optional_u16_sql(value: Option<i64>, column: usize) -> rusqlite::Result<Option<u16>> {
    value
        .map(|value| {
            u16::try_from(value)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
        })
        .transpose()
}

fn path_text(path: &Path) -> Result<&str, CoreError> {
    path.to_str().ok_or(CoreError::InvalidRequest)
}
