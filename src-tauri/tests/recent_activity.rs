use backup_core::{
    events::{ActivityEntry, ActivitySeverity},
    ledger::Ledger,
};
use backup_mic_lib::app_state::AppState;
use tempfile::tempdir;

#[test]
fn snapshot_delivers_the_newest_fifty_persisted_activities_in_order() {
    let directory = tempdir().unwrap();
    let mut ledger = Ledger::open(directory.path().join("ledger.sqlite3")).unwrap();
    for index in 0..55 {
        ledger
            .append_activity(&ActivityEntry {
                occurred_at: format!("2026-08-09T00:00:{index:02}Z"),
                code: "device_detected".to_owned(),
                source_id: None,
                source_label: None,
                count_value: Some(index),
                byte_value: None,
                severity: ActivitySeverity::Info,
            })
            .unwrap();
    }
    drop(ledger);

    let ledger = Ledger::open(directory.path().join("ledger.sqlite3")).unwrap();
    let state = AppState::new(ledger, directory.path().to_path_buf(), false, false).unwrap();
    let snapshot = state.snapshot_with_activity().unwrap();

    assert_eq!(snapshot.recent_activity.len(), 50);
    assert_eq!(
        snapshot
            .recent_activity
            .iter()
            .map(|entry| entry.count_value.unwrap())
            .collect::<Vec<_>>(),
        (5..55).rev().collect::<Vec<_>>()
    );
}
