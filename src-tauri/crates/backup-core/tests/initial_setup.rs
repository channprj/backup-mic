use backup_core::{
    error::CoreError,
    initial_setup::{DESTINATION_SETTING, INITIAL_SETUP_SETTING, InitialSetupMarker},
    ledger::Ledger,
};
use rusqlite::Connection;
use tempfile::tempdir;

const NOW: &str = "2026-08-11T00:00:00Z";

#[test]
fn fresh_destination_and_review_marker_persist_and_complete() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    let mut ledger = Ledger::open(&path).unwrap();

    assert_eq!(ledger.initial_setup_marker().unwrap(), None);
    ledger
        .persist_destination(r#""/tmp/Backup Mic""#, true, NOW)
        .unwrap();
    assert_eq!(
        ledger.initial_setup_marker().unwrap(),
        Some(InitialSetupMarker::SettingsReviewPending)
    );

    ledger.complete_initial_setup(NOW).unwrap();
    assert_eq!(
        ledger.initial_setup_marker().unwrap(),
        Some(InitialSetupMarker::Complete)
    );
    drop(ledger);

    let reopened = Ledger::open(path).unwrap();
    assert_eq!(
        reopened.setting(DESTINATION_SETTING).unwrap().as_deref(),
        Some(r#""/tmp/Backup Mic""#)
    );
    assert_eq!(
        reopened.initial_setup_marker().unwrap(),
        Some(InitialSetupMarker::Complete)
    );
}

#[test]
fn compatible_existing_destination_keeps_an_absent_marker() {
    let directory = tempdir().unwrap();
    let mut ledger = Ledger::open(directory.path().join("ledger.sqlite3")).unwrap();

    ledger
        .persist_destination(r#""/tmp/Existing Backup""#, false, NOW)
        .unwrap();

    assert_eq!(ledger.initial_setup_marker().unwrap(), None);
    assert!(matches!(
        ledger.complete_initial_setup(NOW),
        Err(CoreError::InvalidRequest)
    ));
    assert_eq!(ledger.initial_setup_marker().unwrap(), None);
}

#[test]
fn malformed_or_unknown_setup_markers_fail_closed() {
    let directory = tempdir().unwrap();
    let mut ledger = Ledger::open(directory.path().join("ledger.sqlite3")).unwrap();

    for invalid in ["not-json", "true", r#""unknown""#] {
        ledger
            .set_setting(INITIAL_SETUP_SETTING, invalid, NOW)
            .unwrap();
        assert!(matches!(
            ledger.initial_setup_marker(),
            Err(CoreError::LedgerCorrupt)
        ));
    }
}

#[test]
fn marker_failure_rolls_back_the_destination_in_the_same_transaction() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("ledger.sqlite3");
    let mut ledger = Ledger::open(&path).unwrap();
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            r#"
            CREATE TRIGGER fail_setup_marker BEFORE INSERT ON settings
            WHEN NEW.key = 'initial_setup_state'
            BEGIN
                SELECT RAISE(ABORT, 'fixture failure');
            END;
            "#,
        )
        .unwrap();
    drop(connection);

    assert!(matches!(
        ledger.persist_destination(r#""/tmp/Rolled Back""#, true, NOW),
        Err(CoreError::Ledger(_))
    ));
    assert_eq!(ledger.setting(DESTINATION_SETTING).unwrap(), None);
    assert_eq!(ledger.initial_setup_marker().unwrap(), None);
}
