use std::{fs, path::Path};

use backup_core::{
    device::PairedDevice,
    events::{ActivityEntry, ActivitySeverity},
    ledger::{Ledger, MAX_ACTIVITY_ENTRIES},
    state::Transmitter,
};
use tempfile::tempdir;

fn paired_device() -> PairedDevice {
    PairedDevice {
        transmitter: Transmitter::Tx01,
        expected_uuid: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned(),
        expected_protocol: "USB".to_owned(),
        expected_media_name: "Wireless Mic Tx Media".to_owned(),
        expected_capacity: 15_636_365_312,
    }
}

fn activity(index: usize) -> ActivityEntry {
    ActivityEntry {
        occurred_at: format!("2026-08-09T00:00:{:02}Z", index % 60),
        code: format!("event_{index}"),
        transmitter: Some(Transmitter::Tx01),
        count_value: Some(index as u64),
        byte_value: None,
        severity: ActivitySeverity::Info,
    }
}

#[test]
fn new_ledger_migrates_and_persists_pairing() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    {
        let mut ledger = Ledger::open(&path).unwrap();
        ledger
            .pair_device(&paired_device(), "2026-08-09T00:00:00Z")
            .unwrap();
        assert_eq!(ledger.paired_device_count().unwrap(), 1);
    }
    let ledger = Ledger::open(&path).unwrap();
    assert_eq!(ledger.paired_device_count().unwrap(), 1);
    assert!(!ledger.deletion_disabled());
}

#[test]
fn activity_history_keeps_only_the_newest_fifty_entries() {
    let directory = tempdir().unwrap();
    let mut ledger = Ledger::open(directory.path().join("ledger.sqlite3")).unwrap();
    for index in 0..55 {
        ledger.append_activity(&activity(index)).unwrap();
    }
    let entries = ledger.recent_activity(100).unwrap();
    assert_eq!(entries.len(), MAX_ACTIVITY_ENTRIES);
    assert_eq!(entries[0].code, "event_54");
    assert_eq!(entries[49].code, "event_5");
}

#[test]
fn opening_marks_running_backup_runs_interrupted() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    {
        let mut ledger = Ledger::open(&path).unwrap();
        ledger
            .begin_backup_run("run-1", "2026-08-09T00:00:00Z", 42)
            .unwrap();
    }
    let mut ledger = Ledger::open(&path).unwrap();
    assert_eq!(
        ledger
            .mark_interrupted_runs("2026-08-09T00:01:00Z")
            .unwrap(),
        1
    );
    assert_eq!(
        ledger.backup_run_outcome("run-1").unwrap().as_deref(),
        Some("interrupted")
    );
}

#[test]
fn corrupt_database_is_quarantined_and_disables_deletion() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    fs::write(&path, b"not a sqlite database").unwrap();
    fs::write(path.with_file_name("ledger.sqlite3-wal"), b"wal evidence").unwrap();
    fs::write(path.with_file_name("ledger.sqlite3-shm"), b"shm evidence").unwrap();

    let ledger = Ledger::open(&path).unwrap();
    assert!(ledger.deletion_disabled());
    assert!(path.exists());
    let quarantine = directory.path().join("quarantine");
    assert!(quarantine.is_dir());
    assert_eq!(count_files_recursively(&quarantine), 3);
}

fn count_files_recursively(root: &Path) -> usize {
    fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .map(|path| {
            if path.is_dir() {
                count_files_recursively(&path)
            } else {
                1
            }
        })
        .sum()
}
