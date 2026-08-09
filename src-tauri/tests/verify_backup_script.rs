use std::{fs, path::Path, process::Command};

use backup_core::{
    additional_file::{AdditionalFileClass, VerifiedAdditionalFile},
    artifact::{ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact},
    backup::CancellationToken,
    batch::{BatchPhase, FrozenPreferences, M4A_PROFILE_ID},
    filesystem::modified_nanos,
    hash::hash_file,
    ledger::{Ledger, VerifiedRecording},
    state::Transmitter,
};
use dji_mic_backup_lib::{
    artifact_pipeline::{finalize_prepared_m4a, prepare_m4a},
    platform::macos::audio::AppleAudioTools,
};
use tempfile::tempdir;

const RUN_ID: &str = "verify-script-run";
const SESSION: &str = "TX_MIC001_20260810_010203";

#[test]
fn independent_verifier_checks_complete_cohort_raw_extras_and_privacy_safe_failures() {
    let fixture = tempdir().unwrap();
    let source = fixture.path().join("source");
    let destination = fixture.path().join("destination");
    let session = source.join(SESSION);
    fs::create_dir_all(&session).unwrap();
    fs::create_dir_all(&destination).unwrap();

    let first_source = session.join("TX01_MIC001_20260810_010203.wav");
    let second_source = session.join("TX01_MIC001_20260810_010204.wav");
    write_pcm_wav(&first_source, 48_000, 1, 48_000, 17);
    write_pcm_wav(&second_source, 48_000, 1, 48_000, 29);
    let external_m4a = session.join("recorder-preview.m4a");
    let apple_double = session.join("._TX01_MIC001_20260810_010203.wav");
    fs::write(&external_m4a, b"independent recorder m4a bytes").unwrap();
    fs::write(&apple_double, b"appledouble metadata bytes").unwrap();

    let ledger_path = fixture.path().join("ledger.sqlite3");
    let mut ledger = Ledger::open(&ledger_path).unwrap();
    let required_bytes = [&first_source, &second_source, &external_m4a, &apple_double]
        .into_iter()
        .map(|path| fs::metadata(path).unwrap().len())
        .sum();
    ledger
        .begin_batch_run(
            RUN_ID,
            "2026-08-10T00:00:00Z",
            required_bytes,
            FrozenPreferences {
                automatic_backup: true,
                m4a_conversion: true,
                automatic_trash: false,
            },
        )
        .unwrap();
    ledger
        .advance_batch_phase(RUN_ID, BatchPhase::Copying)
        .unwrap();

    let mut wav_recordings = Vec::new();
    for (index, source_path) in [first_source, second_source].iter().enumerate() {
        let source_relative = source_path.strip_prefix(&source).unwrap().to_path_buf();
        let destination_relative = Path::new("2026/2026-08-10/TX01")
            .join(SESSION)
            .join(source_path.file_name().unwrap());
        let destination_path = destination.join(&destination_relative);
        fs::create_dir_all(destination_path.parent().unwrap()).unwrap();
        fs::copy(source_path, &destination_path).unwrap();
        let source_digest = hash_file(source_path).unwrap();
        let destination_digest = hash_file(&destination_path).unwrap();
        assert_eq!(source_digest, destination_digest);
        let recording = VerifiedRecording {
            id: format!("recording-{index}"),
            transmitter: Transmitter::Tx01,
            source_relative_path: source_relative,
            source_size: source_digest.size,
            source_mtime_ns: modified_nanos(&fs::metadata(source_path).unwrap()).unwrap(),
            source_sha256: source_digest.sha256.clone(),
            artifact: VerifiedArtifact {
                relative_path: destination_relative,
                format: OutputFormat::Wav,
                byte_count: destination_digest.size,
                sha256: destination_digest.sha256,
                audio: None,
            },
            conversion_status: ConversionStatus::NotRequired,
            conversion_error_code: None,
            retirement_status: RetirementStatus::Present,
            retired_session_relative_path: None,
            verified_at: "2026-08-10T00:01:00Z".to_owned(),
            backup_run_id: RUN_ID.to_owned(),
        };
        ledger.commit_verified_recording(&recording).unwrap();
        wav_recordings.push(recording);
    }

    let additional_artifacts = [
        commit_additional(
            &source,
            &destination,
            &mut ledger,
            &external_m4a,
            "additional-m4a",
            AdditionalFileClass::M4a,
        ),
        commit_additional(
            &source,
            &destination,
            &mut ledger,
            &apple_double,
            "additional-apple-double",
            AdditionalFileClass::AppleDouble,
        ),
    ];

    ledger
        .advance_batch_phase(RUN_ID, BatchPhase::CopiesVerified)
        .unwrap();
    let recording_ids = wav_recordings
        .iter()
        .map(|recording| recording.id.clone())
        .collect::<Vec<_>>();
    ledger
        .begin_conversion_cohort(RUN_ID, &recording_ids, M4A_PROFILE_ID)
        .unwrap();

    let mut prepared = Vec::new();
    for recording in &wav_recordings {
        let artifact = prepare_m4a(
            &destination,
            recording,
            &AppleAudioTools,
            &CancellationToken::default(),
            &mut |_| {},
        )
        .unwrap();
        ledger
            .replace_verified_artifact_with_superseded_wav(
                &artifact.recording,
                &artifact.superseded_wav_relative_path,
                artifact.superseded_wav_size,
                &artifact.superseded_wav_sha256,
            )
            .unwrap();
        ledger
            .mark_conversion_item_verified(RUN_ID, &recording.id)
            .unwrap();
        prepared.push(artifact);
    }
    ledger.commit_m4a_barrier(RUN_ID).unwrap();
    for artifact in &prepared {
        finalize_prepared_m4a(artifact).unwrap();
    }
    ledger
        .finish_backup_run(RUN_ID, "2026-08-10T00:02:00Z", "completed", None)
        .unwrap();
    drop(ledger);

    let success = run_verifier(&ledger_path, &destination, &source);
    assert!(
        success.status.success(),
        "{}",
        String::from_utf8_lossy(&success.stderr)
    );
    let stdout = String::from_utf8_lossy(&success.stdout);
    assert!(stdout.contains("Live source WAV files: 2"));
    assert!(stdout.contains("Live additional files: 2"));
    assert!(stdout.contains("Verified 128kbps-profile M4A artifacts: 2"));
    assert!(stdout.contains("Verified raw additional-file artifacts: 2"));

    let apple_double_artifact = destination.join(&additional_artifacts[1]);
    let held_apple_double = fixture.path().join("held-apple-double");
    fs::rename(&apple_double_artifact, &held_apple_double).unwrap();
    assert_private_failure(
        run_verifier(&ledger_path, &destination, &source),
        fixture.path(),
        "additional-file",
    );
    fs::rename(&held_apple_double, &apple_double_artifact).unwrap();

    let m4a_path = destination.join(&prepared[0].recording.artifact.relative_path);
    let original_m4a = fs::read(&m4a_path).unwrap();
    fs::write(&m4a_path, b"changed m4a artifact").unwrap();
    assert_private_failure(
        run_verifier(&ledger_path, &destination, &source),
        fixture.path(),
        "artifact",
    );
    fs::write(&m4a_path, original_m4a).unwrap();

    let missing_barrier_ledger = fixture.path().join("missing-barrier.sqlite3");
    fs::copy(&ledger_path, &missing_barrier_ledger).unwrap();
    let update = Command::new("/usr/bin/sqlite3")
        .arg(&missing_barrier_ledger)
        .arg("UPDATE backup_runs SET batch_phase = 'converting' WHERE id = 'verify-script-run';")
        .output()
        .unwrap();
    assert!(update.status.success());
    assert_private_failure(
        run_verifier(&missing_barrier_ledger, &destination, &source),
        fixture.path(),
        "barrier",
    );
}

fn commit_additional(
    source_root: &Path,
    destination_root: &Path,
    ledger: &mut Ledger,
    source_path: &Path,
    id: &str,
    classification: AdditionalFileClass,
) -> std::path::PathBuf {
    let source_relative = source_path.strip_prefix(source_root).unwrap().to_path_buf();
    let artifact_relative = Path::new("source-extras/2026/2026-08-10/TX01")
        .join(SESSION)
        .join(source_path.file_name().unwrap());
    let artifact_path = destination_root.join(&artifact_relative);
    fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
    fs::copy(source_path, &artifact_path).unwrap();
    let source_digest = hash_file(source_path).unwrap();
    let artifact_digest = hash_file(&artifact_path).unwrap();
    assert_eq!(source_digest, artifact_digest);
    ledger
        .commit_verified_additional_file(&VerifiedAdditionalFile {
            id: id.to_owned(),
            transmitter: Transmitter::Tx01,
            source_relative_path: source_relative,
            source_size: source_digest.size,
            source_mtime_ns: modified_nanos(&fs::metadata(source_path).unwrap()).unwrap(),
            source_sha256: source_digest.sha256,
            artifact_relative_path: artifact_relative.clone(),
            artifact_size: artifact_digest.size,
            artifact_sha256: artifact_digest.sha256,
            classification,
            backup_run_id: RUN_ID.to_owned(),
        })
        .unwrap();
    artifact_relative
}

fn run_verifier(ledger: &Path, destination: &Path, source: &Path) -> std::process::Output {
    let project_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    Command::new(project_root.join("scripts/verify-backup.sh"))
        .arg("--ledger")
        .arg(ledger)
        .arg(destination)
        .arg(format!("TX01={}", source.display()))
        .output()
        .unwrap()
}

fn assert_private_failure(output: std::process::Output, fixture: &Path, expected: &str) {
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.to_ascii_lowercase().contains(expected), "{stderr}");
    assert!(!stderr.contains(&fixture.display().to_string()));
    assert!(!stderr.contains("TX01_MIC001_20260810_010203.wav"));
    assert!(!stderr.contains("source_sha256"));
}

fn write_pcm_wav(path: &Path, sample_rate: u32, channels: u16, frames: u32, pattern: i16) {
    let bits_per_sample = 16_u16;
    let block_align = channels * (bits_per_sample / 8);
    let byte_rate = sample_rate * u32::from(block_align);
    let data_size = frames * u32::from(block_align);
    let mut wav = Vec::with_capacity(44 + usize::try_from(data_size).unwrap());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    for frame in 0..frames {
        let sample = (((frame % 200) as i16) - 100) * pattern;
        for _ in 0..channels {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
    }
    fs::write(path, wav).unwrap();
}
