use backup_core::{
    error::CoreError,
    rule::{
        BackupRule, BackupRuleDraft, DeviceConstraintProfile, FilenameProfile, MAX_GLOB_CHARS,
        MAX_PATTERNS_PER_GROUP, MAX_RULE_TEXT_CHARS, RuleId, apply_filename_profile, compile_rule,
        destination_stem, validate_rule,
    },
};

fn zoom_draft() -> BackupRuleDraft {
    BackupRuleDraft {
        id: None,
        name: "Zoom H1n".to_owned(),
        archive_directory_name: "Zoom H1n".to_owned(),
        enabled: true,
        volume_name_glob: "ZOOM_*".to_owned(),
        required_path_globs: vec!["RECORD/**".to_owned()],
        backup_file_globs: vec!["RECORD/**/*.WAV".to_owned()],
        session_directory_globs: vec!["RECORD/FOLDER*".to_owned()],
        filename_prefix: "zoom-".to_owned(),
        filename_suffix: "-field".to_owned(),
        date_folder_layout: Default::default(),
    }
}

fn stored_rule(draft: BackupRuleDraft, filename_profile: FilenameProfile) -> BackupRule {
    BackupRule {
        id: RuleId::new(),
        name: draft.name,
        archive_directory_name: draft.archive_directory_name,
        enabled: draft.enabled,
        volume_name_glob: draft.volume_name_glob,
        required_path_globs: draft.required_path_globs,
        backup_file_globs: draft.backup_file_globs,
        session_directory_globs: draft.session_directory_globs,
        filename_prefix: draft.filename_prefix,
        filename_suffix: draft.filename_suffix,
        date_folder_layout: draft.date_folder_layout,
        filename_profile,
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
fn compiled_rule_matches_volume_and_paths_case_insensitively() {
    let validated = validate_rule(zoom_draft()).unwrap();
    let compiled = compile_rule(stored_rule(validated, FilenameProfile::Preserve)).unwrap();
    let observed = [
        "record\\folder01",
        "record\\folder01\\zoom0001.wav",
        "record\\folder01\\notes.txt",
    ];

    assert!(compiled.matches_volume_name("zoom_h1n"));
    assert!(compiled.required_paths_match(observed));
    assert!(compiled.selects_backup_file("record\\folder01\\zoom0001.wav"));
    assert!(!compiled.selects_backup_file("record/folder01/zoom0001.mp3"));
    assert!(compiled.matches_session_directory("record/folder01"));
}

#[test]
fn destination_stem_wraps_the_source_stem_without_changing_it() {
    let rule = stored_rule(zoom_draft(), FilenameProfile::Preserve);

    assert_eq!(
        destination_stem(&rule, "ZOOM0001").unwrap(),
        "zoom-ZOOM0001-field"
    );
}

#[test]
fn dji_filename_profile_shortens_only_a_leading_transmitter_prefix() {
    assert_eq!(
        apply_filename_profile(FilenameProfile::DjiTxShort, "TX01_MIC001"),
        "T01_MIC001"
    );
    assert_eq!(
        apply_filename_profile(FilenameProfile::DjiTxShort, "tx02_MIC002"),
        "T02_MIC002"
    );
    assert_eq!(
        apply_filename_profile(FilenameProfile::DjiTxShort, "EDIT_TX01_MIC001"),
        "EDIT_TX01_MIC001"
    );
    assert_eq!(
        apply_filename_profile(FilenameProfile::Preserve, "TX01_MIC001"),
        "TX01_MIC001"
    );
}

#[test]
fn invalid_rules_are_rejected_before_any_filesystem_matching() {
    let mut cases = Vec::new();

    let mut no_backup_patterns = zoom_draft();
    no_backup_patterns.backup_file_globs.clear();
    cases.push(no_backup_patterns);

    let mut malformed_glob = zoom_draft();
    malformed_glob.volume_name_glob = "[abc".to_owned();
    cases.push(malformed_glob);

    let mut unsafe_suffix = zoom_draft();
    unsafe_suffix.filename_suffix = "/field".to_owned();
    cases.push(unsafe_suffix);

    let mut parent_name = zoom_draft();
    parent_name.name = "..".to_owned();
    cases.push(parent_name);

    let mut oversized_name = zoom_draft();
    oversized_name.name = "n".repeat(MAX_RULE_TEXT_CHARS + 1);
    cases.push(oversized_name);

    let mut oversized_glob = zoom_draft();
    oversized_glob.backup_file_globs = vec!["a".repeat(MAX_GLOB_CHARS + 1)];
    cases.push(oversized_glob);

    let mut too_many_patterns = zoom_draft();
    too_many_patterns.required_path_globs = (0..=MAX_PATTERNS_PER_GROUP)
        .map(|index| format!("RECORD/{index}"))
        .collect();
    cases.push(too_many_patterns);

    for draft in cases {
        assert!(matches!(validate_rule(draft), Err(CoreError::InvalidRule)));
    }
}
