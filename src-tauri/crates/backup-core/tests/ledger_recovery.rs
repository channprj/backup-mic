use std::{fs, path::Path};

use backup_core::{
    artifact::{
        ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact, VerifiedAudioProperties,
    },
    device::PairedDevice,
    error::CoreError,
    events::{ActivityEntry, ActivitySeverity},
    ledger::{Ledger, MAX_ACTIVITY_ENTRIES, VerifiedRecording},
    preferences::{BackupPreferences, PreferenceKey},
    state::Transmitter,
};
use rusqlite::Connection;
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

#[test]
fn artifact_and_preferences_migrate_a_version_one_ledger_without_rewriting_hashes() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!("../migrations/0001_initial.sql"))
        .unwrap();
    connection
        .execute_batch(
            r#"
            INSERT INTO backup_runs(id, started_at, finished_at, outcome, required_copy_bytes)
            VALUES ('run-legacy', '2026-08-09T00:00:00Z', '2026-08-09T00:01:00Z', 'complete', 4);
            INSERT INTO recordings(
                id, transmitter, source_relative_path, source_size, source_mtime_ns,
                source_sha256, destination_relative_path, destination_size,
                destination_sha256, verified_at, backup_run_id
            ) VALUES (
                'recording-legacy', 'TX01', 'TX_MIC001_20260809_021747/legacy.wav', 4, '1',
                'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                '2026/2026-08-09/TX01/legacy.wav', 4,
                'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                '2026-08-09T00:01:00Z', 'run-legacy'
            );
            "#,
        )
        .unwrap();
    drop(connection);

    let ledger = Ledger::open(&path).unwrap();
    let recording = ledger
        .verified_recording("recording-legacy")
        .unwrap()
        .unwrap();
    assert_eq!(recording.artifact.format, OutputFormat::Wav);
    assert_eq!(recording.artifact.byte_count, 4);
    assert_eq!(
        recording.artifact.sha256,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(recording.source_sha256, recording.artifact.sha256);
    assert_eq!(
        ledger.read_preferences().unwrap(),
        BackupPreferences::default()
    );
}

#[test]
fn upgrade_v2_preserves_historical_wav_evidence() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!("../migrations/0001_initial.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!(
            "../migrations/0002_artifacts_and_preferences.sql"
        ))
        .unwrap();
    connection
        .execute_batch(
            r#"
            INSERT INTO backup_runs(id, started_at, finished_at, outcome, required_copy_bytes)
            VALUES ('run-v2', '2026-08-09T00:00:00Z', '2026-08-09T00:01:00Z', 'complete', 4);
            INSERT INTO recordings(
                id, transmitter, source_relative_path, source_size, source_mtime_ns,
                source_sha256, destination_relative_path, destination_size,
                destination_sha256, verified_at, backup_run_id, retirement_status
            ) VALUES (
                'recording-v2', 'TX01', 'TX_MIC001_20260809_021747/legacy.wav', 4, '1',
                'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                '2026/2026-08-09/TX01/legacy.wav', 4,
                'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                '2026-08-09T00:01:00Z', 'run-v2', 'legacy_deleted'
            );
            "#,
        )
        .unwrap();
    drop(connection);

    let ledger = Ledger::open(&path).unwrap();
    let recordings = ledger.historical_wav_recordings().unwrap();
    assert_eq!(recordings.len(), 1);
    assert_eq!(
        recordings[0].source_sha256,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(
        recordings[0].artifact.sha256,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert_eq!(
        recordings[0].retirement_status,
        RetirementStatus::LegacyDeleted
    );
}

#[test]
fn artifact_and_preferences_persist_only_typed_boolean_preferences() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    {
        let mut ledger = Ledger::open(&path).unwrap();
        ledger
            .set_preference(
                PreferenceKey::AutomaticBackup,
                false,
                "2026-08-09T00:00:00Z",
            )
            .unwrap();
        ledger
            .set_preference(PreferenceKey::AutomaticTrash, true, "2026-08-09T00:00:01Z")
            .unwrap();
    }

    let ledger = Ledger::open(&path).unwrap();
    assert_eq!(
        ledger.read_preferences().unwrap(),
        BackupPreferences {
            automatic_backup: false,
            m4a_conversion: true,
            automatic_trash: true,
        }
    );
}

#[test]
fn artifact_and_preferences_refuse_malformed_persisted_values() {
    let directory = tempdir().unwrap();
    let mut ledger = Ledger::open(directory.path().join("ledger.sqlite3")).unwrap();
    ledger
        .set_setting("m4a_conversion", "\"sometimes\"", "2026-08-09T00:00:00Z")
        .unwrap();

    assert!(ledger.read_preferences().is_err());
}

#[test]
fn artifact_and_preferences_reject_invalid_m4a_audio_properties_before_sqlite() {
    let directory = tempdir().unwrap();
    let mut ledger = Ledger::open(directory.path().join("ledger.sqlite3")).unwrap();
    ledger
        .begin_backup_run("run-m4a", "2026-08-09T00:00:00Z", 4)
        .unwrap();
    let recording = VerifiedRecording {
        id: "recording-m4a".to_owned(),
        transmitter: Transmitter::Tx02,
        source_relative_path: "TX_MIC001_20260809_021747/source.wav".into(),
        source_size: 4,
        source_mtime_ns: 1,
        source_sha256: "a".repeat(64),
        artifact: VerifiedArtifact {
            relative_path: "2026/2026-08-09/TX02/source.m4a".into(),
            format: OutputFormat::M4a,
            byte_count: 2,
            sha256: "b".repeat(64),
            audio: Some(VerifiedAudioProperties {
                codec: "aac".to_owned(),
                sample_rate_hz: 0,
                channel_count: 1,
                valid_frames: 48_000,
                duration_micros: 1_000_000,
            }),
        },
        conversion_status: ConversionStatus::Complete,
        conversion_error_code: None,
        retirement_status: RetirementStatus::Present,
        retired_session_relative_path: None,
        verified_at: "2026-08-09T00:01:00Z".to_owned(),
        backup_run_id: "run-m4a".to_owned(),
    };

    assert!(matches!(
        ledger.commit_verified_recording(&recording),
        Err(CoreError::InvalidRequest)
    ));
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
