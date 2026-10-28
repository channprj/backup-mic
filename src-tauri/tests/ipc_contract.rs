use std::{fs, path::PathBuf};

use backup_mic_lib::commands::REGISTERED_COMMANDS;
use backup_mic_lib::dto::{AppSnapshotDto, TrashProposalSummaryDto};
use serde_json::Value;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../contracts/fixtures")
        .join(name)
}

fn fixture(name: &str) -> String {
    fs::read_to_string(fixture_path(name)).expect("fixture must be readable")
}

fn assert_safe_json(value: &Value) {
    let serialized = serde_json::to_string(value).expect("value serializes");
    let lower = serialized.to_ascii_lowercase();
    for forbidden in [
        "/volumes/",
        "/users/",
        "source_path",
        "volume_uuid",
        "sha256",
        "source_filename",
        "ledger_id",
        "recording_id",
    ] {
        assert!(!lower.contains(forbidden), "fixture contains {forbidden}");
    }
}

#[test]
fn snapshot_fixtures_round_trip_through_rust_contract() {
    for name in [
        "backup-copying.json",
        "backup-complete.json",
        "error-destination-full.json",
        "partial-trash.json",
    ] {
        let snapshot: AppSnapshotDto =
            serde_json::from_str(&fixture(name)).expect("valid snapshot");
        let value = serde_json::to_value(snapshot).expect("snapshot serializes");
        assert!(value.get("settings").is_some());
        assert!(value.get("sources").is_some());
        assert!(value.get("backup_rules").is_some());
        assert!(value.get("transmitters").is_none());
        assert!(value.get("pairing_candidates").is_none());
        assert!(
            !serde_json::to_string(&value)
                .unwrap()
                .contains("transmitter")
        );
        assert_safe_json(&value);
    }
}

#[test]
fn trash_proposal_fixture_round_trips_through_rust_contract() {
    let proposal: TrashProposalSummaryDto =
        serde_json::from_str(&fixture("trash-proposal.json")).expect("valid proposal");
    let value = serde_json::to_value(proposal).expect("proposal serializes");
    assert_safe_json(&value);
}

#[test]
fn webview_capability_has_no_direct_plugin_authority() {
    let capability_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("capabilities/main.json");
    let capability: Value = serde_json::from_str(
        &fs::read_to_string(capability_path).expect("capability must be readable"),
    )
    .expect("capability must be valid JSON");
    assert_eq!(
        capability["permissions"],
        serde_json::json!(["core:default"])
    );
    assert_eq!(REGISTERED_COMMANDS.len(), 21);
}

#[test]
fn transcription_fixture_is_exactly_what_rust_sends() {
    use backup_core::{
        error::{PublicError, PublicErrorCode},
        transcription::{TranscriptFormats, WHISPER_MODELS, WhisperOptions},
    };
    use backup_mic_lib::transcription::dto::{
        ModelDownloadDto, ModelSourceDto, SelectedModelDto, TranscriptionJobDto,
        TranscriptionPhaseDto, TranscriptionSnapshotDto, WhisperModelDto,
    };

    let models_directory = "~/Library/Application Support/com.channprj.BackupMic/whisper-models";
    let snapshot = TranscriptionSnapshotDto {
        revision: 42,
        enabled: true,
        include_existing_recordings: false,
        output_directory_display: "~/Documents/Backup Mic Transcripts".to_owned(),
        output_directory_is_default: true,
        models_directory_display: models_directory.to_owned(),
        models_directory_is_default: true,
        selected_model: SelectedModelDto {
            source: ModelSourceDto::Catalog,
            id: Some("medium".to_owned()),
            label: "Medium".to_owned(),
            file_display: format!("{models_directory}/ggml-medium.bin"),
            ready: true,
        },
        models: WHISPER_MODELS
            .iter()
            .map(|model| WhisperModelDto {
                id: model.id.to_owned(),
                label: model.label.to_owned(),
                byte_count: model.byte_count,
                english_only: model.english_only,
                quantized: model.quantized,
                installed: matches!(model.id, "tiny" | "medium"),
            })
            .collect(),
        download: Some(ModelDownloadDto {
            model_id: "large-v3-turbo".to_owned(),
            received_bytes: 406_138_819,
            total_bytes: 1_624_555_275,
        }),
        formats: TranscriptFormats::default(),
        whisper: WhisperOptions {
            language: "ko".to_owned(),
            initial_prompt: "백업 마이크 회의록".to_owned(),
            ..WhisperOptions::default()
        },
        job: TranscriptionJobDto {
            phase: TranscriptionPhaseDto::Transcribing,
            current_item: Some("DJI_20261028_101500_TX01.m4a".to_owned()),
            percent: 37,
            queued_count: 3,
            completed_count: 12,
            failed_count: 1,
            last_completed_at: Some("2026-10-28T01:05:00Z".to_owned()),
        },
        error: Some(PublicError {
            code: PublicErrorCode::Internal,
            message_code: "transcription_decode_failed".to_owned(),
            retryable: true,
            transmitter: None,
            source_id: None,
            source_label: None,
        }),
    };

    // Tauri sends command results and events through `serde_json::to_string`, which writes an
    // `f32` in its shortest form; comparing that text keeps `0.2` from reading as `0.2000000029`.
    let sent: Value = serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
    let committed: Value = serde_json::from_str(&fixture("transcription-snapshot.json")).unwrap();
    assert_eq!(sent, committed);
    let serialized = serde_json::to_string(&sent).unwrap().to_ascii_lowercase();
    for forbidden in [
        "/volumes/",
        "/users/",
        "sha256",
        "recording_id",
        "source_path",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "fixture contains {forbidden}"
        );
    }
}
