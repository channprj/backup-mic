//! Opening the SQLite ledger: validation, migration, and corruption quarantine.

use rusqlite::{Connection, OptionalExtension, params};
use std::fs;
use std::path::Path;

use crate::error::CoreError;
use crate::recovery::DELETION_DISABLED_REINDEX_REQUIRED;

use super::Ledger;
use super::sql::now_timestamp;

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
}

pub(super) fn open_validated(path: &Path) -> Result<Connection, CoreError> {
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

pub(super) fn has_sqlite_header(path: &Path) -> Result<bool, CoreError> {
    use std::io::Read;

    let mut file = fs::File::open(path).map_err(CoreError::LedgerIo)?;
    let mut header = [0_u8; 16];
    if file.read_exact(&mut header).is_err() {
        return Ok(false);
    }
    Ok(&header == b"SQLite format 3\0")
}

pub(super) fn migrate(connection: &Connection) -> Result<(), CoreError> {
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(CoreError::Ledger)?;
    connection
        .execute_batch(include_str!("../../migrations/0001_initial.sql"))
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
                "../../migrations/0002_artifacts_and_preferences.sql"
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
            .execute_batch(include_str!("../../migrations/0003_batch_manifests.sql"))
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
                "../../migrations/0004_durable_superseded_wav_evidence.sql"
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
            .execute_batch(include_str!("../../migrations/0005_backup_rules.sql"))
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
            .execute_batch(include_str!("../../migrations/0006_dynamic_sources.sql"))
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
                "../../migrations/0007_rule_date_folder_layout.sql"
            ))
            .map_err(CoreError::Ledger)?;
    }
    let version_eight_applied = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = 8)",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(CoreError::Ledger)?;
    if !version_eight_applied {
        connection
            .execute_batch(include_str!(
                "../../migrations/0008_dji_archive_day_layout.sql"
            ))
            .map_err(CoreError::Ledger)?;
    }
    Ok(())
}

pub(super) fn quarantine_existing(path: &Path) -> Result<(), CoreError> {
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
