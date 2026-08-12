use backup_core::{
    artifact::{ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact},
    ledger::{Ledger, VerifiedRecording},
    rule::BackupRuleDraft,
    source::{SourceId, SourceRecord},
};
use tempfile::tempdir;

fn zoom_draft() -> BackupRuleDraft {
    BackupRuleDraft {
        id: None,
        name: "Zoom H1n".to_owned(),
        archive_directory_name: "Zoom H1n".to_owned(),
        enabled: true,
        volume_name_glob: "ZOOM_*".to_owned(),
        required_path_globs: vec!["RECORD/**".to_owned()],
        backup_file_globs: vec!["RECORD/**/*.WAV".to_owned()],
        session_directory_globs: vec!["RECORD/FOLDER*".to_owned()],
        filename_prefix: "zoom-".to_owned(),
        filename_suffix: String::new(),
        date_folder_layout: Default::default(),
    }
}

fn recording(id: &str, source: &SourceId, run_id: &str, destination: &str) -> VerifiedRecording {
    VerifiedRecording {
        id: id.to_owned(),
        source_id: source.clone(),
        source_relative_path: "RECORD/FOLDER01/REC0001.WAV".into(),
        source_size: 4,
        source_mtime_ns: 1,
        source_sha256: "a".repeat(64),
        artifact: VerifiedArtifact {
            relative_path: destination.into(),
            format: OutputFormat::Wav,
            byte_count: 4,
            sha256: "a".repeat(64),
            audio: None,
        },
        conversion_status: ConversionStatus::NotRequired,
        conversion_error_code: None,
        retirement_status: RetirementStatus::Present,
        retired_session_relative_path: None,
        verified_at: "2026-08-10T01:00:00Z".to_owned(),
        backup_run_id: run_id.to_owned(),
    }
}

#[test]
fn two_same_named_volumes_keep_distinct_source_evidence() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    let mut ledger = Ledger::open(&path).unwrap();
    let rule = ledger
        .save_backup_rule(zoom_draft(), "2026-08-10T00:00:00Z")
        .unwrap();
    let first = SourceRecord {
        id: SourceId::new(),
        rule_id: rule.id.clone(),
        volume_uuid: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_owned(),
        legacy_slot: None,
        display_name: "ZOOM_H1N".to_owned(),
    };
    let second = SourceRecord {
        id: SourceId::new(),
        rule_id: rule.id.clone(),
        volume_uuid: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".to_owned(),
        legacy_slot: None,
        display_name: "ZOOM_H1N".to_owned(),
    };
    ledger
        .upsert_source(&first, "2026-08-10T00:01:00Z")
        .unwrap();
    ledger
        .upsert_source(&second, "2026-08-10T00:01:00Z")
        .unwrap();

    ledger
        .begin_backup_run("run-first", &first.id, "2026-08-10T00:02:00Z", 4)
        .unwrap();
    ledger
        .begin_backup_run("run-second", &second.id, "2026-08-10T00:02:00Z", 4)
        .unwrap();
    let first_recording = recording(
        "recording-first",
        &first.id,
        "run-first",
        "Zoom H1n/2026/08/260810-zoom-REC0001.wav",
    );
    let second_recording = recording(
        "recording-second",
        &second.id,
        "run-second",
        "Zoom H1n/2026/08/260810-zoom-REC0001-duplicate.wav",
    );
    ledger.commit_verified_recording(&first_recording).unwrap();
    ledger.commit_verified_recording(&second_recording).unwrap();

    assert_eq!(ledger.source(&first.id).unwrap(), first);
    assert_eq!(ledger.source(&second.id).unwrap(), second);
    assert_eq!(ledger.sources_for_rule(&rule.id).unwrap().len(), 2);
    assert_eq!(
        ledger
            .verified_recording_candidate_for_source(
                &first.id,
                &first_recording.source_relative_path,
                first_recording.source_size,
                first_recording.source_mtime_ns,
            )
            .unwrap(),
        Some(first_recording.clone())
    );
    assert_eq!(
        ledger
            .verified_recording_for_source(
                &first.id,
                &first_recording.source_relative_path,
                first_recording.source_size,
                first_recording.source_mtime_ns,
                &first_recording.source_sha256,
            )
            .unwrap(),
        Some(first_recording)
    );
    assert_eq!(ledger.verified_recordings().unwrap().len(), 2);
}

#[test]
fn source_ids_serialize_as_opaque_canonical_uuids() {
    let id = SourceId::new();
    let encoded = serde_json::to_string(&id).unwrap();

    assert_eq!(SourceId::parse(id.as_str()).unwrap(), id);
    assert!(uuid::Uuid::parse_str(id.as_str()).is_ok());
    assert!(!encoded.contains("Zoom H1n"));
    assert!(!encoded.contains("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"));
    assert_eq!(serde_json::from_str::<SourceId>(&encoded).unwrap(), id);
    assert!(serde_json::from_str::<SourceId>(r#""not-a-source-id""#).is_err());
}
