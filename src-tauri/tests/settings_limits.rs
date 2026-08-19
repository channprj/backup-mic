use std::{fs, time::Duration};

use backup_core::{ledger::Ledger, preferences::PreferenceLimit};
use backup_mic_lib::{
    app_state::AppState,
    commands::{set_free_space_reserve_for_state, set_rescan_interval_for_state},
};
use tempfile::{TempDir, tempdir};

struct Fixture {
    _state_directory: TempDir,
    destination: TempDir,
    ledger_path: std::path::PathBuf,
    state: AppState,
}

fn fixture() -> Fixture {
    let state_directory = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let ledger_path = state_directory.path().join("ledger.sqlite3");
    let ledger = Ledger::open(&ledger_path).unwrap();
    let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();
    Fixture {
        _state_directory: state_directory,
        destination,
        ledger_path,
        state,
    }
}

/// The whole audit log for the run, so a test can assert what was recorded.
fn audit_log(destination: &TempDir) -> String {
    let year = fs::read_dir(destination.path().join("logs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let month = fs::read_dir(year).unwrap().next().unwrap().unwrap().path();
    let log = fs::read_dir(month).unwrap().next().unwrap().unwrap().path();
    fs::read_to_string(log).unwrap()
}

#[tokio::test]
async fn a_saved_free_space_reserve_reaches_the_snapshot_ledger_and_audit_log() {
    let fixture = fixture();
    assert_eq!(
        fixture.state.snapshot().settings.free_space_reserve_gib,
        10,
        "the default must match the reserve the app shipped with"
    );

    let snapshot =
        set_free_space_reserve_for_state(&fixture.state, 25, "2026-08-19T00:00:00Z".to_owned())
            .await
            .unwrap();

    assert_eq!(snapshot.settings.free_space_reserve_gib, 25);
    assert_eq!(
        fixture.state.free_space_reserve_bytes(),
        25 * 1024 * 1024 * 1024,
        "the capacity preflight must see the new reserve"
    );

    let text = audit_log(&fixture.destination);
    assert!(text.contains("INFO setting.saved"));
    assert!(text.contains("setting=\"free_space_reserve_gib\""));
    assert!(text.contains("value=25"));

    drop(fixture.state);
    assert_eq!(
        Ledger::open(&fixture.ledger_path)
            .unwrap()
            .read_preferences()
            .unwrap()
            .free_space_reserve_gib,
        25
    );
}

#[tokio::test]
async fn a_saved_rescan_interval_reaches_the_snapshot_and_the_device_monitor() {
    let fixture = fixture();
    assert_eq!(
        fixture.state.snapshot().settings.rescan_interval_seconds,
        15
    );

    let snapshot =
        set_rescan_interval_for_state(&fixture.state, 60, "2026-08-19T00:00:00Z".to_owned())
            .await
            .unwrap();

    assert_eq!(snapshot.settings.rescan_interval_seconds, 60);
    assert_eq!(fixture.state.rescan_interval(), Duration::from_secs(60));

    drop(fixture.state);
    assert_eq!(
        Ledger::open(&fixture.ledger_path)
            .unwrap()
            .read_preferences()
            .unwrap()
            .rescan_interval_seconds,
        60
    );
}

#[tokio::test]
async fn an_out_of_range_limit_is_refused_and_changes_nothing() {
    let fixture = fixture();
    let (reserve_low, reserve_high) = PreferenceLimit::FreeSpaceReserveGib.range();
    let (interval_low, interval_high) = PreferenceLimit::RescanIntervalSeconds.range();

    for refused in [0, reserve_low - 1, reserve_high + 1] {
        let error = set_free_space_reserve_for_state(
            &fixture.state,
            refused,
            "2026-08-19T00:00:00Z".to_owned(),
        )
        .await
        .expect_err("an out-of-range reserve must be refused, not clamped");
        assert!(!error.retryable, "the same request will always be refused");
        assert_eq!(fixture.state.snapshot().settings.free_space_reserve_gib, 10);
    }

    for refused in [0, interval_low - 1, interval_high + 1] {
        set_rescan_interval_for_state(&fixture.state, refused, "2026-08-19T00:00:00Z".to_owned())
            .await
            .expect_err("an out-of-range interval must be refused, not clamped");
        assert_eq!(
            fixture.state.snapshot().settings.rescan_interval_seconds,
            15
        );
    }

    drop(fixture.state);
    let preferences = Ledger::open(&fixture.ledger_path)
        .unwrap()
        .read_preferences()
        .unwrap();
    assert_eq!(preferences.free_space_reserve_gib, 10);
    assert_eq!(preferences.rescan_interval_seconds, 15);
}

#[tokio::test]
async fn both_ends_of_each_range_are_accepted() {
    let fixture = fixture();
    let (reserve_low, reserve_high) = PreferenceLimit::FreeSpaceReserveGib.range();
    let (interval_low, interval_high) = PreferenceLimit::RescanIntervalSeconds.range();

    for accepted in [reserve_low, reserve_high] {
        let snapshot = set_free_space_reserve_for_state(
            &fixture.state,
            accepted,
            "2026-08-19T00:00:00Z".to_owned(),
        )
        .await
        .unwrap();
        assert_eq!(snapshot.settings.free_space_reserve_gib, accepted);
    }

    for accepted in [interval_low, interval_high] {
        let snapshot = set_rescan_interval_for_state(
            &fixture.state,
            accepted,
            "2026-08-19T00:00:00Z".to_owned(),
        )
        .await
        .unwrap();
        assert_eq!(snapshot.settings.rescan_interval_seconds, accepted);
    }
}
