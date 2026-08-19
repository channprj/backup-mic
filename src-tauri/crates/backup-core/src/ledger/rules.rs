//! Backup rules and the recorder sources bound to them.

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::error::CoreError;
use crate::preset::DJI_PRESET_KIND;
use crate::preset::DJI_PRESET_REVISION;
use crate::preset::dji_mic_mini_2s_preset;
use crate::rule::BackupRule;
use crate::rule::BackupRuleDraft;
use crate::rule::DateFolderLayout;
use crate::rule::DeviceConstraintProfile;
use crate::rule::FilenameProfile;
use crate::rule::RuleId;
use crate::rule::normalized_rule_name;
use crate::rule::validate_rule;
use crate::source::SourceId;
use crate::source::SourceRecord;
use crate::source::validate_source;

use super::Ledger;
use super::sql::{invalid_text_column, optional_u32};

impl Ledger {
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
}

pub(super) const RULE_COLUMNS: &str = r#"id, name, archive_directory_name, enabled,
    volume_name_glob, filename_prefix, filename_suffix, date_folder_layout, filename_profile,
    device_constraint_profile, preset_kind, preset_revision,
    archive_directory_locked, archived_at, created_at, updated_at"#;

pub(super) struct StoredRuleRow {
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

pub(super) fn row_to_stored_rule(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredRuleRow> {
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

pub(super) fn hydrate_rule(
    connection: &Connection,
    stored: StoredRuleRow,
) -> Result<BackupRule, CoreError> {
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

pub(super) fn replace_rule_patterns(
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

pub(super) fn rule_write_error(error: rusqlite::Error) -> CoreError {
    match &error {
        rusqlite::Error::SqliteFailure(details, _)
            if details.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            CoreError::InvalidRule
        }
        _ => CoreError::Ledger(error),
    }
}

pub(super) fn row_to_source_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<SourceRecord> {
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

pub(super) fn source_write_error(error: rusqlite::Error) -> CoreError {
    match &error {
        rusqlite::Error::SqliteFailure(details, _)
            if details.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            CoreError::InvalidRequest
        }
        _ => CoreError::Ledger(error),
    }
}
