use std::{fs, path::Path};

use backup_core::{
    artifact::{OutputFormat, VerifiedArtifact},
    destination::{DestinationDisposition, plan_rule_file},
    hash::hash_file,
    rule::{BackupRule, DateFolderLayout, DeviceConstraintProfile, FilenameProfile, RuleId},
    rule_scanner::{RuleFileObservation, SelectedFileKind},
    source::SourceId,
};
use tempfile::tempdir;
use time::macros::date;

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
        date_folder_layout: DateFolderLayout::YearMonth,
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

#[test]
fn recording_names_support_the_three_approved_date_folder_layouts() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    fs::create_dir_all(source.path().join("RECORD/FOLDER01")).unwrap();
    fs::write(
        source.path().join("RECORD/FOLDER01/ZOOM0001.WAV"),
        b"zoom audio",
    )
    .unwrap();
    let observation = observed(
        "RECORD/FOLDER01/ZOOM0001.WAV",
        SelectedFileKind::RecordingWav,
        10,
    );

    for (layout, expected) in [
        (
            DateFolderLayout::YearMonthDay,
            "Zoom H1n/2026/08/10/260810-zoom-ZOOM0001-field.wav",
        ),
        (
            DateFolderLayout::YearMonth,
            "Zoom H1n/2026/08/260810-zoom-ZOOM0001-field.wav",
        ),
        (
            DateFolderLayout::CompactDate,
            "Zoom H1n/260810/260810-zoom-ZOOM0001-field.wav",
        ),
    ] {
        let mut rule = zoom_rule();
        rule.date_folder_layout = layout;
        let plan = plan_rule_file(
            source.path(),
            destination.path(),
            &rule,
            &SourceId::new(),
            observation.clone(),
            None,
        )
        .unwrap();

        assert_eq!(plan.relative_destination, Path::new(expected));
    }
}

#[test]
fn dji_profile_prefix_and_suffix_are_preserved_for_every_date_layout() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    fs::create_dir_all(source.path().join("RECORD/FOLDER01")).unwrap();
    fs::write(
        source
            .path()
            .join("RECORD/FOLDER01/TX01_MIC001_20260810_120000.WAV"),
        b"zoom audio",
    )
    .unwrap();
    let observation = observed(
        "RECORD/FOLDER01/TX01_MIC001_20260810_120000.WAV",
        SelectedFileKind::RecordingWav,
        10,
    );

    for layout in [
        DateFolderLayout::YearMonthDay,
        DateFolderLayout::YearMonth,
        DateFolderLayout::CompactDate,
    ] {
        let mut rule = zoom_rule();
        rule.filename_prefix = "mic-".to_owned();
        rule.filename_suffix = "-backup".to_owned();
        rule.filename_profile = FilenameProfile::DjiTxShort;
        rule.date_folder_layout = layout;
        let plan = plan_rule_file(
            source.path(),
            destination.path(),
            &rule,
            &SourceId::new(),
            observation.clone(),
            None,
        )
        .unwrap();

        assert!(
            plan.relative_destination
                .ends_with("260810-mic-T01_MIC001_20260810_120000-backup.wav")
        );
    }
}

fn observed(relative_path: &str, kind: SelectedFileKind, size: u64) -> RuleFileObservation {
    RuleFileObservation {
        relative_path: relative_path.into(),
        file_name: Path::new(relative_path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        kind,
        session_relative_path: Some("RECORD/FOLDER01".into()),
        size,
        modified_nanos: 1,
        archive_date: date!(2026 - 08 - 10),
    }
}

#[test]
fn recording_names_use_rule_directory_calendar_prefix_and_suffix() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    fs::create_dir_all(source.path().join("RECORD/FOLDER01")).unwrap();
    fs::write(
        source.path().join("RECORD/FOLDER01/ZOOM0001.WAV"),
        b"zoom audio",
    )
    .unwrap();
    let source_id = SourceId::new();

    let plan = plan_rule_file(
        source.path(),
        destination.path(),
        &zoom_rule(),
        &source_id,
        observed(
            "RECORD/FOLDER01/ZOOM0001.WAV",
            SelectedFileKind::RecordingWav,
            10,
        ),
        None,
    )
    .unwrap();

    assert_eq!(plan.disposition, DestinationDisposition::Copy);
    assert_eq!(
        plan.relative_destination,
        Path::new("Zoom H1n/2026/08/260810-zoom-ZOOM0001-field.wav")
    );
    assert_eq!(
        plan.relative_destination.with_extension("m4a"),
        Path::new("Zoom H1n/2026/08/260810-zoom-ZOOM0001-field.m4a")
    );
}

#[test]
fn verified_m4a_evidence_reuses_the_calendar_name() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    fs::create_dir_all(source.path().join("RECORD/FOLDER01")).unwrap();
    fs::write(
        source.path().join("RECORD/FOLDER01/ZOOM0001.WAV"),
        b"zoom audio",
    )
    .unwrap();
    let relative = Path::new("Zoom H1n/2026/08/260810-zoom-ZOOM0001-field.m4a");
    fs::create_dir_all(destination.path().join(relative).parent().unwrap()).unwrap();
    fs::write(destination.path().join(relative), b"converted m4a").unwrap();
    let digest = hash_file(&destination.path().join(relative)).unwrap();
    let existing = VerifiedArtifact {
        relative_path: relative.to_path_buf(),
        format: OutputFormat::M4a,
        byte_count: digest.size,
        sha256: digest.sha256,
        audio: None,
    };

    let plan = plan_rule_file(
        source.path(),
        destination.path(),
        &zoom_rule(),
        &SourceId::new(),
        observed(
            "RECORD/FOLDER01/ZOOM0001.WAV",
            SelectedFileKind::RecordingWav,
            10,
        ),
        Some(&existing),
    )
    .unwrap();

    assert_eq!(plan.disposition, DestinationDisposition::Reuse);
    assert_eq!(plan.relative_destination, relative);
}

#[test]
fn verified_m4a_evidence_reuses_its_exact_path_after_archive_rename() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    fs::create_dir_all(source.path().join("RECORD/FOLDER01")).unwrap();
    fs::write(
        source.path().join("RECORD/FOLDER01/ZOOM0001.WAV"),
        b"zoom audio",
    )
    .unwrap();
    let relative = Path::new("Previous Archive/2026/08/260810-ZOOM0001.m4a");
    fs::create_dir_all(destination.path().join(relative).parent().unwrap()).unwrap();
    fs::write(destination.path().join(relative), b"converted m4a").unwrap();
    let digest = hash_file(&destination.path().join(relative)).unwrap();
    let existing = VerifiedArtifact {
        relative_path: relative.to_path_buf(),
        format: OutputFormat::M4a,
        byte_count: digest.size,
        sha256: digest.sha256,
        audio: None,
    };

    let plan = plan_rule_file(
        source.path(),
        destination.path(),
        &zoom_rule(),
        &SourceId::new(),
        observed(
            "RECORD/FOLDER01/ZOOM0001.WAV",
            SelectedFileKind::RecordingWav,
            10,
        ),
        Some(&existing),
    )
    .unwrap();

    assert_eq!(plan.disposition, DestinationDisposition::Reuse);
    assert_eq!(plan.relative_destination, relative);
}

#[test]
fn companion_paths_hide_source_ids_and_reuse_equal_content() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    fs::create_dir_all(source.path().join("RECORD/FOLDER01")).unwrap();
    fs::write(
        source.path().join("RECORD/FOLDER01/notes.txt"),
        b"field notes",
    )
    .unwrap();
    let source_id = SourceId::new();
    let observation = observed("RECORD/FOLDER01/notes.txt", SelectedFileKind::Companion, 11);

    let first = plan_rule_file(
        source.path(),
        destination.path(),
        &zoom_rule(),
        &source_id,
        observation.clone(),
        None,
    )
    .unwrap();
    let components = first
        .relative_destination
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(components[0], "Zoom H1n");
    assert_eq!(components[1], "source-extras");
    assert_eq!(components[2].len(), 24);
    assert!(
        !first
            .relative_destination
            .to_string_lossy()
            .contains(source_id.as_str())
    );
    assert!(
        first
            .relative_destination
            .ends_with("RECORD/FOLDER01/notes.txt")
    );

    fs::create_dir_all(
        destination
            .path()
            .join(&first.relative_destination)
            .parent()
            .unwrap(),
    )
    .unwrap();
    fs::write(
        destination.path().join(&first.relative_destination),
        b"field notes",
    )
    .unwrap();
    let reused = plan_rule_file(
        source.path(),
        destination.path(),
        &zoom_rule(),
        &source_id,
        observation,
        None,
    )
    .unwrap();
    assert_eq!(reused.disposition, DestinationDisposition::Reuse);
    assert_eq!(reused.relative_destination, first.relative_destination);
}

#[test]
fn companion_collision_uses_the_shortest_source_hash_suffix_without_clobbering() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    fs::create_dir_all(source.path().join("RECORD/FOLDER01")).unwrap();
    let source_path = source.path().join("RECORD/FOLDER01/notes.txt");
    fs::write(&source_path, b"field notes").unwrap();
    let source_id = SourceId::new();
    let observation = observed("RECORD/FOLDER01/notes.txt", SelectedFileKind::Companion, 11);
    let default = plan_rule_file(
        source.path(),
        destination.path(),
        &zoom_rule(),
        &source_id,
        observation.clone(),
        None,
    )
    .unwrap();
    fs::create_dir_all(
        destination
            .path()
            .join(&default.relative_destination)
            .parent()
            .unwrap(),
    )
    .unwrap();
    fs::write(
        destination.path().join(&default.relative_destination),
        b"different",
    )
    .unwrap();

    let collision = plan_rule_file(
        source.path(),
        destination.path(),
        &zoom_rule(),
        &source_id,
        observation,
        None,
    )
    .unwrap();
    let digest = hash_file(&source_path).unwrap();

    assert_eq!(collision.disposition, DestinationDisposition::Copy);
    assert_eq!(
        collision
            .relative_destination
            .file_name()
            .unwrap()
            .to_string_lossy(),
        format!("notes-{}.txt", &digest.sha256[..8])
    );
    assert_eq!(
        fs::read(destination.path().join(default.relative_destination)).unwrap(),
        b"different"
    );
}

#[test]
fn dji_prefix_and_suffix_use_the_same_rule_destination_engine() {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    fs::write(
        source.path().join("TX01_MIC001_20260809_010203.WAV"),
        b"dji audio",
    )
    .unwrap();
    let mut rule = zoom_rule();
    rule.name = "DJI Mic Mini 2S".to_owned();
    rule.archive_directory_name = "DJI Mic Mini 2S".to_owned();
    rule.filename_prefix = "rec-".to_owned();
    rule.filename_suffix = "-backup".to_owned();
    rule.filename_profile = FilenameProfile::DjiTxShort;
    let mut observation = observed(
        "TX01_MIC001_20260809_010203.WAV",
        SelectedFileKind::RecordingWav,
        9,
    );
    observation.session_relative_path = None;
    observation.archive_date = date!(2026 - 08 - 09);

    let plan = plan_rule_file(
        source.path(),
        destination.path(),
        &rule,
        &SourceId::new(),
        observation,
        None,
    )
    .unwrap();

    assert_eq!(
        plan.relative_destination,
        Path::new("DJI Mic Mini 2S/2026/08/260809-rec-T01_MIC001_20260809_010203-backup.wav")
    );
}
