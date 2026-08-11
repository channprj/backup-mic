use std::path::PathBuf;

use backup_core::{
    ledger::Ledger,
    preset::DJI_PRESET_ID,
    rule::BackupRuleDraft,
    source::{SourceId, SourceRecord},
};
use backup_mic_lib::{
    app_state::AppState,
    dto::{BackupRuleDto, RuleTestResultDto, SourceSnapshotDto},
};
use serde_json::Value;
use tempfile::tempdir;

fn zoom_draft() -> BackupRuleDraft {
    BackupRuleDraft {
        id: None,
        name: "Zoom H1n".to_owned(),
        archive_directory_name: "Zoom H1n".to_owned(),
        enabled: true,
        volume_name_glob: "ZOOM_*".to_owned(),
        required_path_globs: vec!["RECORD/**".to_owned()],
        backup_file_globs: vec!["RECORD/**/*.WAV".to_owned()],
        session_directory_globs: vec!["RECORD/*".to_owned()],
        filename_prefix: "zoom-".to_owned(),
        filename_suffix: "-field".to_owned(),
        date_folder_layout: Default::default(),
    }
}

fn state() -> AppState {
    let root = tempdir().expect("temp root").keep();
    let ledger = Ledger::open(root.join("ledger.sqlite3")).expect("ledger");
    AppState::new(ledger, PathBuf::from("/tmp/backup-mic-tests"), true, false).expect("state")
}

#[test]
fn snapshot_bootstraps_the_editable_dji_preset_without_pairing_authority() {
    let snapshot = state().snapshot();
    let dji = snapshot
        .backup_rules
        .iter()
        .find(|rule| rule.id == DJI_PRESET_ID)
        .expect("DJI preset");
    assert!(dji.is_dji_preset);
    assert_eq!(dji.backup_file_globs, ["*.WAV", "TX_MIC*/*.WAV"]);

    let json = serde_json::to_value(snapshot).expect("snapshot JSON");
    assert!(json.get("sources").is_some());
    assert!(json.get("backup_rules").is_some());
    assert!(json.get("transmitters").is_none());
    assert!(json.get("pairing_candidates").is_none());
}

#[test]
fn rule_and_source_dtos_expose_only_display_safe_values() {
    let rule: BackupRuleDto = state()
        .snapshot()
        .backup_rules
        .into_iter()
        .next()
        .expect("preset");
    let source = SourceSnapshotDto::idle(
        "11111111-1111-4111-8111-111111111111".to_owned(),
        rule.name.clone(),
        "ZOOM_TEST".to_owned(),
        None,
    );
    assert!(uuid::Uuid::parse_str(&source.source_id).is_ok());
    let result = RuleTestResultDto {
        matched_volumes: vec!["ZOOM_TEST".to_owned()],
        matched_file_count: 3,
        conflict_rule_names: Vec::new(),
    };
    let serialized = serde_json::to_string(&(rule, source, result)).expect("serialize");
    for forbidden in ["volume_uuid", "mount_root", "source_path", "sha256"] {
        assert!(!serialized.contains(forbidden));
    }
}

#[test]
fn saving_and_archiving_rules_refreshes_the_public_snapshot() {
    let state = state();
    let rule = state
        .save_backup_rule_for_state(zoom_draft(), "2026-08-10T00:00:00Z")
        .expect("save rule");
    assert!(
        state
            .snapshot()
            .backup_rules
            .iter()
            .any(|item| item.id == rule.id.as_str())
    );
    state
        .upsert_source_for_state(
            &SourceRecord {
                id: SourceId::new(),
                rule_id: rule.id.clone(),
                volume_uuid: "ZOOM-UUID".to_owned(),
                legacy_slot: None,
                display_name: "ZOOM_TEST".to_owned(),
            },
            "2026-08-10T00:00:30Z",
        )
        .expect("source evidence");

    state
        .archive_backup_rule_for_state(rule.id.as_str(), "2026-08-10T00:01:00Z")
        .expect("archive rule");
    let archived = state
        .snapshot()
        .backup_rules
        .into_iter()
        .find(|item| item.id == rule.id.as_str())
        .expect("archived rule");
    assert!(archived.archived);
    assert!(!archived.enabled);
}

#[test]
fn public_snapshot_json_rejects_unknown_authority_fields() {
    let mut value = serde_json::to_value(state().snapshot()).expect("snapshot");
    value["volume_uuid"] = Value::String("private".to_owned());
    assert!(serde_json::from_value::<backup_mic_lib::dto::AppSnapshotDto>(value).is_err());
}
