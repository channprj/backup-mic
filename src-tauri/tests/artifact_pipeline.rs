use std::{fs, path::Path, thread, time::Duration};

use backup_core::{
    artifact::{
        AudioDescription, ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact,
    },
    backup::CancellationToken,
    filesystem::modified_nanos,
    hash::hash_file,
    ledger::{Ledger, VerifiedRecording},
    state::{CurrentStage, Transmitter},
};
use dji_mic_backup_lib::{
    artifact_pipeline::{publish_m4a, verify_published_artifact},
    platform::macos::audio::AudioTools,
};
use tempfile::tempdir;

struct FakeAudioTools {
    wrong_channels: bool,
}

impl AudioTools for FakeAudioTools {
    fn inspect(&self, path: &Path) -> Result<AudioDescription, backup_core::error::CoreError> {
        if path.extension().and_then(|value| value.to_str()) == Some("wav") {
            Ok(AudioDescription {
                container: "WAVE".to_owned(),
                codec: "lpcm".to_owned(),
                sample_rate_hz: 48_000,
                channels: 1,
                audio_bytes: 96_000,
                packets: 48_000,
                frames_per_packet: 1,
                valid_frames: 48_000,
                total_frames: 48_000,
                priming_frames: 0,
                remainder_frames: 0,
                duration_micros: 1_000_000,
            })
        } else {
            Ok(AudioDescription {
                container: "m4af".to_owned(),
                codec: "aac".to_owned(),
                sample_rate_hz: 48_000,
                channels: if self.wrong_channels { 2 } else { 1 },
                audio_bytes: 4,
                packets: 47,
                frames_per_packet: 1_024,
                valid_frames: 48_000,
                total_frames: 50_176,
                priming_frames: 2_112,
                remainder_frames: 64,
                duration_micros: 1_000_000,
            })
        }
    }

    fn convert_aac_lc_192k(
        &self,
        _input: &Path,
        output: &Path,
    ) -> Result<(), backup_core::error::CoreError> {
        fs::write(output, b"m4a!").map_err(backup_core::error::CoreError::CopyFailed)
    }
}

fn wav_recording(destination: &Path, ledger: &mut Ledger) -> VerifiedRecording {
    let relative = Path::new("2026/2026-08-09/TX02/recording.wav");
    let path = destination.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"wav!").unwrap();
    let digest = hash_file(&path).unwrap();
    let source_mtime_ns = modified_nanos(&fs::metadata(&path).unwrap()).unwrap();
    ledger
        .begin_backup_run("run", "2026-08-09T00:00:00Z", digest.size)
        .unwrap();
    let recording = VerifiedRecording {
        id: "recording".to_owned(),
        transmitter: Transmitter::Tx02,
        source_relative_path: "TX_MIC001_20260809_021747/recording.wav".into(),
        source_size: digest.size,
        source_mtime_ns,
        source_sha256: digest.sha256.clone(),
        artifact: VerifiedArtifact {
            relative_path: relative.to_path_buf(),
            format: OutputFormat::Wav,
            byte_count: digest.size,
            sha256: digest.sha256,
            audio: None,
        },
        conversion_status: ConversionStatus::NotRequired,
        conversion_error_code: None,
        retirement_status: RetirementStatus::Present,
        retired_session_relative_path: None,
        verified_at: "2026-08-09T00:01:00Z".to_owned(),
        backup_run_id: "run".to_owned(),
    };
    ledger.commit_verified_recording(&recording).unwrap();
    recording
}

#[test]
fn published_artifact_verification_rejects_source_metadata_changes() {
    let destination = tempdir().unwrap();
    let state = tempdir().unwrap();
    let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
    let wav = wav_recording(destination.path(), &mut ledger);
    let result = publish_m4a(
        destination.path(),
        &wav,
        &mut ledger,
        &FakeAudioTools {
            wrong_channels: false,
        },
        &CancellationToken::default(),
        &mut |_| {},
    )
    .unwrap();
    let live_source = destination.path().join(&wav.artifact.relative_path);
    for _ in 0..10 {
        thread::sleep(Duration::from_millis(2));
        fs::write(&live_source, b"wav!").unwrap();
        let current_mtime = modified_nanos(&fs::metadata(&live_source).unwrap()).unwrap();
        if current_mtime != wav.source_mtime_ns {
            break;
        }
    }
    assert_ne!(
        modified_nanos(&fs::metadata(&live_source).unwrap()).unwrap(),
        wav.source_mtime_ns
    );

    assert!(matches!(
        verify_published_artifact(
            destination.path(),
            &live_source,
            &result.recording,
            &FakeAudioTools {
                wrong_channels: false,
            },
            &CancellationToken::default(),
        ),
        Err(backup_core::error::CoreError::SourceChanged)
    ));
}

#[test]
fn publishes_an_inspected_hashed_m4a_and_retains_the_superseded_wav() {
    let destination = tempdir().unwrap();
    let state = tempdir().unwrap();
    let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
    let wav = wav_recording(destination.path(), &mut ledger);
    let mut stages = Vec::new();

    let result = publish_m4a(
        destination.path(),
        &wav,
        &mut ledger,
        &FakeAudioTools {
            wrong_channels: false,
        },
        &CancellationToken::default(),
        &mut |stage| stages.push(stage),
    )
    .unwrap();

    assert_eq!(result.recording.artifact.format, OutputFormat::M4a);
    assert_eq!(
        result.recording.artifact.relative_path,
        Path::new("2026/2026-08-09/TX02/recording.m4a")
    );
    assert!(
        destination
            .path()
            .join(&result.recording.artifact.relative_path)
            .exists()
    );
    assert!(
        destination
            .path()
            .join(&wav.artifact.relative_path)
            .exists()
    );
    assert_eq!(result.superseded_wav, wav.artifact.relative_path);
    assert_eq!(
        stages,
        vec![CurrentStage::Conversion, CurrentStage::ArtifactVerification]
    );
    assert_eq!(
        ledger
            .verified_recording("recording")
            .unwrap()
            .unwrap()
            .artifact
            .format,
        OutputFormat::M4a
    );
    let from_source_identity = ledger
        .verified_recording_for_source(
            Transmitter::Tx02,
            &wav.source_relative_path,
            wav.source_size,
            wav.source_mtime_ns,
            &wav.source_sha256,
        )
        .unwrap()
        .unwrap();
    verify_published_artifact(
        destination.path(),
        &destination.path().join(&wav.artifact.relative_path),
        &from_source_identity,
        &FakeAudioTools {
            wrong_channels: false,
        },
        &CancellationToken::default(),
    )
    .unwrap();
}

#[test]
fn invalid_conversion_keeps_the_verified_wav_and_cleans_owned_parts() {
    let destination = tempdir().unwrap();
    let state = tempdir().unwrap();
    let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
    let wav = wav_recording(destination.path(), &mut ledger);

    assert!(
        publish_m4a(
            destination.path(),
            &wav,
            &mut ledger,
            &FakeAudioTools {
                wrong_channels: true,
            },
            &CancellationToken::default(),
            &mut |_| {},
        )
        .is_err()
    );

    assert!(
        destination
            .path()
            .join(&wav.artifact.relative_path)
            .exists()
    );
    assert_eq!(
        ledger
            .verified_recording("recording")
            .unwrap()
            .unwrap()
            .artifact
            .format,
        OutputFormat::Wav
    );
    let directory = destination.path().join("2026/2026-08-09/TX02");
    assert!(fs::read_dir(directory).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".part-")
    }));
}

#[test]
fn repairs_a_finalized_m4a_only_with_an_app_owned_matching_recovery_marker() {
    let destination = tempdir().unwrap();
    let state = tempdir().unwrap();
    let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
    let wav = wav_recording(destination.path(), &mut ledger);
    let final_relative = wav.artifact.relative_path.with_extension("m4a");
    let final_path = destination.path().join(&final_relative);
    fs::write(&final_path, b"m4a!").unwrap();
    let artifact_digest = hash_file(&final_path).unwrap();
    let marker = final_path
        .parent()
        .unwrap()
        .join(".recording.m4a.recovery-550e8400-e29b-41d4-a716-446655440000");
    fs::write(
        &marker,
        format!(
            "v1\nsource_sha256={}\nartifact_sha256={}\nartifact_bytes={}\n",
            wav.source_sha256, artifact_digest.sha256, artifact_digest.size
        ),
    )
    .unwrap();
    let mut stages = Vec::new();

    let recovered = publish_m4a(
        destination.path(),
        &wav,
        &mut ledger,
        &FakeAudioTools {
            wrong_channels: false,
        },
        &CancellationToken::default(),
        &mut |stage| stages.push(stage),
    )
    .unwrap();

    assert_eq!(recovered.recording.artifact.relative_path, final_relative);
    assert_eq!(stages, vec![CurrentStage::ArtifactVerification]);
    assert!(!marker.exists());
    assert_eq!(
        fs::read_dir(final_path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .filter(
                |entry| entry.path().extension().and_then(|value| value.to_str()) == Some("m4a")
            )
            .count(),
        1
    );
    assert_eq!(
        ledger
            .verified_recording("recording")
            .unwrap()
            .unwrap()
            .artifact
            .format,
        OutputFormat::M4a
    );
}
