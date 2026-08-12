use std::{fs, path::Path, process::Command};

use backup_core::{
    artifact::{
        AudioDescription, ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact,
        VerifiedAudioProperties,
    },
    filesystem::modified_nanos,
    hash::hash_file,
    ledger::{Ledger, VerifiedRecording},
    source::{SourceId, SourceRecord},
};
use backup_mic_lib::{
    platform::macos::audio::AudioTools,
    recovery_tool::{RecoveryMode, recover_verified_recordings},
};
use tempfile::TempDir;

struct FakeAudioTools {
    fail_conversion: bool,
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
                channels: 1,
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

    fn convert_aac_lc_128k(
        &self,
        _input: &Path,
        output: &Path,
    ) -> Result<(), backup_core::error::CoreError> {
        if self.fail_conversion {
            return Err(backup_core::error::CoreError::AudioToolFailed);
        }
        fs::write(output, b"m4a!").map_err(backup_core::error::CoreError::CopyFailed)
    }
}

struct Fixture {
    state: TempDir,
    source: TempDir,
    destination: TempDir,
    ledger: Ledger,
    recording_id: String,
    source_path: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let state = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
        let source_id = SourceId::new();
        let rule = ledger.dji_rule().unwrap();
        ledger
            .upsert_source(
                &SourceRecord {
                    id: source_id.clone(),
                    rule_id: rule.id,
                    volume_uuid: "recovery-fixture".to_owned(),
                    legacy_slot: Some("TX01".to_owned()),
                    display_name: "Recovery Fixture".to_owned(),
                },
                "2026-08-12T00:00:00Z",
            )
            .unwrap();
        let relative = Path::new("TX_MIC001_20260810_155045/TX01_MIC001_20260810_155045_edit.wav");
        let source_path = source.path().join(relative);
        fs::create_dir_all(source_path.parent().unwrap()).unwrap();
        fs::write(&source_path, b"verified source wav").unwrap();
        let source_digest = hash_file(&source_path).unwrap();
        let source_mtime_ns = modified_nanos(&fs::metadata(&source_path).unwrap()).unwrap();
        ledger
            .begin_backup_run("recovery-run", &source_id, "2026-08-12T00:00:00Z", 0)
            .unwrap();
        let recording_id = "9f12236f-0781-4ca0-ab9d-b05078e7e512".to_owned();
        ledger
            .commit_verified_recording(&VerifiedRecording {
                id: recording_id.clone(),
                source_id,
                source_relative_path: relative.to_path_buf(),
                source_size: source_digest.size,
                source_mtime_ns,
                source_sha256: source_digest.sha256,
                artifact: VerifiedArtifact {
                    relative_path: "missing/old.m4a".into(),
                    format: OutputFormat::M4a,
                    byte_count: 99,
                    sha256: "a".repeat(64),
                    audio: Some(VerifiedAudioProperties {
                        codec: "aac".to_owned(),
                        sample_rate_hz: 48_000,
                        channel_count: 1,
                        valid_frames: 48_000,
                        duration_micros: 1_000_000,
                    }),
                },
                conversion_status: ConversionStatus::Complete,
                conversion_error_code: None,
                retirement_status: RetirementStatus::MovedToTrash,
                retired_session_relative_path: Some("TX_MIC001_20260810_155045".into()),
                verified_at: "2026-08-12T00:01:00Z".to_owned(),
                backup_run_id: "recovery-run".to_owned(),
            })
            .unwrap();
        Self {
            state,
            source,
            destination,
            ledger,
            recording_id,
            source_path,
        }
    }
}

#[cfg(feature = "recovery-tool")]
#[test]
fn command_defaults_to_dry_run_and_reports_counts_only() {
    let fixture = Fixture::new();
    let output = Command::new(env!("CARGO_BIN_EXE_recover_verified_recordings"))
        .args([
            "--ledger",
            fixture
                .state
                .path()
                .join("ledger.sqlite3")
                .to_str()
                .unwrap(),
            "--destination",
            fixture.destination.path().to_str().unwrap(),
            "--source-root",
            fixture.source.path().to_str().unwrap(),
            "--recording-id",
            &fixture.recording_id,
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!({
            "selected": 1,
            "recoverable": 1,
            "recovered": 0,
            "failed": 0
        })
    );
    assert!(output.stderr.is_empty());
    assert_eq!(fs::read_dir(fixture.destination.path()).unwrap().count(), 0);
}

#[test]
fn dry_run_verifies_evidence_without_changing_files_or_ledger() {
    let mut fixture = Fixture::new();
    let before = fixture
        .ledger
        .verified_recording(&fixture.recording_id)
        .unwrap()
        .unwrap();
    let summary = recover_verified_recordings(
        &mut fixture.ledger,
        fixture.destination.path(),
        fixture.source.path(),
        std::slice::from_ref(&fixture.recording_id),
        RecoveryMode::DryRun,
        &FakeAudioTools {
            fail_conversion: false,
        },
    )
    .unwrap();

    assert_eq!(
        (
            summary.selected,
            summary.recoverable,
            summary.recovered,
            summary.failed
        ),
        (1, 1, 0, 0)
    );
    assert_eq!(
        fixture
            .ledger
            .verified_recording(&fixture.recording_id)
            .unwrap()
            .unwrap(),
        before
    );
    assert_eq!(fs::read_dir(fixture.destination.path()).unwrap().count(), 0);
}

#[test]
fn apply_recovers_m4a_without_mutating_the_trashed_source() {
    let mut fixture = Fixture::new();
    let source_before = hash_file(&fixture.source_path).unwrap();
    let summary = recover_verified_recordings(
        &mut fixture.ledger,
        fixture.destination.path(),
        fixture.source.path(),
        std::slice::from_ref(&fixture.recording_id),
        RecoveryMode::Apply,
        &FakeAudioTools {
            fail_conversion: false,
        },
    )
    .unwrap();

    assert_eq!(
        (
            summary.selected,
            summary.recoverable,
            summary.recovered,
            summary.failed
        ),
        (1, 1, 1, 0)
    );
    assert_eq!(hash_file(&fixture.source_path).unwrap(), source_before);
    let recovered = fixture
        .ledger
        .verified_recording(&fixture.recording_id)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.artifact.format, OutputFormat::M4a);
    assert_eq!(recovered.retirement_status, RetirementStatus::MovedToTrash);
    assert!(
        recovered
            .artifact
            .relative_path
            .starts_with("DJI Mic Mini 2S")
    );
    assert!(
        fixture
            .destination
            .path()
            .join(recovered.artifact.relative_path)
            .is_file()
    );
}

#[test]
fn mismatched_source_and_overlapping_roots_fail_closed() {
    let mut fixture = Fixture::new();
    fs::write(&fixture.source_path, b"tampered source wav").unwrap();
    let summary = recover_verified_recordings(
        &mut fixture.ledger,
        fixture.destination.path(),
        fixture.source.path(),
        std::slice::from_ref(&fixture.recording_id),
        RecoveryMode::Apply,
        &FakeAudioTools {
            fail_conversion: false,
        },
    )
    .unwrap();
    assert_eq!((summary.recovered, summary.failed), (0, 1));

    assert!(
        recover_verified_recordings(
            &mut fixture.ledger,
            fixture.destination.path(),
            fixture.destination.path(),
            std::slice::from_ref(&fixture.recording_id),
            RecoveryMode::DryRun,
            &FakeAudioTools {
                fail_conversion: false,
            },
        )
        .is_err()
    );
}

#[test]
fn divergent_destination_is_never_overwritten() {
    let mut fixture = Fixture::new();
    let occupied_relative =
        Path::new("DJI Mic Mini 2S/2026/08/260810-T01_MIC001_20260810_155045_edit.wav");
    let occupied = fixture.destination.path().join(occupied_relative);
    fs::create_dir_all(occupied.parent().unwrap()).unwrap();
    fs::write(&occupied, b"unrelated destination bytes").unwrap();

    let summary = recover_verified_recordings(
        &mut fixture.ledger,
        fixture.destination.path(),
        fixture.source.path(),
        std::slice::from_ref(&fixture.recording_id),
        RecoveryMode::Apply,
        &FakeAudioTools {
            fail_conversion: false,
        },
    )
    .unwrap();

    assert_eq!((summary.recovered, summary.failed), (1, 0));
    assert_eq!(fs::read(&occupied).unwrap(), b"unrelated destination bytes");
    let recovered = fixture
        .ledger
        .verified_recording(&fixture.recording_id)
        .unwrap()
        .unwrap();
    assert_ne!(recovered.artifact.relative_path, occupied_relative);
    assert!(
        recovered
            .artifact
            .relative_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains('-')
    );
}

#[test]
fn conversion_failure_leaves_a_verified_resumable_wav() {
    let mut fixture = Fixture::new();
    let first = recover_verified_recordings(
        &mut fixture.ledger,
        fixture.destination.path(),
        fixture.source.path(),
        std::slice::from_ref(&fixture.recording_id),
        RecoveryMode::Apply,
        &FakeAudioTools {
            fail_conversion: true,
        },
    )
    .unwrap();
    assert_eq!((first.recovered, first.failed), (0, 1));
    assert_eq!(
        fixture
            .ledger
            .verified_recording(&fixture.recording_id)
            .unwrap()
            .unwrap()
            .artifact
            .format,
        OutputFormat::Wav
    );

    let resumed = recover_verified_recordings(
        &mut fixture.ledger,
        fixture.destination.path(),
        fixture.source.path(),
        std::slice::from_ref(&fixture.recording_id),
        RecoveryMode::Apply,
        &FakeAudioTools {
            fail_conversion: false,
        },
    )
    .unwrap();
    assert_eq!((resumed.recovered, resumed.failed), (1, 0));
}
