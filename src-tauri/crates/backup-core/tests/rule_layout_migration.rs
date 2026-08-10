use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use backup_core::{
    additional_file::{AdditionalFileClass, VerifiedAdditionalFile},
    artifact::{
        ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact, VerifiedAudioProperties,
    },
    backup::CancellationToken,
    batch::FrozenPreferences,
    deletion::TrashAdapter,
    error::CoreError,
    hash::hash_file,
    layout::{migrate_legacy_rule_layout, migrate_legacy_rule_layout_resilient},
    ledger::{Ledger, VerifiedRecording},
    source::{SourceId, SourceRecord},
};
use tempfile::tempdir;

struct RecordingTrash {
    root: PathBuf,
    moved: Mutex<Vec<PathBuf>>,
}

impl RecordingTrash {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            moved: Mutex::new(Vec::new()),
        }
    }

    fn moved(&self) -> Vec<PathBuf> {
        self.moved.lock().unwrap().clone()
    }
}

impl TrashAdapter for RecordingTrash {
    fn move_to_trash(&self, absolute_path: &Path) -> Result<(), CoreError> {
        let mut moved = self.moved.lock().unwrap();
        let target = self.root.join(format!(
            "{}-{}",
            moved.len(),
            absolute_path
                .file_name()
                .ok_or(CoreError::TrashFailed)?
                .to_string_lossy()
        ));
        fs::rename(absolute_path, target).map_err(|_| CoreError::TrashFailed)?;
        moved.push(absolute_path.to_path_buf());
        Ok(())
    }
}

struct FailFirstTrash<'a> {
    delegate: &'a RecordingTrash,
    failed: AtomicBool,
}

impl TrashAdapter for FailFirstTrash<'_> {
    fn move_to_trash(&self, absolute_path: &Path) -> Result<(), CoreError> {
        if !self.failed.swap(true, Ordering::SeqCst) {
            return Err(CoreError::TrashFailed);
        }
        self.delegate.move_to_trash(absolute_path)
    }
}

fn setup_ledger(destination: &Path) -> (Ledger, SourceId) {
    let mut ledger = Ledger::open(destination.join("ledger.sqlite3")).unwrap();
    let rule = ledger.dji_rule().unwrap();
    let source = SourceRecord {
        id: SourceId::new(),
        rule_id: rule.id,
        volume_uuid: "rule-layout-device".to_owned(),
        legacy_slot: Some("TX01".to_owned()),
        display_name: "DJI Mic Mini 2S (TX01)".to_owned(),
    };
    ledger
        .upsert_source(&source, "2026-08-10T00:00:00Z")
        .unwrap();
    ledger
        .begin_batch_run(
            "layout-run",
            &source.id,
            "2026-08-10T00:00:00Z",
            0,
            FrozenPreferences {
                automatic_backup: true,
                m4a_conversion: true,
                automatic_trash: false,
            },
        )
        .unwrap();
    (ledger, source.id)
}

fn write_artifact(root: &Path, relative: &Path, bytes: &[u8]) -> VerifiedArtifact {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    let digest = hash_file(&path).unwrap();
    VerifiedArtifact {
        relative_path: relative.to_path_buf(),
        format: if relative.extension().unwrap() == "m4a" {
            OutputFormat::M4a
        } else {
            OutputFormat::Wav
        },
        byte_count: digest.size,
        sha256: digest.sha256,
        audio: (relative.extension().unwrap() == "m4a").then(|| VerifiedAudioProperties {
            codec: "aac".to_owned(),
            sample_rate_hz: 48_000,
            channel_count: 1,
            valid_frames: 48_000,
            duration_micros: 1_000_000,
        }),
    }
}

fn recording(
    id: &str,
    source_id: &SourceId,
    source_relative: &str,
    artifact: VerifiedArtifact,
) -> VerifiedRecording {
    VerifiedRecording {
        id: id.to_owned(),
        source_id: source_id.clone(),
        source_relative_path: source_relative.into(),
        source_size: artifact.byte_count,
        source_mtime_ns: 1,
        source_sha256: artifact.sha256.clone(),
        conversion_status: if artifact.format == OutputFormat::M4a {
            ConversionStatus::Complete
        } else {
            ConversionStatus::NotRequired
        },
        conversion_error_code: None,
        retirement_status: RetirementStatus::Present,
        retired_session_relative_path: None,
        verified_at: "2026-08-10T00:00:01Z".to_owned(),
        backup_run_id: "layout-run".to_owned(),
        artifact,
    }
}

#[test]
fn verified_wav_and_m4a_move_under_the_dji_rule_without_losing_evidence() {
    let destination = tempdir().unwrap();
    let trash_directory = tempdir().unwrap();
    let trash = RecordingTrash::new(trash_directory.path().to_path_buf());
    let (mut ledger, source_id) = setup_ledger(destination.path());
    let wav_old = Path::new("2026/08/260810-T01_interview.wav");
    let m4a_old = Path::new("2026/08/260810-T02_interview.m4a");
    let wav = recording(
        "wav-recording",
        &source_id,
        "TX_MIC001/TX01_interview.wav",
        write_artifact(destination.path(), wav_old, b"verified wav"),
    );
    let m4a = recording(
        "m4a-recording",
        &source_id,
        "TX_MIC001/TX02_interview.wav",
        write_artifact(destination.path(), m4a_old, b"verified m4a"),
    );
    ledger.commit_verified_recording(&wav).unwrap();
    ledger.commit_verified_recording(&m4a).unwrap();
    let rule = ledger.dji_rule().unwrap();

    let migrated = migrate_legacy_rule_layout(
        destination.path(),
        &mut ledger,
        &rule,
        &trash,
        &CancellationToken::default(),
    )
    .unwrap();

    assert_eq!(migrated.len(), 2);
    for (id, old, bytes) in [
        ("wav-recording", wav_old, b"verified wav".as_slice()),
        ("m4a-recording", m4a_old, b"verified m4a".as_slice()),
    ] {
        let target = Path::new("DJI Mic Mini 2S").join(old);
        assert_eq!(fs::read(destination.path().join(&target)).unwrap(), bytes);
        assert_eq!(
            ledger
                .verified_recording(id)
                .unwrap()
                .unwrap()
                .artifact
                .relative_path,
            target
        );
        assert!(!destination.path().join(old).exists());
    }
    assert_eq!(trash.moved().len(), 2);
    assert!(ledger.dji_rule().unwrap().archive_directory_locked);
    assert!(!destination.path().join("2026/08").exists());
    assert!(!destination.path().join("2026").exists());
}

#[test]
fn extras_collisions_repairs_and_mismatches_are_safe_and_idempotent() {
    let destination = tempdir().unwrap();
    let trash_directory = tempdir().unwrap();
    let trash = RecordingTrash::new(trash_directory.path().to_path_buf());
    let (mut ledger, source_id) = setup_ledger(destination.path());

    let extra_old = Path::new("source-extras/2026/2026-08-10/TX01/TX_MIC001/notes.txt");
    let extra_artifact = write_artifact(destination.path(), extra_old, b"field notes");
    ledger
        .commit_verified_additional_file(&VerifiedAdditionalFile {
            id: "extra".to_owned(),
            source_id: source_id.clone(),
            source_relative_path: "TX_MIC001/notes.txt".into(),
            source_size: extra_artifact.byte_count,
            source_mtime_ns: 1,
            source_sha256: extra_artifact.sha256.clone(),
            artifact_relative_path: extra_old.to_path_buf(),
            artifact_size: extra_artifact.byte_count,
            artifact_sha256: extra_artifact.sha256,
            classification: AdditionalFileClass::Other,
            backup_run_id: "layout-run".to_owned(),
        })
        .unwrap();

    let same_old = Path::new("2026/08/260810-T01-same.wav");
    let same_artifact = write_artifact(destination.path(), same_old, b"same bytes");
    let same_target = Path::new("DJI Mic Mini 2S").join(same_old);
    write_artifact(destination.path(), &same_target, b"same bytes");
    ledger
        .commit_verified_recording(&recording(
            "same",
            &source_id,
            "TX_MIC001/TX01-same.wav",
            same_artifact,
        ))
        .unwrap();

    let collision_old = Path::new("2026/08/260810-T01-collision.wav");
    let collision_artifact = write_artifact(destination.path(), collision_old, b"source bytes");
    let collision_hash = collision_artifact.sha256.clone();
    let collision_target = Path::new("DJI Mic Mini 2S").join(collision_old);
    write_artifact(destination.path(), &collision_target, b"different bytes");
    ledger
        .commit_verified_recording(&recording(
            "collision",
            &source_id,
            "TX_MIC001/TX01-collision.wav",
            collision_artifact,
        ))
        .unwrap();

    let repair_old = Path::new("2026/08/260810-T01-repair.wav");
    let repair_target = Path::new("DJI Mic Mini 2S").join(repair_old);
    let repair_artifact = write_artifact(destination.path(), &repair_target, b"repair bytes");
    let mut repair_evidence = repair_artifact.clone();
    repair_evidence.relative_path = repair_old.to_path_buf();
    ledger
        .commit_verified_recording(&recording(
            "repair",
            &source_id,
            "TX_MIC001/TX01-repair.wav",
            repair_evidence,
        ))
        .unwrap();

    let mismatch_old = Path::new("2026/08/260810-T01-mismatch.wav");
    let mismatch_artifact = write_artifact(destination.path(), mismatch_old, b"expected bytes");
    ledger
        .commit_verified_recording(&recording(
            "mismatch",
            &source_id,
            "TX_MIC001/TX01-mismatch.wav",
            mismatch_artifact,
        ))
        .unwrap();
    fs::write(destination.path().join(mismatch_old), b"changed bytes").unwrap();

    let rule = ledger.dji_rule().unwrap();
    let migrated = migrate_legacy_rule_layout(
        destination.path(),
        &mut ledger,
        &rule,
        &trash,
        &CancellationToken::default(),
    )
    .unwrap();

    assert_eq!(migrated.len(), 4);
    assert!(
        destination
            .path()
            .join("DJI Mic Mini 2S/source-extras/2026/2026-08-10/TX01/TX_MIC001/notes.txt")
            .is_file()
    );
    assert_eq!(
        ledger
            .verified_additional_file("extra")
            .unwrap()
            .unwrap()
            .artifact_relative_path,
        Path::new("DJI Mic Mini 2S/source-extras/2026/2026-08-10/TX01/TX_MIC001/notes.txt")
    );
    assert_eq!(
        ledger
            .verified_recording("same")
            .unwrap()
            .unwrap()
            .artifact
            .relative_path,
        same_target
    );
    assert_eq!(
        ledger
            .verified_recording("repair")
            .unwrap()
            .unwrap()
            .artifact
            .relative_path,
        repair_target
    );
    let collision_path = ledger
        .verified_recording("collision")
        .unwrap()
        .unwrap()
        .artifact
        .relative_path;
    assert_eq!(
        collision_path,
        Path::new("DJI Mic Mini 2S/2026/08")
            .join(format!("260810-T01-collision-{}.wav", &collision_hash[..8]))
    );
    assert_eq!(
        fs::read(destination.path().join(collision_target)).unwrap(),
        b"different bytes"
    );
    assert_eq!(
        fs::read(destination.path().join(mismatch_old)).unwrap(),
        b"changed bytes"
    );
    assert_eq!(
        ledger
            .verified_recording("mismatch")
            .unwrap()
            .unwrap()
            .artifact
            .relative_path,
        mismatch_old
    );

    assert!(
        migrate_legacy_rule_layout(
            destination.path(),
            &mut ledger,
            &rule,
            &trash,
            &CancellationToken::default(),
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn resilient_startup_migration_reports_one_failure_and_continues_other_artifacts() {
    let destination = tempdir().unwrap();
    let trash_directory = tempdir().unwrap();
    let recording_trash = RecordingTrash::new(trash_directory.path().to_path_buf());
    let trash = FailFirstTrash {
        delegate: &recording_trash,
        failed: AtomicBool::new(false),
    };
    let (mut ledger, source_id) = setup_ledger(destination.path());
    for (id, relative) in [
        ("first", "2026/08/260810-T01-a.wav"),
        ("second", "2026/08/260810-T01-b.wav"),
    ] {
        let artifact = write_artifact(destination.path(), Path::new(relative), id.as_bytes());
        ledger
            .commit_verified_recording(&recording(
                id,
                &source_id,
                &format!("TX_MIC001/{id}.wav"),
                artifact,
            ))
            .unwrap();
    }
    let rule = ledger.dji_rule().unwrap();
    let mut failures = 0;

    let migrated = migrate_legacy_rule_layout_resilient(
        destination.path(),
        &mut ledger,
        &rule,
        &trash,
        &CancellationToken::default(),
        &mut |_| failures += 1,
    )
    .unwrap();

    assert_eq!(failures, 1);
    assert_eq!(migrated.len(), 1);
    assert_eq!(
        ledger
            .verified_recording("first")
            .unwrap()
            .unwrap()
            .artifact
            .relative_path,
        Path::new("2026/08/260810-T01-a.wav")
    );
    assert_eq!(
        ledger
            .verified_recording("second")
            .unwrap()
            .unwrap()
            .artifact
            .relative_path,
        Path::new("DJI Mic Mini 2S/2026/08/260810-T01-b.wav")
    );
}
