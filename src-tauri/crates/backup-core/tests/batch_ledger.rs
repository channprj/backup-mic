use backup_core::{
    additional_file::{AdditionalFileClass, VerifiedAdditionalFile},
    artifact::{
        ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact, VerifiedAudioProperties,
    },
    batch::{BatchPhase, FrozenPreferences, M4A_PROFILE_ID},
    error::CoreError,
    ledger::{Ledger, VerifiedRecording},
    source::{SourceId, SourceRecord},
};
use tempfile::tempdir;

fn test_source(ledger: &mut Ledger) -> SourceId {
    let source = SourceRecord {
        id: SourceId::new(),
        rule_id: ledger.dji_rule().unwrap().id,
        volume_uuid: "batch-ledger-source".to_owned(),
        legacy_slot: Some("TX01".to_owned()),
        display_name: "Batch Ledger Source".to_owned(),
    };
    ledger
        .upsert_source(&source, "2026-08-10T00:00:00Z")
        .unwrap();
    source.id
}

fn wav_recording(
    id: &str,
    source_id: &SourceId,
    source_name: &str,
    hash_byte: char,
) -> VerifiedRecording {
    VerifiedRecording {
        id: id.to_owned(),
        source_id: source_id.clone(),
        source_relative_path: format!("TX_MIC001_20260810_001116/{source_name}").into(),
        source_size: 4,
        source_mtime_ns: 1,
        source_sha256: hash_byte.to_string().repeat(64),
        artifact: VerifiedArtifact {
            relative_path: format!("2026/2026-08-10/TX01/{source_name}").into(),
            format: OutputFormat::Wav,
            byte_count: 4,
            sha256: hash_byte.to_string().repeat(64),
            audio: None,
        },
        conversion_status: ConversionStatus::NotRequired,
        conversion_error_code: None,
        retirement_status: RetirementStatus::Present,
        retired_session_relative_path: None,
        verified_at: "2026-08-10T00:00:10Z".to_owned(),
        backup_run_id: "run-1".to_owned(),
    }
}

fn m4a_recording(
    id: &str,
    source_id: &SourceId,
    source_name: &str,
    source_hash_byte: char,
) -> VerifiedRecording {
    let mut recording = wav_recording(id, source_id, source_name, source_hash_byte);
    recording.artifact = VerifiedArtifact {
        relative_path: format!(
            "2026/2026-08-10/TX01/{}.m4a",
            source_name.trim_end_matches(".wav")
        )
        .into(),
        format: OutputFormat::M4a,
        byte_count: 2,
        sha256: "d".repeat(64),
        audio: Some(VerifiedAudioProperties {
            codec: "aac".to_owned(),
            sample_rate_hz: 48_000,
            channel_count: 1,
            valid_frames: 48_000,
            duration_micros: 1_000_000,
        }),
    };
    recording.conversion_status = ConversionStatus::Complete;
    recording
}

#[test]
fn batch_barrier_and_additional_file_evidence_survive_reopen() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    let mut ledger = Ledger::open(&path).unwrap();
    let source_id = test_source(&mut ledger);
    let additional = VerifiedAdditionalFile {
        id: "additional-1".to_owned(),
        source_id: source_id.clone(),
        source_relative_path: "TX_MIC001_20260810_001116/external.m4a".into(),
        source_size: 3,
        source_mtime_ns: 2,
        source_sha256: "c".repeat(64),
        artifact_relative_path:
            "source-extras/2026/2026-08-10/TX01/TX_MIC001_20260810_001116/external.m4a".into(),
        artifact_size: 3,
        artifact_sha256: "c".repeat(64),
        classification: AdditionalFileClass::M4a,
        backup_run_id: "run-1".to_owned(),
    };
    ledger
        .begin_batch_run(
            "run-1",
            &source_id,
            "2026-08-10T00:00:00Z",
            11,
            FrozenPreferences {
                automatic_backup: true,
                m4a_conversion: true,
                automatic_trash: false,
            },
        )
        .unwrap();
    ledger
        .advance_batch_phase("run-1", BatchPhase::Copying)
        .unwrap();
    ledger
        .commit_verified_recording(&wav_recording("recording-1", &source_id, "first.wav", 'a'))
        .unwrap();
    ledger
        .commit_verified_recording(&wav_recording("recording-2", &source_id, "second.wav", 'b'))
        .unwrap();
    ledger.commit_verified_additional_file(&additional).unwrap();
    let mut replayed_additional = additional.clone();
    replayed_additional.id = "additional-replayed-id".to_owned();
    assert_eq!(
        ledger
            .commit_verified_additional_file(&replayed_additional)
            .unwrap(),
        "additional-1"
    );
    ledger
        .advance_batch_phase("run-1", BatchPhase::CopiesVerified)
        .unwrap();
    ledger
        .begin_conversion_cohort(
            "run-1",
            &["recording-1".to_owned(), "recording-2".to_owned()],
            M4A_PROFILE_ID,
        )
        .unwrap();
    assert!(matches!(
        ledger.mark_conversion_item_verified("run-1", "recording-1"),
        Err(CoreError::InvalidRequest)
    ));
    ledger
        .replace_verified_artifact(&m4a_recording("recording-1", &source_id, "first.wav", 'a'))
        .unwrap();
    ledger
        .mark_conversion_item_verified("run-1", "recording-1")
        .unwrap();
    assert!(matches!(
        ledger.commit_m4a_barrier("run-1"),
        Err(CoreError::InvalidRequest)
    ));
    ledger
        .replace_verified_artifact(&m4a_recording("recording-2", &source_id, "second.wav", 'b'))
        .unwrap();
    ledger
        .mark_conversion_item_verified("run-1", "recording-2")
        .unwrap();
    ledger.commit_m4a_barrier("run-1").unwrap();
    drop(ledger);

    let ledger = Ledger::open(&path).unwrap();
    let run = ledger.batch_run_evidence("run-1").unwrap().unwrap();
    assert_eq!(run.source_id, Some(source_id));
    assert_eq!(run.phase, BatchPhase::M4aCohortVerified);
    assert_eq!(run.m4a_profile_id.as_deref(), Some("aac_lc_128k_v1"));
    assert_eq!(
        ledger.verified_additional_file("additional-1").unwrap(),
        Some(additional)
    );
}

#[test]
fn batch_phase_transitions_require_the_exact_predecessor() {
    let directory = tempdir().unwrap();
    let mut ledger = Ledger::open(directory.path().join("ledger.sqlite3")).unwrap();
    let source_id = test_source(&mut ledger);
    ledger
        .begin_batch_run(
            "run-1",
            &source_id,
            "2026-08-10T00:00:00Z",
            0,
            FrozenPreferences {
                automatic_backup: true,
                m4a_conversion: false,
                automatic_trash: false,
            },
        )
        .unwrap();

    assert!(matches!(
        ledger.advance_batch_phase("run-1", BatchPhase::CopiesVerified),
        Err(CoreError::InvalidRequest)
    ));
    ledger
        .advance_batch_phase("run-1", BatchPhase::Copying)
        .unwrap();
    assert!(matches!(
        ledger.advance_batch_phase("run-1", BatchPhase::Copying),
        Err(CoreError::InvalidRequest)
    ));
}

#[test]
fn a_fresh_destination_copy_replaces_stale_m4a_evidence() {
    let directory = tempdir().unwrap();
    let mut ledger = Ledger::open(directory.path().join("ledger.sqlite3")).unwrap();
    let source_id = test_source(&mut ledger);
    for (id, started_at) in [
        ("run-1", "2026-08-10T00:00:00Z"),
        ("run-after-destination-change", "2026-08-10T00:01:00Z"),
    ] {
        ledger
            .begin_batch_run(
                id,
                &source_id,
                started_at,
                4,
                FrozenPreferences {
                    automatic_backup: true,
                    m4a_conversion: true,
                    automatic_trash: false,
                },
            )
            .unwrap();
    }
    let wav = wav_recording("recording-1", &source_id, "first.wav", 'a');
    ledger.commit_verified_recording(&wav).unwrap();
    ledger
        .replace_verified_artifact(&m4a_recording("recording-1", &source_id, "first.wav", 'a'))
        .unwrap();

    let mut fresh_copy = wav_recording("new-random-id", &source_id, "first.wav", 'a');
    fresh_copy.backup_run_id = "run-after-destination-change".to_owned();
    fresh_copy.verified_at = "2026-08-10T00:01:00Z".to_owned();
    assert_eq!(
        ledger
            .replace_verified_recording_from_fresh_copy(&fresh_copy)
            .unwrap(),
        "recording-1"
    );

    let replaced = ledger.verified_recording("recording-1").unwrap().unwrap();
    assert_eq!(replaced.artifact, fresh_copy.artifact);
    assert_eq!(replaced.conversion_status, ConversionStatus::NotRequired);
    assert_eq!(replaced.backup_run_id, "run-after-destination-change");
    assert_eq!(ledger.superseded_wav_evidence("recording-1").unwrap(), None);
}
