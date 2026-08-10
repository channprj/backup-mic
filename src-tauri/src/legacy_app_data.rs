use std::{
    fs::{self, File},
    path::Path,
    time::Duration,
};

use backup_core::{error::CoreError, ledger::Ledger};
use rusqlite::{Connection, OpenFlags, backup::Backup};
use tempfile::NamedTempFile;

pub const LEGACY_BUNDLE_IDENTIFIER: &str = "com.channprj.DJIMicBackup";
pub const CURRENT_BUNDLE_IDENTIFIER: &str = "com.channprj.BackupMic";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LegacyMigrationOutcome {
    NotNeeded,
    Migrated,
}

pub fn prepare_app_data(
    current_app_data: &Path,
    legacy_app_data: &Path,
) -> Result<LegacyMigrationOutcome, CoreError> {
    migrate_legacy_ledger(
        &legacy_app_data.join("ledger.sqlite3"),
        &current_app_data.join("ledger.sqlite3"),
    )
}

pub fn migrate_legacy_ledger(
    legacy_ledger: &Path,
    current_ledger: &Path,
) -> Result<LegacyMigrationOutcome, CoreError> {
    if current_ledger.exists() {
        validate_read_only(current_ledger)?;
        return Ok(LegacyMigrationOutcome::NotNeeded);
    }
    if !legacy_ledger.exists() {
        return Ok(LegacyMigrationOutcome::NotNeeded);
    }
    require_regular_file(legacy_ledger)?;

    let parent = current_ledger.parent().ok_or_else(|| {
        CoreError::LedgerIo(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "ledger target has no parent",
        ))
    })?;
    fs::create_dir_all(parent).map_err(CoreError::LedgerIo)?;
    sync_directory(parent)?;

    let source = open_read_only(legacy_ledger)?;
    require_quick_check(&source)?;

    let mut temporary = NamedTempFile::new_in(parent).map_err(CoreError::LedgerIo)?;
    temporary
        .as_file_mut()
        .set_permissions(owner_only_permissions())
        .map_err(CoreError::LedgerIo)?;
    {
        let mut destination = Connection::open(temporary.path()).map_err(CoreError::Ledger)?;
        let backup = Backup::new(&source, &mut destination).map_err(CoreError::Ledger)?;
        backup
            .run_to_completion(128, Duration::from_millis(10), None)
            .map_err(CoreError::Ledger)?;
        drop(backup);
        require_quick_check(&destination)?;
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(CoreError::LedgerIo)?;

    {
        let migrated = Ledger::open(temporary.path())?;
        drop(migrated);
        let connection = Connection::open(temporary.path()).map_err(CoreError::Ledger)?;
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
            .map_err(CoreError::Ledger)?;
        require_quick_check(&connection)?;
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(CoreError::LedgerIo)?;

    temporary
        .persist_noclobber(current_ledger)
        .map_err(|error| CoreError::LedgerIo(error.error))?;
    sync_directory(parent)?;
    Ok(LegacyMigrationOutcome::Migrated)
}

fn open_read_only(path: &Path) -> Result<Connection, CoreError> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(CoreError::Ledger)
}

fn validate_read_only(path: &Path) -> Result<(), CoreError> {
    require_regular_file(path)?;
    let connection = open_read_only(path)?;
    require_quick_check(&connection)
}

fn require_quick_check(connection: &Connection) -> Result<(), CoreError> {
    let result = connection
        .query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
        .map_err(CoreError::Ledger)?;
    if result == "ok" {
        Ok(())
    } else {
        Err(CoreError::LedgerCorrupt)
    }
}

fn require_regular_file(path: &Path) -> Result<(), CoreError> {
    let metadata = fs::symlink_metadata(path).map_err(CoreError::LedgerIo)?;
    if metadata.file_type().is_file() {
        Ok(())
    } else {
        Err(CoreError::LedgerIo(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "ledger path is not a regular file",
        )))
    }
}

fn sync_directory(path: &Path) -> Result<(), CoreError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(CoreError::LedgerIo)
}

#[cfg(unix)]
fn owner_only_permissions() -> fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    fs::Permissions::from_mode(0o600)
}

#[cfg(not(unix))]
fn owner_only_permissions() -> fs::Permissions {
    fs::Permissions::readonly()
}
