use std::{
    fs::{self, File, FileTimes},
    path::{Path, PathBuf},
    time::{Duration, UNIX_EPOCH},
};

use backup_core::{
    clock::Clock,
    error::CoreError,
    rule::{BackupRule, DeviceConstraintProfile, FilenameProfile, RuleId, compile_rule},
    rule_scanner::{MAX_SCAN_DEPTH, SelectedFileKind, scan_rule_once, scan_rule_stable},
};
use tempfile::tempdir;
use time::{OffsetDateTime, UtcOffset, macros::date};

fn zoom_rule() -> BackupRule {
    BackupRule {
        id: RuleId::new(),
        name: "Zoom H1n".to_owned(),
        archive_directory_name: "Zoom H1n".to_owned(),
        enabled: true,
        volume_name_glob: "ZOOM_*".to_owned(),
        required_path_globs: vec!["RECORD/**".to_owned()],
        backup_file_globs: vec!["RECORD/**/*.WAV".to_owned(), "RECORD/**/*.txt".to_owned()],
        session_directory_globs: vec!["RECORD/FOLDER*".to_owned()],
        filename_prefix: "zoom-".to_owned(),
        filename_suffix: "-field".to_owned(),
        date_folder_layout: Default::default(),
        filename_profile: FilenameProfile::Preserve,
        device_constraint_profile: DeviceConstraintProfile::GenericExternal,
        preset_kind: None,
        preset_revision: None,
        archive_directory_locked: false,
        archived_at: None,
        created_at: "2026-08-10T00:00:00Z".to_owned(),
        updated_at: "2026-08-10T00:00:00Z".to_owned(),
    }
}

fn set_modified(path: &Path, timestamp: i64) {
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(
            FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(timestamp as u64)),
        )
        .unwrap();
}

fn create_zoom_fixture(root: &Path) -> PathBuf {
    let session = root.join("RECORD/FOLDER01");
    fs::create_dir_all(&session).unwrap();
    let wav = session.join("ZOOM0001.WAV");
    fs::write(&wav, b"zoom audio").unwrap();
    fs::write(session.join("notes.txt"), b"field notes").unwrap();
    fs::write(session.join("unrelated.bin"), b"not selected").unwrap();
    fs::create_dir(root.join(".Trashes")).unwrap();
    fs::write(root.join(".Trashes/ignored.WAV"), b"trash").unwrap();
    let timestamp =
        OffsetDateTime::new_utc(date!(2026 - 08 - 10), time::Time::MIDNIGHT).unix_timestamp();
    set_modified(&wav, timestamp);
    wav
}

#[test]
fn bounded_rule_scan_selects_only_matching_safe_regular_files() {
    let source = tempdir().unwrap();
    create_zoom_fixture(source.path());

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let outside = tempdir().unwrap();
        let target = outside.path().join("outside.WAV");
        fs::write(&target, b"outside").unwrap();
        symlink(&target, source.path().join("RECORD/FOLDER01/linked.WAV")).unwrap();
    }

    let rule = compile_rule(zoom_rule()).unwrap();
    let result = scan_rule_once(source.path(), &rule, UtcOffset::UTC).unwrap();

    assert_eq!(result.files.len(), 2);
    assert_eq!(
        result.files[0].relative_path,
        Path::new("RECORD/FOLDER01/ZOOM0001.WAV")
    );
    assert_eq!(result.files[0].kind, SelectedFileKind::RecordingWav);
    assert_eq!(result.files[0].archive_date, date!(2026 - 08 - 10));
    assert_eq!(
        result.files[0].session_relative_path.as_deref(),
        Some(Path::new("RECORD/FOLDER01"))
    );
    assert_eq!(
        result.files[1].relative_path,
        Path::new("RECORD/FOLDER01/notes.txt")
    );
    assert_eq!(result.files[1].kind, SelectedFileKind::Companion);
    assert_eq!(
        result.files[1].session_relative_path.as_deref(),
        Some(Path::new("RECORD/FOLDER01"))
    );
    assert!(result.unsafe_session_count >= 1);
    assert!(
        result
            .files
            .iter()
            .all(|file| !file.relative_path.starts_with(".Trashes"))
    );
}

#[test]
fn scan_limit_rejects_a_tree_deeper_than_the_public_bound() {
    let source = tempdir().unwrap();
    let mut directory = source.path().to_path_buf();
    for index in 0..=MAX_SCAN_DEPTH {
        directory.push(format!("level-{index}"));
        fs::create_dir(&directory).unwrap();
    }

    let error = scan_rule_once(
        source.path(),
        &compile_rule(zoom_rule()).unwrap(),
        UtcOffset::UTC,
    )
    .unwrap_err();

    assert!(matches!(error, CoreError::RuleScanLimit));
}

struct MutatingClock {
    path: PathBuf,
}

impl Clock for MutatingClock {
    fn sleep(&self, duration: Duration) {
        assert_eq!(duration, Duration::from_secs(2));
        fs::write(&self.path, b"zoom audio changed between scans").unwrap();
    }
}

#[test]
fn stable_scan_excludes_a_file_changed_between_observations() {
    let source = tempdir().unwrap();
    let wav = create_zoom_fixture(source.path());
    let result = scan_rule_stable(
        source.path(),
        &compile_rule(zoom_rule()).unwrap(),
        UtcOffset::UTC,
        &MutatingClock { path: wav },
    )
    .unwrap();

    assert_eq!(result.files.len(), 1);
    assert_eq!(
        result.files[0].relative_path,
        Path::new("RECORD/FOLDER01/notes.txt")
    );
    assert_eq!(result.unsafe_session_count, 1);
}

#[test]
fn nested_session_directories_are_unsafe_but_do_not_block_safe_backup() {
    let source = tempdir().unwrap();
    create_zoom_fixture(source.path());
    let nested = source.path().join("RECORD/FOLDER01/nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("hidden.WAV"), b"must not be selected").unwrap();

    let result = scan_rule_once(
        source.path(),
        &compile_rule(zoom_rule()).unwrap(),
        UtcOffset::UTC,
    )
    .unwrap();

    assert_eq!(result.files.len(), 2);
    assert_eq!(result.unsafe_session_count, 2);
    assert!(
        result
            .files
            .iter()
            .all(|file| !file.relative_path.starts_with("RECORD/FOLDER01/nested"))
    );
}

#[test]
fn dji_profile_keeps_the_encoded_recording_date_authoritative() {
    let source = tempdir().unwrap();
    let path = source.path().join("TX01_MIC001_20260809_010203.WAV");
    fs::write(&path, b"dji audio").unwrap();
    let old_timestamp =
        OffsetDateTime::new_utc(date!(2024 - 01 - 02), time::Time::MIDNIGHT).unix_timestamp();
    set_modified(&path, old_timestamp);
    let mut rule = zoom_rule();
    rule.required_path_globs.clear();
    rule.backup_file_globs = vec!["*.WAV".to_owned()];
    rule.session_directory_globs.clear();
    rule.filename_profile = FilenameProfile::DjiTxShort;

    let result =
        scan_rule_once(source.path(), &compile_rule(rule).unwrap(), UtcOffset::UTC).unwrap();

    assert_eq!(result.files[0].archive_date, date!(2026 - 08 - 09));
}
