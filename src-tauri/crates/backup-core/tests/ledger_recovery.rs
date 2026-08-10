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
    source::{SourceId, SourceRecord},
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

fn test_source(ledger: &mut Ledger, slot: &str) -> SourceId {
    let source = SourceRecord {
        id: SourceId::new(),
        rule_id: ledger.dji_rule().unwrap().id,
        volume_uuid: format!("test-{slot}-uuid"),
        legacy_slot: Some(slot.to_owned()),
        display_name: format!("Test {slot}"),
    };
    ledger
        .upsert_source(&source, "2026-08-09T00:00:00Z")
        .unwrap();
    source.id
}

fn activity(index: usize, source_id: &SourceId) -> ActivityEntry {
    ActivityEntry {
        occurred_at: format!("2026-08-09T00:00:{:02}Z", index % 60),
        code: format!("event_{index}"),
        source_id: Some(source_id.clone()),
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
    let source_id = test_source(&mut ledger, "TX01");
    for index in 0..55 {
        ledger
            .append_activity(&activity(index, &source_id))
            .unwrap();
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
        let source_id = test_source(&mut ledger, "TX01");
        ledger
            .begin_backup_run("run-1", &source_id, "2026-08-09T00:00:00Z", 42)
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
fn upgrade_v3_restores_durable_evidence_for_already_retired_wavs() {
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
        .execute_batch(include_str!("../migrations/0003_batch_manifests.sql"))
        .unwrap();
    connection
        .execute_batch(
            r#"
            INSERT INTO backup_runs(
                id, started_at, finished_at, outcome, required_copy_bytes,
                batch_phase, frozen_m4a_conversion, m4a_profile_id
            ) VALUES (
                'run-v3', '2026-08-10T00:00:00Z', '2026-08-10T00:01:00Z',
                'completed', 4, 'm4a_cohort_verified', 1, 'aac_lc_128k_v1'
            );
            INSERT INTO recordings(
                id, transmitter, source_relative_path, source_size, source_mtime_ns,
                source_sha256, destination_relative_path, destination_size,
                destination_sha256, verified_at, backup_run_id, artifact_format,
                artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
                artifact_valid_frames, artifact_duration_micros, conversion_status
            ) VALUES (
                'recording-v3', 'TX01', 'TX_MIC001_20260810_010203/recording.wav',
                4, '1',
                'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                '2026/2026-08-10/TX01/recording.m4a', 2,
                'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                '2026-08-10T00:01:00Z', 'run-v3', 'm4a', 'aac', 48000, 1,
                48000, 1000000, 'complete'
            );
            "#,
        )
        .unwrap();
    drop(connection);

    let ledger = Ledger::open(&path).unwrap();
    let (relative_path, byte_count, sha256) = ledger
        .superseded_wav_evidence("recording-v3")
        .unwrap()
        .unwrap();

    assert_eq!(
        relative_path,
        Path::new("2026/2026-08-10/TX01/recording.wav")
    );
    assert_eq!(byte_count, 4);
    assert_eq!(sha256, "a".repeat(64));
    assert!(ledger.pending_superseded_wavs().unwrap().is_empty());
}

#[test]
fn upgrade_v4_adds_the_same_dji_preset_as_a_fresh_ledger() {
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
        .execute_batch(include_str!("../migrations/0003_batch_manifests.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!(
            "../migrations/0004_durable_superseded_wav_evidence.sql"
        ))
        .unwrap();
    drop(connection);

    let ledger = Ledger::open(&path).unwrap();
    let upgraded_preset = ledger.dji_rule().unwrap();
    let fresh = Ledger::open(directory.path().join("fresh.sqlite3")).unwrap();
    let fresh_preset = fresh.dji_rule().unwrap();

    assert_eq!(upgraded_preset.id, fresh_preset.id);
    assert_eq!(upgraded_preset.name, fresh_preset.name);
    assert_eq!(
        upgraded_preset.backup_file_globs,
        fresh_preset.backup_file_globs
    );
    drop(ledger);
    drop(fresh);

    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 5",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn upgrade_v4_backfills_dynamic_sources_without_rewriting_evidence() {
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
        .execute_batch(include_str!("../migrations/0003_batch_manifests.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!(
            "../migrations/0004_durable_superseded_wav_evidence.sql"
        ))
        .unwrap();
    connection
        .execute_batch(
            r#"
            INSERT INTO paired_devices(
                transmitter, volume_uuid, protocol, media_name, nominal_capacity, paired_at
            ) VALUES
                ('TX01', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', 'USB',
                 'Wireless Mic Tx Media', 15636365312, '2026-08-09T00:00:00Z'),
                ('TX02', 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', 'USB',
                 'Mic Tx', 15636365312, '2026-08-09T00:00:01Z');

            INSERT INTO backup_runs(
                id, started_at, finished_at, outcome, required_copy_bytes, batch_phase
            ) VALUES (
                'run-paired', '2026-08-09T01:00:00Z', '2026-08-09T02:01:00Z',
                'complete', 9, 'completed'
            );

            INSERT INTO recordings(
                id, transmitter, source_relative_path, source_size, source_mtime_ns,
                source_sha256, destination_relative_path, destination_size,
                destination_sha256, verified_at, backup_run_id
            ) VALUES
                ('recording-tx01', 'TX01', 'TX01_MIC001_20260809_010000.wav', 4, '1',
                 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 '2026/2026-08-09/TX01/one.wav', 4,
                 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                 '2026-08-09T01:01:00Z', 'run-paired'),
                ('recording-tx02', 'TX02', 'TX02_MIC001_20260809_020000.wav', 5, '2',
                 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                 '2026/2026-08-09/TX02/two.wav', 5,
                 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                 '2026-08-09T02:01:00Z', 'run-paired');

            INSERT INTO additional_files(
                id, transmitter, source_relative_path, source_size, source_mtime_ns,
                source_sha256, artifact_relative_path, artifact_size, artifact_sha256,
                classification, backup_run_id
            ) VALUES (
                'additional-tx01', 'TX01', 'TX_MIC001/notes.txt', 3, '3',
                'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                'source-extras/TX01/notes.txt', 3,
                'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                'other', 'run-paired'
            );

            INSERT INTO activity(
                occurred_at, code, transmitter, count_value, severity
            ) VALUES ('2026-08-09T02:01:00Z', 'backup_completed', 'TX02', 1, 'success');

            INSERT INTO deletion_runs(
                id, transmitter, started_at, finished_at, outcome,
                proposed_file_count, proposed_bytes
            ) VALUES (
                'deletion-tx01', 'TX01', '2026-08-09T03:00:00Z',
                '2026-08-09T03:01:00Z', 'complete', 1, 4
            );
            INSERT INTO deletion_items(deletion_run_id, recording_id, outcome, removed_at)
            VALUES ('deletion-tx01', 'recording-tx01', 'moved_to_trash',
                    '2026-08-09T03:01:00Z');
            "#,
        )
        .unwrap();
    drop(connection);

    let ledger = Ledger::open(&path).unwrap();
    let dji = ledger.dji_rule().unwrap();
    let sources = ledger.sources_for_rule(&dji.id).unwrap();
    assert_eq!(sources.len(), 2);
    let tx01 = sources
        .iter()
        .find(|source| source.legacy_slot.as_deref() == Some("TX01"))
        .unwrap();
    let tx02 = sources
        .iter()
        .find(|source| source.legacy_slot.as_deref() == Some("TX02"))
        .unwrap();
    assert_eq!(tx01.volume_uuid, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
    assert_eq!(tx02.volume_uuid, "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
    assert_eq!(
        ledger
            .verified_recording("recording-tx01")
            .unwrap()
            .unwrap()
            .source_id,
        tx01.id
    );
    assert_eq!(
        ledger
            .verified_recording("recording-tx02")
            .unwrap()
            .unwrap()
            .source_id,
        tx02.id
    );
    assert_eq!(
        ledger
            .verified_additional_file("additional-tx01")
            .unwrap()
            .unwrap()
            .source_id,
        tx01.id
    );
    assert_eq!(
        ledger.recent_activity(1).unwrap()[0].source_id,
        Some(tx02.id.clone())
    );
    assert_eq!(
        ledger
            .batch_run_evidence("run-paired")
            .unwrap()
            .unwrap()
            .source_id,
        None
    );
    drop(ledger);

    let connection = Connection::open(&path).unwrap();
    for (table, expected) in [
        ("recordings", 2_i64),
        ("additional_files", 1),
        ("backup_runs", 0),
        ("activity", 1),
        ("deletion_runs", 1),
        ("deletion_items", 1),
    ] {
        let query = format!("SELECT COUNT(*) FROM {table} WHERE source_id IS NOT NULL");
        assert_eq!(
            connection
                .query_row(&query, [], |row| row.get::<_, i64>(0))
                .unwrap(),
            expected,
            "missing source evidence in {table}"
        );
    }
    assert_eq!(
        connection
            .query_row(
                "SELECT source_sha256 FROM recordings WHERE id = 'recording-tx02'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "b".repeat(64)
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT batch_phase FROM backup_runs WHERE id = 'run-paired'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "completed"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT outcome FROM deletion_items WHERE recording_id = 'recording-tx01'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "moved_to_trash"
    );
    assert!(
        connection
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
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
    let source_id = test_source(&mut ledger, "TX02");
    ledger
        .begin_backup_run("run-m4a", &source_id, "2026-08-09T00:00:00Z", 4)
        .unwrap();
    let recording = VerifiedRecording {
        id: "recording-m4a".to_owned(),
        source_id,
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
