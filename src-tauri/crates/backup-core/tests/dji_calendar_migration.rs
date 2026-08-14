use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use backup_core::{
    artifact::{
        ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact, VerifiedAudioProperties,
    },
    backup::CancellationToken,
    batch::FrozenPreferences,
    deletion::TrashAdapter,
    error::CoreError,
    hash::hash_file,
    layout::{
        migrate_verified_dji_calendar_layout, migrate_verified_dji_calendar_layout_resilient,
    },
    ledger::{Ledger, VerifiedRecording},
    rule::{BackupRuleDraft, DateFolderLayout},
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

struct Fixture {
    ledger: Ledger,
    dji_source: SourceId,
    generic_source: SourceId,
}

impl Fixture {
    fn new(root: &Path) -> Self {
        let mut ledger = Ledger::open(root.join("ledger.sqlite3")).unwrap();
        let dji_rule = ledger.dji_rule().unwrap();
        let generic_rule = ledger
            .save_backup_rule(
                BackupRuleDraft {
                    id: None,
                    name: "Zoom H1n".to_owned(),
                    archive_directory_name: "Zoom H1n".to_owned(),
                    enabled: true,
                    volume_name_glob: "ZOOM_*".to_owned(),
                    required_path_globs: vec!["RECORD/**".to_owned()],
                    backup_file_globs: vec!["RECORD/**/*.WAV".to_owned()],
                    session_directory_globs: vec!["RECORD/FOLDER*".to_owned()],
                    filename_prefix: String::new(),
                    filename_suffix: String::new(),
                    date_folder_layout: DateFolderLayout::YearMonth,
                },
                "2026-08-14T00:00:00Z",
            )
            .unwrap();
        let dji_source = SourceId::new();
        let generic_source = SourceId::new();
        for (source, rule_id, volume_uuid, display_name) in [
            (
                &dji_source,
                dji_rule.id,
                "dji-calendar-source",
                "DJI Mic Mini 2S (TX01)",
            ),
            (
                &generic_source,
                generic_rule.id,
                "zoom-calendar-source",
                "Zoom H1n",
            ),
        ] {
            ledger
                .upsert_source(
                    &SourceRecord {
                        id: source.clone(),
                        rule_id,
                        volume_uuid: volume_uuid.to_owned(),
                        legacy_slot: None,
                        display_name: display_name.to_owned(),
                    },
                    "2026-08-14T00:00:00Z",
                )
                .unwrap();
        }
        for (run_id, source) in [("dji-run", &dji_source), ("generic-run", &generic_source)] {
            ledger
                .begin_batch_run(
                    run_id,
                    source,
                    "2026-08-14T00:00:00Z",
                    0,
                    FrozenPreferences {
                        automatic_backup: true,
                        m4a_conversion: true,
                        automatic_trash: false,
                    },
                )
                .unwrap();
        }
        Self {
            ledger,
            dji_source,
            generic_source,
        }
    }

    fn commit(
        &mut self,
        root: &Path,
        id: &str,
        source_id: &SourceId,
        source_relative: &str,
        artifact_relative: &str,
        bytes: &[u8],
    ) -> VerifiedRecording {
        let artifact = write_artifact(root, Path::new(artifact_relative), bytes);
        let recording = recording(
            id,
            source_id,
            source_relative,
            if source_id == &self.dji_source {
                "dji-run"
            } else {
                "generic-run"
            },
            artifact,
        );
        self.ledger.commit_verified_recording(&recording).unwrap();
        recording
    }
}

fn write_artifact(root: &Path, relative: &Path, bytes: &[u8]) -> VerifiedArtifact {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap();
    let digest = hash_file(&path).unwrap();
    let format = if relative
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("m4a"))
    {
        OutputFormat::M4a
    } else {
        OutputFormat::Wav
    };
    VerifiedArtifact {
        relative_path: relative.to_path_buf(),
        format,
        byte_count: digest.size,
        sha256: digest.sha256,
        audio: (format == OutputFormat::M4a).then(|| VerifiedAudioProperties {
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
    run_id: &str,
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
        verified_at: "2026-08-14T00:00:01Z".to_owned(),
        backup_run_id: run_id.to_owned(),
        artifact,
    }
}

#[test]
fn migrates_every_verified_dji_legacy_shape_into_one_calendar_month() {
    let destination = tempdir().unwrap();
    let trash_root = tempdir().unwrap();
    let trash = RecordingTrash::new(trash_root.path().to_path_buf());
    let mut fixture = Fixture::new(destination.path());
    let source = fixture.dji_source.clone();
    let cases = [
        (
            "day-directory",
            "TX_MIC001/TX01_MIC001_20260814_010203_edit.wav",
            "DJI Mic Mini 2S/2026/08/14/260814-T01_MIC001_20260814_010203_edit.m4a",
            "2026/08/260814-T01_MIC001_20260814_010203_edit.m4a",
        ),
        (
            "month-directory",
            "TX_MIC002/TX02_MIC002_20260814_010204_edit.wav",
            "DJI Mic Mini 2S/2026/08/260814-T02_MIC002_20260814_010204_edit.m4a",
            "2026/08/260814-T02_MIC002_20260814_010204_edit.m4a",
        ),
        (
            "dated-transmitter-directory",
            "TX_MIC003/TX01_MIC003_20260814_010205_edit.wav",
            "2026/2026-08-14/TX01/TX01_MIC003_20260814_010205_edit.m4a",
            "2026/08/260814-T01_MIC003_20260814_010205_edit.m4a",
        ),
        (
            "dated-directory",
            "TX_MIC004/TX02_MIC004_20260814_010206_edit.wav",
            "2026/2026-08-14/TX02_MIC004_20260814_010206_edit.wav",
            "2026/08/260814-T02_MIC004_20260814_010206_edit.wav",
        ),
    ];
    for (id, source_relative, old, _) in cases {
        fixture.commit(
            destination.path(),
            id,
            &source,
            source_relative,
            old,
            id.as_bytes(),
        );
    }

    let migrated = migrate_verified_dji_calendar_layout(
        destination.path(),
        &mut fixture.ledger,
        &trash,
        &CancellationToken::default(),
    )
    .unwrap();

    assert_eq!(migrated.len(), cases.len());
    for (id, _, old, expected) in cases {
        assert!(!destination.path().join(old).exists());
        assert_eq!(
            fixture
                .ledger
                .verified_recording(id)
                .unwrap()
                .unwrap()
                .artifact
                .relative_path,
            Path::new(expected)
        );
        assert_eq!(
            fs::read(destination.path().join(expected)).unwrap(),
            id.as_bytes()
        );
    }
    assert_eq!(trash.moved().len(), cases.len());
    assert!(!destination.path().join("DJI Mic Mini 2S").exists());
    assert!(!destination.path().join("2026/2026-08-14").exists());
    assert!(
        migrate_verified_dji_calendar_layout(
            destination.path(),
            &mut fixture.ledger,
            &trash,
            &CancellationToken::default(),
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn reuses_equal_content_suffixes_collisions_and_leaves_generic_rules_untouched() {
    let destination = tempdir().unwrap();
    let trash_root = tempdir().unwrap();
    let trash = RecordingTrash::new(trash_root.path().to_path_buf());
    let mut fixture = Fixture::new(destination.path());
    let dji_source = fixture.dji_source.clone();
    let generic_source = fixture.generic_source.clone();

    let equal_old = "DJI Mic Mini 2S/2026/08/14/equal.m4a";
    let equal_target = "2026/08/260814-T01_MIC005_20260814_010207_edit.m4a";
    let equal = fixture.commit(
        destination.path(),
        "equal",
        &dji_source,
        "TX01_MIC005_20260814_010207_edit.wav",
        equal_old,
        b"equal bytes",
    );
    write_artifact(destination.path(), Path::new(equal_target), b"equal bytes");

    let collision_old = "DJI Mic Mini 2S/2026/08/14/collision.m4a";
    let collision_default = "2026/08/260814-T02_MIC006_20260814_010208_edit.m4a";
    let collision = fixture.commit(
        destination.path(),
        "collision",
        &dji_source,
        "TX02_MIC006_20260814_010208_edit.wav",
        collision_old,
        b"collision source",
    );
    write_artifact(
        destination.path(),
        Path::new(collision_default),
        b"different bytes",
    );

    let generic_old = "Zoom H1n/2026/08/260814-ZOOM0001.wav";
    fixture.commit(
        destination.path(),
        "generic",
        &generic_source,
        "RECORD/FOLDER01/ZOOM0001.WAV",
        generic_old,
        b"generic bytes",
    );

    let migrated = migrate_verified_dji_calendar_layout(
        destination.path(),
        &mut fixture.ledger,
        &trash,
        &CancellationToken::default(),
    )
    .unwrap();

    assert_eq!(migrated.len(), 2);
    assert_eq!(
        fixture
            .ledger
            .verified_recording("equal")
            .unwrap()
            .unwrap()
            .artifact
            .relative_path,
        Path::new(equal_target)
    );
    assert_eq!(
        fs::read(destination.path().join(equal_target)).unwrap(),
        b"equal bytes"
    );
    let collision_target = Path::new("2026/08").join(format!(
        "260814-T02_MIC006_20260814_010208_edit-{}.m4a",
        &collision.artifact.sha256[..8]
    ));
    assert_eq!(
        fixture
            .ledger
            .verified_recording("collision")
            .unwrap()
            .unwrap()
            .artifact
            .relative_path,
        collision_target
    );
    assert_eq!(
        fs::read(destination.path().join(collision_default)).unwrap(),
        b"different bytes"
    );
    assert_eq!(
        fixture
            .ledger
            .verified_recording("generic")
            .unwrap()
            .unwrap()
            .artifact
            .relative_path,
        Path::new(generic_old)
    );
    assert_eq!(
        fs::read(destination.path().join(generic_old)).unwrap(),
        b"generic bytes"
    );
    assert_eq!(equal.artifact.sha256.len(), 64);
    assert_eq!(trash.moved().len(), 2);
}

#[test]
fn refuses_tampered_symlinked_and_cancelled_migrations() {
    let destination = tempdir().unwrap();
    let trash_root = tempdir().unwrap();
    let trash = RecordingTrash::new(trash_root.path().to_path_buf());
    let mut fixture = Fixture::new(destination.path());
    let source = fixture.dji_source.clone();
    let tampered_old = "DJI Mic Mini 2S/2026/08/14/tampered.m4a";
    fixture.commit(
        destination.path(),
        "tampered",
        &source,
        "TX01_MIC007_20260814_010209_edit.wav",
        tampered_old,
        b"expected bytes",
    );
    fs::write(destination.path().join(tampered_old), b"changed bytes").unwrap();

    let symlink_old = "DJI Mic Mini 2S/2026/08/14/symlink.m4a";
    fixture.commit(
        destination.path(),
        "symlink",
        &source,
        "TX02_MIC008_20260814_010210_edit.wav",
        symlink_old,
        b"symlink bytes",
    );
    let symlink_target = destination.path().join("symlink-target.m4a");
    fs::write(&symlink_target, b"symlink bytes").unwrap();
    fs::remove_file(destination.path().join(symlink_old)).unwrap();
    std::os::unix::fs::symlink(&symlink_target, destination.path().join(symlink_old)).unwrap();

    let migrated = migrate_verified_dji_calendar_layout(
        destination.path(),
        &mut fixture.ledger,
        &trash,
        &CancellationToken::default(),
    )
    .unwrap();

    assert!(migrated.is_empty());
    assert_eq!(trash.moved().len(), 0);
    for (id, old) in [("tampered", tampered_old), ("symlink", symlink_old)] {
        assert_eq!(
            fixture
                .ledger
                .verified_recording(id)
                .unwrap()
                .unwrap()
                .artifact
                .relative_path,
            Path::new(old)
        );
    }

    let cancellation = CancellationToken::default();
    cancellation.cancel();
    let mut reported_failures = 0;
    assert!(matches!(
        migrate_verified_dji_calendar_layout_resilient(
            destination.path(),
            &mut fixture.ledger,
            &trash,
            &cancellation,
            &mut |_| reported_failures += 1,
        ),
        Err(CoreError::Cancelled)
    ));
    assert_eq!(reported_failures, 0);
}
