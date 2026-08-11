use backup_core::{
    error::CoreError,
    ledger::Ledger,
    preferences::{BackupPreferences, PreferenceKey},
    preset::{DJI_PRESET_KIND, DJI_PRESET_REVISION},
    rule::{
        BackupRule, BackupRuleDraft, DateFolderLayout, DeviceConstraintProfile, FilenameProfile,
        RuleId,
    },
};
use rusqlite::{Connection, params};
use tempfile::tempdir;

fn zoom_draft(name: &str) -> BackupRuleDraft {
    BackupRuleDraft {
        id: None,
        name: name.to_owned(),
        archive_directory_name: name.to_owned(),
        enabled: true,
        volume_name_glob: "ZOOM_*".to_owned(),
        required_path_globs: vec!["RECORD/**".to_owned()],
        backup_file_globs: vec!["RECORD/**/*.WAV".to_owned(), "STEREO/**/*.WAV".to_owned()],
        session_directory_globs: vec!["RECORD/FOLDER*".to_owned()],
        filename_prefix: "zoom-".to_owned(),
        filename_suffix: "-field".to_owned(),
        date_folder_layout: Default::default(),
    }
}

fn editable_draft(rule: &BackupRule) -> BackupRuleDraft {
    BackupRuleDraft {
        id: Some(rule.id.clone()),
        name: rule.name.clone(),
        archive_directory_name: rule.archive_directory_name.clone(),
        enabled: rule.enabled,
        volume_name_glob: rule.volume_name_glob.clone(),
        required_path_globs: rule.required_path_globs.clone(),
        backup_file_globs: rule.backup_file_globs.clone(),
        session_directory_globs: rule.session_directory_globs.clone(),
        filename_prefix: rule.filename_prefix.clone(),
        filename_suffix: rule.filename_suffix.clone(),
        date_folder_layout: rule.date_folder_layout,
    }
}

fn insert_device_binding(path: &std::path::Path, rule_id: &RuleId, slot_label: &str) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute(
            r#"INSERT INTO rule_device_bindings(
                 rule_id, volume_uuid, volume_uuid_normalized, protocol, media_name,
                 nominal_capacity, slot_label, paired_at
               ) VALUES (?1, ?2, ?3, 'USB', 'Wireless Mic Tx Media', 15636365312, ?4, ?5)"#,
            params![
                rule_id.as_str(),
                format!("{slot_label}-UUID"),
                format!("{slot_label}-uuid"),
                slot_label,
                "2026-08-10T00:00:00Z",
            ],
        )
        .unwrap();
}

fn binding_count(path: &std::path::Path, rule_id: &RuleId) -> u64 {
    let connection = Connection::open(path).unwrap();
    let count = connection
        .query_row(
            "SELECT COUNT(*) FROM rule_device_bindings WHERE rule_id = ?1",
            [rule_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    u64::try_from(count).unwrap()
}

#[test]
fn new_ledger_seeds_the_editable_dji_preset_and_round_trips_user_rules() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    let mut ledger = Ledger::open(&path).unwrap();

    let rules = ledger.backup_rules(false).unwrap();
    assert_eq!(rules.len(), 1);
    let dji = &rules[0];
    assert_eq!(dji.name, "DJI Mic Mini 2S");
    assert_eq!(dji.archive_directory_name, "DJI Mic Mini 2S");
    assert!(dji.enabled);
    assert_eq!(dji.volume_name_glob, "*");
    assert!(dji.required_path_globs.is_empty());
    assert_eq!(
        dji.backup_file_globs,
        ["*.WAV".to_owned(), "TX_MIC*/*.WAV".to_owned()]
    );
    assert_eq!(dji.session_directory_globs, ["TX_MIC*".to_owned()]);
    assert_eq!(dji.filename_profile, FilenameProfile::DjiTxShort);
    assert_eq!(dji.date_folder_layout, DateFolderLayout::YearMonth);
    assert_eq!(
        dji.device_constraint_profile,
        DeviceConstraintProfile::DjiMicMini2s
    );
    assert_eq!(dji.preset_kind.as_deref(), Some(DJI_PRESET_KIND));
    assert_eq!(dji.preset_revision, Some(DJI_PRESET_REVISION));
    assert!(!dji.archive_directory_locked);
    assert_eq!(dji.archived_at, None);

    let saved = ledger
        .save_backup_rule(zoom_draft("Zoom H1n"), "2026-08-10T01:00:00Z")
        .unwrap();
    assert_eq!(saved.filename_profile, FilenameProfile::Preserve);
    assert_eq!(
        saved.device_constraint_profile,
        DeviceConstraintProfile::GenericExternal
    );
    assert_eq!(saved.preset_kind, None);
    assert_eq!(saved.preset_revision, None);
    drop(ledger);

    let ledger = Ledger::open(&path).unwrap();
    let restored = ledger
        .backup_rules(false)
        .unwrap()
        .into_iter()
        .find(|rule| rule.id == saved.id)
        .unwrap();
    assert_eq!(restored, saved);
}

#[test]
fn date_folder_layouts_round_trip_through_create_and_update() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    let mut ledger = Ledger::open(&path).unwrap();

    for (index, layout) in [
        DateFolderLayout::YearMonthDay,
        DateFolderLayout::YearMonth,
        DateFolderLayout::CompactDate,
    ]
    .into_iter()
    .enumerate()
    {
        let mut draft = zoom_draft(&format!("Zoom {index}"));
        draft.date_folder_layout = layout;
        let saved = ledger
            .save_backup_rule(draft, "2026-08-12T00:00:00Z")
            .unwrap();
        assert_eq!(saved.date_folder_layout, layout);

        let mut edit = editable_draft(&saved);
        edit.date_folder_layout = match layout {
            DateFolderLayout::YearMonthDay => DateFolderLayout::CompactDate,
            DateFolderLayout::YearMonth | DateFolderLayout::CompactDate => {
                DateFolderLayout::YearMonthDay
            }
        };
        let expected = edit.date_folder_layout;
        let updated = ledger
            .save_backup_rule(edit, "2026-08-12T00:01:00Z")
            .unwrap();
        assert_eq!(updated.date_folder_layout, expected);
    }

    drop(ledger);
    let reopened = Ledger::open(&path).unwrap();
    assert_eq!(
        reopened
            .backup_rules(false)
            .unwrap()
            .into_iter()
            .filter(|rule| rule.preset_kind.is_none())
            .count(),
        3
    );
}

#[test]
fn version_six_rules_migrate_to_year_month_layout() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!("../migrations/0001_initial.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!(
            "../migrations/0002_artifacts_and_preferences.sql"
        ))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0003_batch_manifests.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!(
            "../migrations/0004_durable_superseded_wav_evidence.sql"
        ))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0005_backup_rules.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../migrations/0006_dynamic_sources.sql"))
        .unwrap();
    drop(connection);

    let ledger = Ledger::open(&path).unwrap();
    assert_eq!(
        ledger.dji_rule().unwrap().date_folder_layout,
        DateFolderLayout::YearMonth
    );
    drop(ledger);

    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 7",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn normalized_active_rule_names_are_unique() {
    let directory = tempdir().unwrap();
    let mut ledger = Ledger::open(directory.path().join("ledger.sqlite3")).unwrap();
    ledger
        .save_backup_rule(zoom_draft("Caf\u{e9}"), "2026-08-10T01:00:00Z")
        .unwrap();

    assert!(matches!(
        ledger.save_backup_rule(zoom_draft("CAFE\u{301}"), "2026-08-10T01:01:00Z"),
        Err(CoreError::InvalidRule)
    ));
}

#[test]
fn restoring_the_dji_preset_preserves_identity_bindings_and_preferences() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    let ledger = Ledger::open(&path).unwrap();
    let original = ledger.dji_rule().unwrap();
    drop(ledger);

    insert_device_binding(&path, &original.id, "TX01");

    let mut ledger = Ledger::open(&path).unwrap();
    ledger
        .set_preference(PreferenceKey::AutomaticTrash, true, "2026-08-10T01:00:00Z")
        .unwrap();
    let mut edited = editable_draft(&original);
    edited.filename_prefix = "custom-".to_owned();
    edited.date_folder_layout = DateFolderLayout::CompactDate;
    ledger
        .save_backup_rule(edited, "2026-08-10T01:01:00Z")
        .unwrap();
    ledger
        .archive_backup_rule(&original.id, "2026-08-10T01:02:00Z")
        .unwrap();
    assert!(
        !ledger
            .backup_rules(false)
            .unwrap()
            .iter()
            .any(|rule| rule.id == original.id)
    );

    let restored = ledger.restore_dji_preset("2026-08-10T01:03:00Z").unwrap();
    assert_eq!(restored.id, original.id);
    assert!(restored.enabled);
    assert_eq!(restored.filename_prefix, "");
    assert_eq!(restored.date_folder_layout, DateFolderLayout::YearMonth);
    assert_eq!(restored.preset_revision, Some(DJI_PRESET_REVISION));
    assert_eq!(restored.archived_at, None);
    assert_eq!(
        ledger.read_preferences().unwrap(),
        BackupPreferences {
            automatic_backup: true,
            m4a_conversion: true,
            automatic_trash: true,
        }
    );
    drop(ledger);

    assert_eq!(binding_count(&path, &original.id), 1);
}

#[test]
fn archive_deletes_unused_rules_but_retains_referenced_rules_as_evidence() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    let mut ledger = Ledger::open(&path).unwrap();
    let unused = ledger
        .save_backup_rule(zoom_draft("Unused"), "2026-08-10T01:00:00Z")
        .unwrap();
    let referenced = ledger
        .save_backup_rule(zoom_draft("Referenced"), "2026-08-10T01:01:00Z")
        .unwrap();
    drop(ledger);
    insert_device_binding(&path, &referenced.id, "TX02");

    let mut ledger = Ledger::open(&path).unwrap();
    ledger
        .archive_backup_rule(&unused.id, "2026-08-10T01:02:00Z")
        .unwrap();
    ledger
        .archive_backup_rule(&referenced.id, "2026-08-10T01:03:00Z")
        .unwrap();

    let all_rules = ledger.backup_rules(true).unwrap();
    assert!(!all_rules.iter().any(|rule| rule.id == unused.id));
    let archived = all_rules
        .iter()
        .find(|rule| rule.id == referenced.id)
        .unwrap();
    assert!(!archived.enabled);
    assert_eq!(
        archived.archived_at.as_deref(),
        Some("2026-08-10T01:03:00Z")
    );
}

#[test]
fn evidence_locked_archive_directory_cannot_be_changed() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    let mut ledger = Ledger::open(&path).unwrap();
    let saved = ledger
        .save_backup_rule(zoom_draft("Locked"), "2026-08-10T01:00:00Z")
        .unwrap();
    drop(ledger);

    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE backup_rules SET archive_directory_locked = 1 WHERE id = ?1",
            [saved.id.as_str()],
        )
        .unwrap();
    drop(connection);

    let mut ledger = Ledger::open(&path).unwrap();
    let mut edit = editable_draft(
        &ledger
            .backup_rules(false)
            .unwrap()
            .into_iter()
            .find(|rule| rule.id == saved.id)
            .unwrap(),
    );
    edit.archive_directory_name = "Different".to_owned();
    assert!(matches!(
        ledger.save_backup_rule(edit, "2026-08-10T01:01:00Z"),
        Err(CoreError::InvalidRule)
    ));
}

#[test]
fn evidence_locked_rule_can_be_renamed_without_changing_its_archive_directory() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    let mut ledger = Ledger::open(&path).unwrap();
    let saved = ledger
        .save_backup_rule(zoom_draft("Locked"), "2026-08-10T01:00:00Z")
        .unwrap();
    ledger
        .lock_rule_archive_directory(&saved.id, &saved.archive_directory_name)
        .unwrap();

    let locked = ledger.backup_rule(&saved.id).unwrap().unwrap();
    let mut edit = editable_draft(&locked);
    edit.name = "Renamed".to_owned();
    edit.filename_prefix = "renamed-".to_owned();
    let updated = ledger
        .save_backup_rule(edit, "2026-08-10T01:01:00Z")
        .unwrap();

    assert_eq!(updated.name, "Renamed");
    assert_eq!(updated.archive_directory_name, "Locked");
    assert_eq!(updated.filename_prefix, "renamed-");
    assert!(updated.archive_directory_locked);
}
