use std::{fs, path::PathBuf};

use dji_mic_backup_lib::commands::REGISTERED_COMMANDS;
use dji_mic_backup_lib::dto::{AppSnapshotDto, DeletionProposalSummaryDto};
use serde_json::Value;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../contracts/fixtures")
        .join(name)
}

fn fixture(name: &str) -> String {
    fs::read_to_string(fixture_path(name)).expect("fixture must be readable")
}

fn assert_safe_json(value: &Value) {
    let serialized = serde_json::to_string(value).expect("value serializes");
    let lower = serialized.to_ascii_lowercase();
    for forbidden in [
        "/volumes/",
        "/users/",
        "source_path",
        "volume_uuid",
        "sha256",
        "filename",
        "ledger_id",
        "recording_id",
    ] {
        assert!(!lower.contains(forbidden), "fixture contains {forbidden}");
    }
}

#[test]
fn snapshot_fixtures_round_trip_through_rust_contract() {
    for name in [
        "backup-copying.json",
        "backup-complete.json",
        "error-destination-full.json",
        "partial-deletion.json",
    ] {
        let snapshot: AppSnapshotDto =
            serde_json::from_str(&fixture(name)).expect("valid snapshot");
        let value = serde_json::to_value(snapshot).expect("snapshot serializes");
        assert_safe_json(&value);
    }
}

#[test]
fn deletion_proposal_fixture_round_trips_through_rust_contract() {
    let proposal: DeletionProposalSummaryDto =
        serde_json::from_str(&fixture("deletion-proposal.json")).expect("valid proposal");
    let value = serde_json::to_value(proposal).expect("proposal serializes");
    assert_safe_json(&value);
}

#[test]
fn webview_capability_has_no_direct_plugin_authority() {
    let capability_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("capabilities/main.json");
    let capability: Value = serde_json::from_str(
        &fs::read_to_string(capability_path).expect("capability must be readable"),
    )
    .expect("capability must be valid JSON");
    assert_eq!(
        capability["permissions"],
        serde_json::json!(["core:default"])
    );
    assert_eq!(REGISTERED_COMMANDS.len(), 9);
}
