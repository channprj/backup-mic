use std::{fs, sync::Arc, thread};

use backup_core::{
    audit_log::{AuditDurability, AuditEvent, AuditLevel, AuditSink, AuditValue, FileAuditLog},
    error::CoreError,
    state::Transmitter,
};
use tempfile::tempdir;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

fn timestamp() -> OffsetDateTime {
    OffsetDateTime::parse("2026-08-09T20:01:42.613+09:00", &Rfc3339).unwrap()
}

#[test]
fn daily_log_uses_local_calendar_path_and_a_literal_human_readable_line() {
    let destination = tempdir().unwrap();
    let log = FileAuditLog::new(destination.path());
    let fields = [
        (
            "source",
            AuditValue::Text("TX02_MIC003_20260809_195540_edit.wav"),
        ),
        (
            "output",
            AuditValue::Text("2026/2026-08-09/TX02/TX02_MIC003_20260809_195540_edit.m4a"),
        ),
        ("source_bytes", AuditValue::Unsigned(56_859_016)),
        ("format", AuditValue::Text("m4a")),
    ];
    let event = AuditEvent {
        occurred_at: timestamp(),
        level: AuditLevel::Info,
        code: "backup.verified",
        transmitter: Some(Transmitter::Tx02),
        fields: &fields,
    };

    let path = log.append(&event, AuditDurability::SyncData).unwrap();

    assert_eq!(
        path.strip_prefix(destination.path()).unwrap(),
        std::path::Path::new("logs/2026/08/260809-backup-mic.log")
    );
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "2026-08-09T20:01:42.613+09:00 INFO backup.verified tx=TX02 source=\"TX02_MIC003_20260809_195540_edit.wav\" output=\"2026/2026-08-09/TX02/TX02_MIC003_20260809_195540_edit.m4a\" source_bytes=56859016 format=\"m4a\"\n"
    );
}

#[test]
fn filename_control_characters_cannot_forge_a_second_log_line() {
    let destination = tempdir().unwrap();
    let log = FileAuditLog::new(destination.path());
    let fields = [(
        "source",
        AuditValue::Text("recording\"\n2026-01-01 ERROR forged=true\r.wav"),
    )];

    let path = log
        .append(
            &AuditEvent {
                occurred_at: timestamp(),
                level: AuditLevel::Warning,
                code: "backup.deferred",
                transmitter: Some(Transmitter::Tx01),
                fields: &fields,
            },
            AuditDurability::Buffered,
        )
        .unwrap();
    let output = fs::read_to_string(path).unwrap();

    assert_eq!(output.lines().count(), 1);
    assert!(output.contains("recording\\\"\\n2026-01-01 ERROR forged=true\\r.wav"));
}

#[test]
fn private_or_unapproved_fields_are_rejected_before_opening_a_log() {
    let destination = tempdir().unwrap();
    let log = FileAuditLog::new(destination.path());
    for fields in [
        vec![("source", AuditValue::Text("/Volumes/DJI-MIC-2/private.wav"))],
        vec![(
            "reason",
            AuditValue::Text("60fc7226f9fcaa230c4788f44152bb5257a7a18f60a7a4d8bcb9da7fa2291818"),
        )],
        vec![(
            "reason",
            AuditValue::Text("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
        )],
        vec![("volume_uuid", AuditValue::Text("redacted"))],
        vec![("value", AuditValue::Text("/Volumes/DJI-MIC-2/private.wav"))],
    ] {
        let result = log.append(
            &AuditEvent {
                occurred_at: timestamp(),
                level: AuditLevel::Error,
                code: "backup.refused",
                transmitter: None,
                fields: &fields,
            },
            AuditDurability::Buffered,
        );
        assert!(matches!(result, Err(CoreError::InvalidAuditEvent)));
    }
    assert!(!destination.path().join("logs").exists());
}

#[test]
fn a_saved_numeric_setting_records_its_value() {
    let destination = tempdir().unwrap();
    let log = FileAuditLog::new(destination.path());
    let fields = [
        ("setting", AuditValue::Text("free_space_reserve_gib")),
        ("value", AuditValue::Unsigned(25)),
        ("applies", AuditValue::Text("next_operation")),
    ];

    let path = log
        .append(
            &AuditEvent {
                occurred_at: timestamp(),
                level: AuditLevel::Info,
                code: "setting.saved",
                transmitter: None,
                fields: &fields,
            },
            AuditDurability::SyncData,
        )
        .unwrap();

    let line = fs::read_to_string(path).unwrap();
    assert_eq!(
        line,
        "2026-08-09T20:01:42.613+09:00 INFO setting.saved setting=\"free_space_reserve_gib\" value=25 applies=\"next_operation\"\n"
    );
}

#[test]
fn concurrent_writers_append_complete_non_interleaved_events() {
    let destination = tempdir().unwrap();
    let log = Arc::new(FileAuditLog::new(destination.path()));
    let mut writers = Vec::new();
    for index in 0_u64..16 {
        let log = Arc::clone(&log);
        writers.push(thread::spawn(move || {
            let fields = [("count", AuditValue::Unsigned(index))];
            log.append(
                &AuditEvent {
                    occurred_at: timestamp(),
                    level: AuditLevel::Info,
                    code: "scan.complete",
                    transmitter: None,
                    fields: &fields,
                },
                AuditDurability::Buffered,
            )
            .unwrap();
        }));
    }
    for writer in writers {
        writer.join().unwrap();
    }

    let output = fs::read_to_string(
        destination
            .path()
            .join("logs/2026/08/260809-backup-mic.log"),
    )
    .unwrap();
    assert_eq!(output.lines().count(), 16);
    assert!(output.lines().all(|line| line.contains(" scan.complete ")));
}

#[test]
fn core_errors_expose_stable_codes_without_private_source_text() {
    let error = CoreError::CopyFailed(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "/Volumes/private/TX01/secret.wav",
    ));

    assert_eq!(error.diagnostic_code(), "copy_failed");
    assert_eq!(
        error.diagnostic_io_kind(),
        Some(std::io::ErrorKind::PermissionDenied)
    );
    assert_eq!(error.diagnostic_io_kind_code(), Some("permission_denied"));
    assert!(!error.diagnostic_code().contains("/Volumes/"));
}

#[cfg(unix)]
fn append_test_event(log: &FileAuditLog) -> Result<std::path::PathBuf, CoreError> {
    log.append(
        &AuditEvent {
            occurred_at: timestamp(),
            level: AuditLevel::Info,
            code: "scan.complete",
            transmitter: None,
            fields: &[],
        },
        AuditDurability::SyncData,
    )
}

#[cfg(unix)]
#[test]
fn audit_log_refuses_linked_leaf_without_modifying_its_target() {
    use std::os::unix::fs::symlink;

    let fixture = tempdir().unwrap();
    let destination = fixture.path().join("destination");
    let outside = fixture.path().join("outside.txt");
    fs::write(&outside, b"keep this file unchanged").unwrap();
    let log = FileAuditLog::new(&destination);
    let path = log.path_for(timestamp());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    symlink(&outside, &path).unwrap();

    assert!(matches!(
        append_test_event(&log),
        Err(CoreError::AuditLogUnavailable(_))
    ));
    assert_eq!(fs::read(&outside).unwrap(), b"keep this file unchanged");
}

#[cfg(unix)]
#[test]
fn audit_log_refuses_links_at_every_directory_boundary() {
    use std::os::unix::fs::symlink;

    for relative in [
        "destination",
        "destination/logs",
        "destination/logs/2026",
        "destination/logs/2026/08",
    ] {
        let fixture = tempdir().unwrap();
        let outside = fixture.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let link = fixture.path().join(relative);
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        symlink(&outside, &link).unwrap();
        let log = FileAuditLog::new(fixture.path().join("destination"));

        assert!(
            matches!(
                append_test_event(&log),
                Err(CoreError::AuditLogUnavailable(_))
            ),
            "accepted {relative}"
        );
        assert!(log.ensure_directory(timestamp()).is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    }
}

#[cfg(unix)]
#[test]
fn fallback_log_refuses_a_linked_root() {
    use std::os::unix::fs::symlink;

    let fixture = tempdir().unwrap();
    let outside = fixture.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let fallback = fixture.path().join("fallback");
    symlink(&outside, &fallback).unwrap();

    assert!(matches!(
        append_test_event(&FileAuditLog::new_log_root(fallback)),
        Err(CoreError::AuditLogUnavailable(_))
    ));
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn audit_log_refuses_hard_linked_files() {
    let fixture = tempdir().unwrap();
    let outside = fixture.path().join("outside.txt");
    fs::write(&outside, b"untouched").unwrap();
    let log = FileAuditLog::new(fixture.path().join("destination"));
    let path = log.path_for(timestamp());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::hard_link(&outside, &path).unwrap();

    assert!(matches!(
        append_test_event(&log),
        Err(CoreError::AuditLogUnavailable(_))
    ));
    assert_eq!(fs::read(&outside).unwrap(), b"untouched");
}

#[cfg(unix)]
#[test]
fn newly_created_audit_logs_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = tempdir().unwrap();
    let log = FileAuditLog::new(fixture.path());
    let path = append_test_event(&log).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[cfg(unix)]
#[test]
fn audit_log_refuses_dangling_links_and_special_files() {
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;

    let fixture = tempdir().unwrap();
    let log = FileAuditLog::new(fixture.path().join("destination"));
    let path = log.path_for(timestamp());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let outside = fixture.path().join("must-not-be-created");
    symlink(&outside, &path).unwrap();
    assert!(append_test_event(&log).is_err());
    assert!(!outside.exists());
    fs::remove_file(&path).unwrap();

    let _socket = UnixListener::bind(&path).unwrap();
    assert!(append_test_event(&log).is_err());
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(append_test_event(&log).is_err());
}

#[cfg(unix)]
#[test]
fn fallback_log_can_create_missing_parent_directories() {
    let fixture = tempdir().unwrap();
    let log = FileAuditLog::new_log_root(fixture.path().join("missing/parents/fallback"));
    let path = append_test_event(&log).unwrap();
    assert!(fs::read_to_string(path).unwrap().contains("scan.complete"));
}
