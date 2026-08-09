use std::fs;

use backup_core::state::Transmitter;
use backup_core::{
    error::{CoreError, PublicError, PublicErrorCode},
    ledger::Ledger,
};
use dji_mic_backup_lib::app_state::AppState;
use dji_mic_backup_lib::failure_reporter::{FailureEvent, FailureReporter, FailureWriteOutcome};
use tempfile::tempdir;
use time::macros::datetime;

fn setting_failure() -> FailureEvent {
    FailureEvent {
        operation: "set_m4a_conversion",
        stage: "setting_persistence",
        transmitter: Some(Transmitter::Tx01),
        item_name: Some("TX01_MIC001_20260810_001116.wav".to_owned()),
        error_code: "ledger_operation_failed".to_owned(),
        os_kind: Some("permission_denied".to_owned()),
        retryable: true,
    }
}

#[test]
fn failure_reporter_prefers_the_destination_log() {
    let destination = tempdir().unwrap();
    let fallback = tempdir().unwrap();
    let reporter = FailureReporter::new(fallback.path());

    let outcome = reporter.report(
        Some(destination.path()),
        datetime!(2026-08-10 09:30:15 +09:00),
        &setting_failure(),
    );

    assert_eq!(outcome, FailureWriteOutcome::Primary);
    assert!(
        destination
            .path()
            .join("logs/2026/08/260810-backup-mic.log")
            .is_file()
    );
    assert!(!fallback.path().join("2026").exists());
}

#[test]
fn failure_reporter_uses_a_privacy_safe_fallback_when_primary_is_unavailable() {
    let primary = tempdir().unwrap();
    let fallback = tempdir().unwrap();
    fs::write(primary.path().join("logs"), b"not a directory").unwrap();
    let reporter = FailureReporter::new(fallback.path());

    let outcome = reporter.report(
        Some(primary.path()),
        datetime!(2026-08-10 09:30:15 +09:00),
        &setting_failure(),
    );

    assert_eq!(outcome, FailureWriteOutcome::Fallback);
    let output = fs::read_to_string(fallback.path().join("2026/08/260810-backup-mic.log")).unwrap();
    assert!(output.contains("operation=\"set_m4a_conversion\""));
    assert!(output.contains("stage=\"setting_persistence\""));
    assert!(output.contains("error_code=\"ledger_operation_failed\""));
    assert!(output.contains("os_kind=\"permission_denied\""));
    assert!(output.contains("retryable=true"));
    assert!(!output.contains("/Volumes/"));
    assert!(!output.contains("60fc7226f9fcaa230c4788f44152bb5257a7a18f60a7a4d8bcb9da7fa2291818"));
    assert!(!output.contains("not a directory"));
}

#[test]
fn app_state_reports_a_sanitized_failure_when_the_destination_log_is_unavailable() {
    let state_directory = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let fallback = tempdir().unwrap();
    fs::write(destination.path().join("logs"), b"blocked").unwrap();
    let ledger = Ledger::open(state_directory.path().join("ledger.sqlite3")).unwrap();
    let state = AppState::new_with_failure_root(
        ledger,
        destination.path().to_path_buf(),
        true,
        false,
        fallback.path().to_path_buf(),
    )
    .unwrap();
    let error = CoreError::CopyFailed(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "/Volumes/DJI-MIC/private.wav",
    ));

    let outcome = state.report_failure(
        "backup_now",
        "copy",
        &error,
        Some(Transmitter::Tx01),
        Some("/Volumes/DJI-MIC/TX_MIC001_20260810_001116/private.wav"),
    );

    assert_eq!(outcome, FailureWriteOutcome::Fallback);
    let output = fs::read_to_string(fallback.path().join("2026/08/260810-backup-mic.log")).unwrap();
    assert!(output.contains("item=\"private.wav\""));
    assert!(output.contains("error_code=\"copy_failed\""));
    assert!(!output.contains("/Volumes/"));
}

#[test]
fn app_state_reports_adapter_failures_with_the_public_support_code() {
    let state_directory = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let fallback = tempdir().unwrap();
    let ledger = Ledger::open(state_directory.path().join("ledger.sqlite3")).unwrap();
    let state = AppState::new_with_failure_root(
        ledger,
        destination.path().to_path_buf(),
        true,
        false,
        fallback.path().to_path_buf(),
    )
    .unwrap();
    let error = PublicError {
        code: PublicErrorCode::Internal,
        message_code: "autostart_failed".to_owned(),
        retryable: true,
        transmitter: None,
    };

    let outcome = state.report_public_failure("set_autostart", "adapter", &error, None);

    assert_eq!(outcome, FailureWriteOutcome::Primary);
    let output = fs::read_to_string(
        destination
            .path()
            .join("logs/2026/08/260810-backup-mic.log"),
    )
    .unwrap();
    assert!(output.contains("operation=\"set_autostart\""));
    assert!(output.contains("error_code=\"autostart_failed\""));
}
