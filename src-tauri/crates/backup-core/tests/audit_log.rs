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
