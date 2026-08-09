use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    artifact::{
        ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact, VerifiedAudioProperties,
    },
    device::PairedDevice,
    error::CoreError,
    events::{ActivityEntry, ActivitySeverity},
    preferences::{BackupPreferences, PreferenceKey, decode_bool},
    recovery::DELETION_DISABLED_REINDEX_REQUIRED,
    state::Transmitter,
};

pub const MAX_ACTIVITY_ENTRIES: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRecording {
    pub id: String,
    pub transmitter: Transmitter,
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
pub struct PendingDeletionItem {
    pub recording_id: String,
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
                     occurred_at, code, transmitter, count_value, byte_value, severity
                   ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
                params![
                    entry.occurred_at,
                    entry.code,
                    entry.transmitter.map(transmitter_name),
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
                r#"SELECT occurred_at, code, transmitter, count_value, byte_value, severity
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
            let (occurred_at, code, transmitter, count_value, byte_value, severity) =
                row.map_err(CoreError::Ledger)?;
            Ok(ActivityEntry {
                occurred_at,
                code,
                transmitter: transmitter
                    .map(|value| parse_transmitter(&value))
                    .transpose()?,
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
        started_at: &str,
        required_copy_bytes: u64,
    ) -> Result<(), CoreError> {
        self.connection
            .execute(
                r#"INSERT INTO backup_runs(
                     id, started_at, outcome, required_copy_bytes
                   ) VALUES (?1, ?2, 'running', ?3)"#,
                params![id, started_at, to_i64(required_copy_bytes)?],
            )
            .map_err(CoreError::Ledger)?;
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
                   WHERE transmitter = ?1 AND source_relative_path = ?2
                     AND source_size = ?3 AND source_mtime_ns = ?4 AND source_sha256 = ?5"#,
                params![
                    transmitter_name(recording.transmitter),
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
                         id, transmitter, source_relative_path, source_size, source_mtime_ns,
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
                        transmitter_name(recording.transmitter),
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
                r#"SELECT id, transmitter, source_relative_path, source_size, source_mtime_ns,
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

    pub fn verified_recording_for_source(
        &self,
        transmitter: Transmitter,
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
                r#"SELECT id, transmitter, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, destination_relative_path, destination_size,
                          destination_sha256, verified_at, backup_run_id, artifact_format,
                          artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
                          artifact_valid_frames, artifact_duration_micros, conversion_status,
                          conversion_error_code, retirement_status,
                          retired_session_relative_path
                   FROM recordings
                   WHERE transmitter = ?1 AND source_relative_path = ?2
                     AND source_size = ?3 AND source_mtime_ns = ?4
                     AND source_sha256 = ?5"#,
                params![
                    transmitter_name(transmitter),
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
        validate_verified_recording(recording)?;
        let destination_relative_path = path_text(&recording.artifact.relative_path)?;
        let audio = recording
            .artifact
            .audio
            .as_ref()
            .ok_or(CoreError::InvalidRequest)?;
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
                       backup_run_id = ?13
                   WHERE id = ?14 AND transmitter = ?15 AND source_relative_path = ?16
                     AND source_size = ?17 AND source_mtime_ns = ?18
                     AND source_sha256 = ?19"#,
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
                    recording.id,
                    transmitter_name(recording.transmitter),
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

    pub fn verified_recording_count(&self) -> Result<u64, CoreError> {
        let count = self
            .connection
            .query_row("SELECT COUNT(*) FROM recordings", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(CoreError::Ledger)?;
        u64::try_from(count).map_err(|_| CoreError::LedgerCorrupt)
    }

    pub fn legacy_retired_recordings(
        &self,
        transmitter: Transmitter,
    ) -> Result<Vec<LegacyRetiredRecording>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT id, source_relative_path
                   FROM recordings
                   WHERE transmitter = ?1 AND retirement_status = 'legacy_deleted'
                   ORDER BY source_relative_path"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([transmitter_name(transmitter)], |row| {
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
        transmitter: Transmitter,
        started_at: &str,
        items: &[PendingDeletionItem],
    ) -> Result<(), CoreError> {
        let proposed_bytes = items.iter().try_fold(0_u64, |total, item| {
            total
                .checked_add(item.source_size)
                .ok_or(CoreError::InvalidRequest)
        })?;
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        transaction
            .execute(
                r#"INSERT INTO deletion_runs(
                     id, transmitter, started_at, outcome, proposed_file_count, proposed_bytes
                   ) VALUES (?1, ?2, ?3, 'running', ?4, ?5)"#,
                params![
                    id,
                    transmitter_name(transmitter),
                    started_at,
                    to_i64(u64::try_from(items.len()).map_err(|_| CoreError::InvalidRequest)?)?,
                    to_i64(proposed_bytes)?,
                ],
            )
            .map_err(CoreError::Ledger)?;
        for item in items {
            transaction
                .execute(
                    r#"INSERT INTO deletion_items(deletion_run_id, recording_id, outcome)
                       VALUES (?1, ?2, 'pending')"#,
                    params![id, item.recording_id],
                )
                .map_err(CoreError::Ledger)?;
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
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn record_deletion_target_success(
        &mut self,
        deletion_run_id: &str,
        recording_ids: &[&str],
        removed_at: &str,
        retired_session_relative_path: Option<&Path>,
    ) -> Result<(), CoreError> {
        if recording_ids.is_empty() {
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
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn record_deletion_target_failure(
        &mut self,
        deletion_run_id: &str,
        recording_ids: &[&str],
        error_code: &str,
    ) -> Result<(), CoreError> {
        if recording_ids.is_empty() {
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
        transmitter: parse_transmitter_sql(&row.get::<_, String>(1)?, 1)?,
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

fn parse_transmitter_sql(value: &str, column: usize) -> rusqlite::Result<Transmitter> {
    parse_transmitter(value).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid transmitter",
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
