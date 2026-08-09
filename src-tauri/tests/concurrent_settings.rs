use std::{fs, time::Duration};

use backup_core::ledger::Ledger;
use dji_mic_backup_lib::{app_state::AppState, commands::set_m4a_conversion_for_state};
use tempfile::tempdir;

#[tokio::test]
async fn m4a_setting_saves_while_an_operation_keeps_its_frozen_preferences() {
    let state_directory = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let ledger_path = state_directory.path().join("ledger.sqlite3");
    let ledger = Ledger::open(&ledger_path).unwrap();
    let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();
    let operation = state.begin_operation().unwrap();
    let frozen = state.frozen_preferences();
    assert!(frozen.m4a_conversion);

    let snapshot = tokio::time::timeout(
        Duration::from_secs(2),
        set_m4a_conversion_for_state(&state, false, "2026-08-10T00:00:00Z".to_owned()),
    )
    .await
    .expect("setting save must not wait for the active operation")
    .unwrap();

    assert!(!snapshot.settings.m4a_conversion);
    assert!(!state.snapshot().settings.m4a_conversion);
    assert!(frozen.m4a_conversion);
    drop(operation);
    drop(state);
    assert!(
        !Ledger::open(&ledger_path)
            .unwrap()
            .read_preferences()
            .unwrap()
            .m4a_conversion
    );
    let log = destination
        .path()
        .join("logs/2026/08/260810-backup-mic.log");
    let text = fs::read_to_string(log).unwrap();
    assert!(text.contains("INFO setting.saved"));
    assert!(text.contains("setting=\"m4a_conversion\""));
    assert!(text.contains("applies=\"next_run\""));
}
