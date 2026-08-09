use std::{fs, path::Path};

use backup_core::{
    artifact::{
        ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact, VerifiedAudioProperties,
    },
    backup::CancellationToken,
    batch::FrozenPreferences,
    deletion::TrashAdapter,
    error::CoreError,
    hash::hash_file,
    layout::flatten_verified_recording_layout,
    ledger::{Ledger, VerifiedRecording},
    state::Transmitter,
};
use tempfile::tempdir;

struct MovingTrash<'a> {
    root: &'a Path,
}

impl TrashAdapter for MovingTrash<'_> {
    fn move_to_trash(&self, absolute_path: &Path) -> Result<(), CoreError> {
        let target = self
            .root
            .join(absolute_path.file_name().ok_or(CoreError::TrashFailed)?);
        fs::rename(absolute_path, target).map_err(|_| CoreError::TrashFailed)
    }
}

#[test]
fn verified_transmitter_artifact_moves_into_the_shared_date_folder() {
    let destination = tempdir().unwrap();
    let trash = tempdir().unwrap();
    let old_relative =
        Path::new("2026/2026-08-10/TX02").join("TX02_MIC001_20260810_004713_edit.m4a");
    let flat_relative = Path::new("2026/2026-08-10/TX02_MIC001_20260810_004713_edit.m4a");
    let old_path = destination.path().join(&old_relative);
    fs::create_dir_all(old_path.parent().unwrap()).unwrap();
    fs::write(&old_path, b"verified m4a bytes").unwrap();
    let artifact_digest = hash_file(&old_path).unwrap();

    let mut ledger = Ledger::open(destination.path().join("ledger.sqlite3")).unwrap();
    ledger
        .begin_batch_run(
            "run-1",
            "2026-08-10T00:00:00Z",
            0,
            FrozenPreferences {
                automatic_backup: true,
                m4a_conversion: true,
                automatic_trash: false,
            },
        )
        .unwrap();
    ledger
        .commit_verified_recording(&VerifiedRecording {
            id: "recording-1".to_owned(),
            transmitter: Transmitter::Tx02,
            source_relative_path: "TX_MIC001_20260810_004713/TX02_MIC001_20260810_004713_edit.wav"
                .into(),
            source_size: 67_940_296,
            source_mtime_ns: 1,
            source_sha256: "a".repeat(64),
            artifact: VerifiedArtifact {
                relative_path: old_relative.clone(),
                format: OutputFormat::M4a,
                byte_count: artifact_digest.size,
                sha256: artifact_digest.sha256.clone(),
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
            retirement_status: RetirementStatus::Present,
            retired_session_relative_path: None,
            verified_at: "2026-08-10T00:00:01Z".to_owned(),
            backup_run_id: "run-1".to_owned(),
        })
        .unwrap();

    let migrated = flatten_verified_recording_layout(
        destination.path(),
        &mut ledger,
        &MovingTrash { root: trash.path() },
        &CancellationToken::default(),
    )
    .unwrap();

    assert_eq!(migrated.len(), 1);
    assert_eq!(migrated[0].from, old_relative);
    assert_eq!(migrated[0].to, flat_relative);
    assert_eq!(
        fs::read(destination.path().join(flat_relative)).unwrap(),
        b"verified m4a bytes"
    );
    assert!(!old_path.exists());
    assert!(!old_path.parent().unwrap().exists());
    assert_eq!(
        fs::read(trash.path().join(old_path.file_name().unwrap())).unwrap(),
        b"verified m4a bytes"
    );
    assert_eq!(
        ledger
            .verified_recording("recording-1")
            .unwrap()
            .unwrap()
            .artifact
            .relative_path,
        Path::new("2026/2026-08-10/TX02_MIC001_20260810_004713_edit.m4a")
    );
}
