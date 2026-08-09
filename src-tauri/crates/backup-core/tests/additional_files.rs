use std::{fs, path::Path};

use backup_core::{
    additional_file::AdditionalFileClass,
    backup::{BackupItemContext, CancellationToken, execute_additional_file_copy},
    destination::{DestinationDisposition, plan_additional_file},
    error::CoreError,
    recording::AdditionalFileObservation,
    scanner::{ScanIssue, scan_once},
    state::Transmitter,
};
use tempfile::tempdir;
use time::UtcOffset;

fn create_observed_session(root: &Path) {
    let session = root.join("TX_MIC001_20260810_001116");
    fs::create_dir(&session).unwrap();
    fs::write(
        session.join("TX01_MIC001_20260810_001116.wav"),
        b"first wav",
    )
    .unwrap();
    fs::write(
        session.join("TX01_MIC002_20260810_001117.wav"),
        b"second wav",
    )
    .unwrap();
    fs::write(
        session.join("TX01_MIC001_20260810_001116.m4a"),
        b"external m4a",
    )
    .unwrap();
    fs::write(
        session.join("._TX01_MIC001_20260810_001116.m4a"),
        b"apple double",
    )
    .unwrap();
}

#[test]
fn recognized_session_inventories_wavs_m4a_and_appledouble_in_order() {
    let source = tempdir().unwrap();
    create_observed_session(source.path());

    let scan = scan_once(source.path(), Transmitter::Tx01, UtcOffset::UTC).unwrap();

    assert_eq!(scan.recordings.len(), 2);
    assert_eq!(scan.additional_files.len(), 2);
    assert_eq!(
        scan.additional_files[0].relative_path,
        Path::new("TX_MIC001_20260810_001116/._TX01_MIC001_20260810_001116.m4a")
    );
    assert_eq!(
        scan.additional_files[0].classification,
        AdditionalFileClass::AppleDouble
    );
    assert_eq!(
        scan.additional_files[1].relative_path,
        Path::new("TX_MIC001_20260810_001116/TX01_MIC001_20260810_001116.m4a")
    );
    assert_eq!(
        scan.additional_files[1].classification,
        AdditionalFileClass::M4a
    );
}

#[test]
fn recognized_session_preserves_hidden_wavs_and_other_regular_files_as_additional() {
    let source = tempdir().unwrap();
    create_observed_session(source.path());
    let session = source.path().join("TX_MIC001_20260810_001116");
    fs::write(session.join(".hidden.wav"), b"hidden wav").unwrap();
    fs::write(session.join("notes.bin"), b"metadata").unwrap();

    let scan = scan_once(source.path(), Transmitter::Tx01, UtcOffset::UTC).unwrap();

    assert_eq!(scan.recordings.len(), 2);
    assert!(scan.additional_files.iter().any(|file| {
        file.file_name == ".hidden.wav" && file.classification == AdditionalFileClass::Other
    }));
    assert!(scan.additional_files.iter().any(|file| {
        file.file_name == "notes.bin" && file.classification == AdditionalFileClass::Other
    }));
}

#[cfg(unix)]
#[test]
fn recognized_session_reports_symlinks_and_nested_directories_as_unsafe() {
    use std::os::unix::fs::symlink;

    let source = tempdir().unwrap();
    let outside = tempdir().unwrap();
    create_observed_session(source.path());
    let session = source.path().join("TX_MIC001_20260810_001116");
    fs::write(outside.path().join("outside.m4a"), b"outside").unwrap();
    symlink(
        outside.path().join("outside.m4a"),
        session.join("linked.m4a"),
    )
    .unwrap();
    fs::create_dir(session.join("nested")).unwrap();
    fs::write(session.join("nested/inside.m4a"), b"inside").unwrap();

    let scan = scan_once(source.path(), Transmitter::Tx01, UtcOffset::UTC).unwrap();

    assert!(scan.issues.contains(&ScanIssue::UnsafeSessionEntry));
    assert_eq!(scan.additional_files.len(), 2);
}

#[test]
fn raw_additional_files_are_copied_source_equal_under_source_extras() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    create_observed_session(source.path());
    let scan = scan_once(source.path(), Transmitter::Tx01, UtcOffset::UTC).unwrap();
    let context = BackupItemContext {
        source_root: source.path(),
        destination_root: destination.path(),
        transmitter: Transmitter::Tx01,
        backup_run_id: "run-1",
        verified_at: "2026-08-10T00:00:00Z",
    };

    for observation in &scan.additional_files {
        let destination_relative =
            Path::new("source-extras/2026/2026-08-10/TX01").join(&observation.relative_path);
        let evidence = execute_additional_file_copy(
            &context,
            observation,
            &destination_relative,
            &CancellationToken::default(),
        )
        .unwrap();

        assert_eq!(evidence.source_relative_path, observation.relative_path);
        assert_eq!(evidence.artifact_relative_path, destination_relative);
        assert_eq!(evidence.source_sha256, evidence.artifact_sha256);
        assert_eq!(
            fs::read(source.path().join(&observation.relative_path)).unwrap(),
            fs::read(destination.path().join(&evidence.artifact_relative_path)).unwrap()
        );
        assert!(source.path().join(&observation.relative_path).is_file());
    }
}

#[test]
fn additional_file_destination_preserves_the_session_beneath_source_extras() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    create_observed_session(source.path());
    let scan = scan_once(source.path(), Transmitter::Tx01, UtcOffset::UTC).unwrap();
    let external = scan
        .additional_files
        .iter()
        .find(|file| file.classification == AdditionalFileClass::M4a)
        .unwrap()
        .clone();

    let plan = plan_additional_file(
        source.path(),
        destination.path(),
        Transmitter::Tx01,
        external,
    )
    .unwrap();

    assert_eq!(plan.disposition, DestinationDisposition::Copy);
    assert_eq!(
        plan.relative_destination,
        Path::new(
            "source-extras/2026/2026-08-10/TX01/TX_MIC001_20260810_001116/TX01_MIC001_20260810_001116.m4a"
        )
    );
}

#[test]
fn additional_copy_rejects_an_observation_whose_name_does_not_match_its_path() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    create_observed_session(source.path());
    let scan = scan_once(source.path(), Transmitter::Tx01, UtcOffset::UTC).unwrap();
    let observed = scan
        .additional_files
        .iter()
        .find(|file| file.classification == AdditionalFileClass::M4a)
        .unwrap();
    let forged = AdditionalFileObservation {
        file_name: "different.m4a".to_owned(),
        ..observed.clone()
    };
    let context = BackupItemContext {
        source_root: source.path(),
        destination_root: destination.path(),
        transmitter: Transmitter::Tx01,
        backup_run_id: "run-1",
        verified_at: "2026-08-10T00:00:00Z",
    };

    let error = execute_additional_file_copy(
        &context,
        &forged,
        Path::new("source-extras/2026/2026-08-10/TX01/TX_MIC001_20260810_001116/file.m4a"),
        &CancellationToken::default(),
    )
    .unwrap_err();

    assert!(matches!(error, CoreError::InvalidRequest));
}
