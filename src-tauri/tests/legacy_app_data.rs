use std::{fs, os::unix::fs::PermissionsExt};

use backup_core::{
    ledger::Ledger,
    rule::BackupRuleDraft,
    source::{SourceId, SourceRecord},
};
use backup_mic_lib::legacy_app_data::{
    LegacyMigrationOutcome, migrate_legacy_ledger, prepare_app_data,
};
use rusqlite::{Connection, params};
use tempfile::tempdir;

fn zoom_rule() -> BackupRuleDraft {
    BackupRuleDraft {
        id: None,
        name: "Zoom H1n".to_owned(),
        archive_directory_name: "Zoom H1n".to_owned(),
        enabled: true,
        volume_name_glob: "ZOOM_*".to_owned(),
        required_path_globs: vec!["RECORD/**".to_owned()],
        backup_file_globs: vec!["RECORD/**/*.WAV".to_owned()],
        session_directory_globs: vec![],
        filename_prefix: "zoom-".to_owned(),
        filename_suffix: "-field".to_owned(),
    }
}

fn seed_legacy(path: &std::path::Path) {
    let mut ledger = Ledger::open(path).unwrap();
    ledger
        .set_setting(
            "destination_path",
            &serde_json::to_string("/Users/example/Existing Backup").unwrap(),
            "2026-08-10T01:00:00Z",
        )
        .unwrap();
    let rule = ledger
        .save_backup_rule(zoom_rule(), "2026-08-10T01:01:00Z")
        .unwrap();
    ledger
        .upsert_source(
            &SourceRecord {
                id: SourceId::parse("aaaa0000-0000-4000-8000-000000000001").unwrap(),
                rule_id: rule.id,
                volume_uuid: "LEGACY-ZOOM-UUID".to_owned(),
                legacy_slot: None,
                display_name: "ZOOM_LEGACY".to_owned(),
            },
            "2026-08-10T01:02:00Z",
        )
        .unwrap();
}

fn assert_migrated_state(path: &std::path::Path, expected_destination: &str) {
    let ledger = Ledger::open(path).unwrap();
    assert_eq!(
        ledger.setting("destination_path").unwrap(),
        Some(serde_json::to_string(expected_destination).unwrap())
    );
    let rule = ledger
        .backup_rules(false)
        .unwrap()
        .into_iter()
        .find(|rule| rule.name == "Zoom H1n")
        .unwrap();
    let sources = ledger.sources_for_rule(&rule.id).unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].display_name, "ZOOM_LEGACY");
}

#[test]
fn migrates_a_closed_legacy_ledger_without_mutating_it() {
    let directory = tempdir().unwrap();
    let legacy = directory.path().join("legacy/ledger.sqlite3");
    let current = directory.path().join("current/ledger.sqlite3");
    seed_legacy(&legacy);
    let legacy_before = fs::read(&legacy).unwrap();

    assert_eq!(
        migrate_legacy_ledger(&legacy, &current).unwrap(),
        LegacyMigrationOutcome::Migrated
    );

    assert_eq!(fs::read(&legacy).unwrap(), legacy_before);
    assert_eq!(
        fs::metadata(&current).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let current_entries = fs::read_dir(current.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(current_entries, ["ledger.sqlite3"]);
    assert_migrated_state(&current, "/Users/example/Existing Backup");
}

#[test]
fn online_backup_includes_committed_wal_content_and_leaves_legacy_files_unchanged() {
    let directory = tempdir().unwrap();
    let legacy = directory.path().join("legacy/ledger.sqlite3");
    let current = directory.path().join("current/ledger.sqlite3");
    seed_legacy(&legacy);

    let connection = Connection::open(&legacy).unwrap();
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .unwrap();
    connection
        .pragma_update(None, "wal_autocheckpoint", 0)
        .unwrap();
    connection
        .execute(
            "UPDATE settings SET value_json = ?1 WHERE key = 'destination_path'",
            params![serde_json::to_string("/Users/example/WAL Backup").unwrap()],
        )
        .unwrap();
    let wal = legacy.with_file_name("ledger.sqlite3-wal");
    assert!(wal.is_file());
    let legacy_before = fs::read(&legacy).unwrap();
    let wal_before = fs::read(&wal).unwrap();

    assert_eq!(
        migrate_legacy_ledger(&legacy, &current).unwrap(),
        LegacyMigrationOutcome::Migrated
    );

    assert_eq!(fs::read(&legacy).unwrap(), legacy_before);
    assert_eq!(fs::read(&wal).unwrap(), wal_before);
    assert_migrated_state(&current, "/Users/example/WAL Backup");
    drop(connection);
}

#[test]
fn corrupt_legacy_or_an_occupied_invalid_target_never_installs_a_replacement() {
    let directory = tempdir().unwrap();
    let corrupt = directory.path().join("corrupt.sqlite3");
    let current = directory.path().join("current/ledger.sqlite3");
    fs::write(&corrupt, b"not sqlite").unwrap();
    let corrupt_before = fs::read(&corrupt).unwrap();

    assert!(migrate_legacy_ledger(&corrupt, &current).is_err());
    assert!(!current.exists());
    assert_eq!(fs::read(&corrupt).unwrap(), corrupt_before);

    let valid = directory.path().join("valid/ledger.sqlite3");
    seed_legacy(&valid);
    fs::create_dir_all(current.parent().unwrap()).unwrap();
    fs::write(&current, b"occupied").unwrap();

    assert!(migrate_legacy_ledger(&valid, &current).is_err());
    assert_eq!(fs::read(&current).unwrap(), b"occupied");
}

#[test]
fn app_data_preparation_is_idempotent_and_uses_exact_ledger_paths() {
    let directory = tempdir().unwrap();
    let legacy_data = directory.path().join("com.channprj.DJIMicBackup");
    let current_data = directory.path().join("com.channprj.BackupMic");
    seed_legacy(&legacy_data.join("ledger.sqlite3"));

    assert_eq!(
        prepare_app_data(&current_data, &legacy_data).unwrap(),
        LegacyMigrationOutcome::Migrated
    );
    assert_eq!(
        prepare_app_data(&current_data, &legacy_data).unwrap(),
        LegacyMigrationOutcome::NotNeeded
    );
    assert_migrated_state(
        &current_data.join("ledger.sqlite3"),
        "/Users/example/Existing Backup",
    );
}
