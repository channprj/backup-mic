use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, OptionalExtension, params};

use crate::{
    device::PairedDevice,
    error::CoreError,
    events::{ActivityEntry, ActivitySeverity},
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
    pub destination_relative_path: PathBuf,
    pub destination_size: u64,
    pub destination_sha256: String,
    pub verified_at: String,
    pub backup_run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDeletionItem {
    pub recording_id: String,
    pub source_size: u64,
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
        self.connection
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
        if recording.source_sha256 != recording.destination_sha256
            || recording.source_size != recording.destination_size
            || !crate::filesystem::is_safe_relative_path(&recording.source_relative_path)
            || !crate::filesystem::is_safe_relative_path(&recording.destination_relative_path)
        {
            return Err(CoreError::InvalidRequest);
        }
        let source_relative_path = path_text(&recording.source_relative_path)?;
        let destination_relative_path = path_text(&recording.destination_relative_path)?;
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        let existing = transaction
            .query_row(
                r#"SELECT id, destination_relative_path, destination_size, destination_sha256
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
                    ))
                },
            )
            .optional()
            .map_err(CoreError::Ledger)?;
        let id = if let Some((id, existing_path, existing_size, existing_hash)) = existing {
            if existing_path != destination_relative_path
                || optional_u64(Some(existing_size))? != Some(recording.destination_size)
                || existing_hash != recording.destination_sha256
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
                         destination_sha256, verified_at, backup_run_id
                       ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
                    params![
                        recording.id,
                        transmitter_name(recording.transmitter),
                        source_relative_path,
                        to_i64(recording.source_size)?,
                        recording.source_mtime_ns.to_string(),
                        recording.source_sha256,
                        destination_relative_path,
                        to_i64(recording.destination_size)?,
                        recording.destination_sha256,
                        recording.verified_at,
                        recording.backup_run_id,
                    ],
                )
                .map_err(CoreError::Ledger)?;
            recording.id.clone()
        };
        transaction.commit().map_err(CoreError::Ledger)?;
        Ok(id)
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
        }
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn record_deletion_success(
        &mut self,
        deletion_run_id: &str,
        recording_id: &str,
        removed_at: &str,
    ) -> Result<(), CoreError> {
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        let item_changed = transaction
            .execute(
                r#"UPDATE deletion_items
                   SET outcome = 'deleted', removed_at = ?1, error_code = NULL
                   WHERE deletion_run_id = ?2 AND recording_id = ?3 AND outcome = 'pending'"#,
                params![removed_at, deletion_run_id, recording_id],
            )
            .map_err(CoreError::Ledger)?;
        let recording_changed = transaction
            .execute(
                r#"UPDATE recordings
                   SET source_deleted_at = ?1, deletion_error_code = NULL
                   WHERE id = ?2 AND source_deleted_at IS NULL"#,
                params![removed_at, recording_id],
            )
            .map_err(CoreError::Ledger)?;
        if item_changed != 1 || recording_changed != 1 {
            return Err(CoreError::LedgerCorrupt);
        }
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn record_deletion_failure(
        &mut self,
        deletion_run_id: &str,
        recording_id: &str,
        error_code: &str,
    ) -> Result<(), CoreError> {
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        let item_changed = transaction
            .execute(
                r#"UPDATE deletion_items SET outcome = 'failed', error_code = ?1
                   WHERE deletion_run_id = ?2 AND recording_id = ?3 AND outcome = 'pending'"#,
                params![error_code, deletion_run_id, recording_id],
            )
            .map_err(CoreError::Ledger)?;
        let recording_changed = transaction
            .execute(
                "UPDATE recordings SET deletion_error_code = ?1 WHERE id = ?2",
                params![error_code, recording_id],
            )
            .map_err(CoreError::Ledger)?;
        if item_changed != 1 || recording_changed != 1 {
            return Err(CoreError::LedgerCorrupt);
        }
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
        .map_err(CoreError::Ledger)
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

fn path_text(path: &Path) -> Result<&str, CoreError> {
    path.to_str().ok_or(CoreError::InvalidRequest)
}
