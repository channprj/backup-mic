//! The per-run audit trail: which recorder, which files, how long, and where a run stopped.
//!
//! Lines name a run by the first eight characters of its ledger id and a recorder by its volume
//! label, so one search follows a run through the daily log and `backup_runs`. Writing the trail
//! never fails a backup; a line that cannot be written is reported as an audit log failure.

use std::path::Path;
use std::time::{Duration, Instant};

use backup_core::audit_log::{
    AuditDurability, AuditEvent, AuditLevel, AuditValue, text_is_loggable,
};
use backup_core::error::CoreError;

use crate::app_state::{AppState, sanitize_item_name};
use crate::clock;

use super::pipeline::source_failure_is_process_wide;

pub(super) fn short_run_id(run_id: &str) -> &str {
    run_id.get(..8).unwrap_or(run_id)
}

/// A volume label the log accepts; one that looks like an identity is replaced.
pub(super) fn loggable_label(label: &str) -> &str {
    if !label.is_empty() && text_is_loggable(label) {
        label
    } else {
        "unnamed"
    }
}

pub(super) fn duration_ms(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

/// Fields of `backup.run_started`: why the run began and which recorders it covers.
pub(super) fn run_started_fields<'a>(
    trigger: &'static str,
    sources: usize,
    labels: &'a str,
) -> [(&'static str, AuditValue<'a>); 3] {
    [
        ("trigger", AuditValue::Text(trigger)),
        ("sources", AuditValue::Unsigned(count(sources))),
        ("source", AuditValue::Text(labels)),
    ]
}

/// Fields of `backup.run_complete` and `backup.waiting_for_device`. The line is written with
/// `SyncData` and a rejected line fails the run, so every key here must be allowlisted.
pub(super) fn run_complete_fields(
    files: u64,
    mode: &'static str,
    sources: usize,
    elapsed: Duration,
) -> [(&'static str, AuditValue<'static>); 4] {
    [
        ("count", AuditValue::Unsigned(files)),
        ("mode", AuditValue::Text(mode)),
        ("sources", AuditValue::Unsigned(count(sources))),
        ("duration_ms", AuditValue::Unsigned(duration_ms(elapsed))),
    ]
}

fn item_name(path: &Path) -> Option<String> {
    sanitize_item_name(path.to_str()).filter(|name| text_is_loggable(name))
}

pub(super) fn append_trail(
    state: &AppState,
    level: AuditLevel,
    code: &'static str,
    fields: &[(&'static str, AuditValue<'_>)],
) {
    let event = AuditEvent {
        occurred_at: clock::local_now(),
        level,
        code,
        transmitter: None,
        fields,
    };
    if let Err(error) = state.append_audit(&event, AuditDurability::Buffered) {
        state.report_failure("backup_run", "audit_log", &error, None, None);
    }
}

/// Reports a recorder that failed before its run began, during scanning or planning.
pub(super) fn source_not_started(
    state: &AppState,
    label: &str,
    stage: &'static str,
    error: &CoreError,
) {
    if matches!(error, CoreError::DeviceRemoved | CoreError::Cancelled) {
        return;
    }
    state.report_failure_with_context(
        "backup_run",
        stage,
        error,
        None,
        None,
        &[("source", AuditValue::Text(loggable_label(label)))],
    );
}

/// The trail of one recorder's run, from its plan to its outcome.
pub(super) struct SourceRunLog<'a> {
    state: &'a AppState,
    run: String,
    source: String,
    started: Instant,
    stage: &'static str,
    item: Option<String>,
    copied: u64,
    converted: u64,
}

impl<'a> SourceRunLog<'a> {
    pub(super) fn start(
        state: &'a AppState,
        run_id: &str,
        label: &str,
        files: usize,
        copy_files: usize,
        bytes: u64,
    ) -> Self {
        let log = Self {
            state,
            run: short_run_id(run_id).to_owned(),
            source: loggable_label(label).to_owned(),
            started: Instant::now(),
            stage: "run_start",
            item: None,
            copied: 0,
            converted: 0,
        };
        append_trail(
            state,
            AuditLevel::Info,
            "backup.source_started",
            &[
                ("run", AuditValue::Text(&log.run)),
                ("source", AuditValue::Text(&log.source)),
                ("files", AuditValue::Unsigned(count(files))),
                ("copy_files", AuditValue::Unsigned(count(copy_files))),
                ("bytes", AuditValue::Unsigned(bytes)),
            ],
        );
        log
    }

    /// Notes the step in progress, so a failure names the stage and file it interrupted.
    pub(super) fn at(&mut self, stage: &'static str, item: Option<&Path>) {
        self.stage = stage;
        self.item = item.and_then(item_name);
    }

    pub(super) fn file_copied(&mut self, item: &Path, bytes: u64, elapsed: Duration) {
        self.copied = self.copied.saturating_add(1);
        let item = item_name(item);
        append_trail(
            self.state,
            AuditLevel::Info,
            "backup.file_copied",
            &[
                ("run", AuditValue::Text(&self.run)),
                ("source", AuditValue::Text(&self.source)),
                ("item", AuditValue::Text(item.as_deref().unwrap_or("none"))),
                ("bytes", AuditValue::Unsigned(bytes)),
                ("duration_ms", AuditValue::Unsigned(duration_ms(elapsed))),
            ],
        );
    }

    pub(super) fn file_converted(
        &mut self,
        item: &Path,
        source_bytes: u64,
        output_bytes: u64,
        elapsed: Duration,
    ) {
        self.converted = self.converted.saturating_add(1);
        let item = item_name(item);
        append_trail(
            self.state,
            AuditLevel::Info,
            "backup.file_converted",
            &[
                ("run", AuditValue::Text(&self.run)),
                ("source", AuditValue::Text(&self.source)),
                ("item", AuditValue::Text(item.as_deref().unwrap_or("none"))),
                ("source_bytes", AuditValue::Unsigned(source_bytes)),
                ("output_bytes", AuditValue::Unsigned(output_bytes)),
                ("duration_ms", AuditValue::Unsigned(duration_ms(elapsed))),
            ],
        );
    }

    pub(super) fn completed(&self, mode: &'static str, verified_files: u64) {
        append_trail(
            self.state,
            AuditLevel::Info,
            "backup.source_complete",
            &[
                ("run", AuditValue::Text(&self.run)),
                ("source", AuditValue::Text(&self.source)),
                ("mode", AuditValue::Text(mode)),
                ("files", AuditValue::Unsigned(verified_files)),
                ("copied", AuditValue::Unsigned(self.copied)),
                ("converted", AuditValue::Unsigned(self.converted)),
                (
                    "duration_ms",
                    AuditValue::Unsigned(duration_ms(self.started.elapsed())),
                ),
            ],
        );
    }

    /// Records how the run ended early. A removed recorder or a cancel is a warning; anything
    /// else is also reported as a failure with the OS error behind it.
    pub(super) fn failed(&self, error: &CoreError, verified_files: u64) {
        let (level, mode) = match error {
            CoreError::DeviceRemoved => (AuditLevel::Warning, "device_removed"),
            CoreError::Cancelled => (AuditLevel::Warning, "cancelled"),
            _ => (AuditLevel::Error, "partial_failure"),
        };
        if level == AuditLevel::Error && !source_failure_is_process_wide(error) {
            self.state.report_failure_with_context(
                "backup_run",
                self.stage,
                error,
                None,
                self.item.as_deref(),
                &[
                    ("run", AuditValue::Text(&self.run)),
                    ("source", AuditValue::Text(&self.source)),
                ],
            );
        }
        append_trail(
            self.state,
            level,
            "backup.source_complete",
            &[
                ("run", AuditValue::Text(&self.run)),
                ("source", AuditValue::Text(&self.source)),
                ("mode", AuditValue::Text(mode)),
                ("stage", AuditValue::Text(self.stage)),
                (
                    "item",
                    AuditValue::Text(self.item.as_deref().unwrap_or("none")),
                ),
                ("error_code", AuditValue::Text(error.diagnostic_code())),
                ("files", AuditValue::Unsigned(verified_files)),
                ("copied", AuditValue::Unsigned(self.copied)),
                ("converted", AuditValue::Unsigned(self.converted)),
                (
                    "duration_ms",
                    AuditValue::Unsigned(duration_ms(self.started.elapsed())),
                ),
            ],
        );
    }
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use backup_core::audit_log::{AuditSink, FileAuditLog};

    use super::*;

    fn append(code: &'static str, fields: &[(&'static str, AuditValue<'_>)]) -> String {
        let destination = tempfile::tempdir().unwrap();
        let path = FileAuditLog::new(destination.path())
            .append(
                &AuditEvent {
                    occurred_at: time::macros::datetime!(2026-10-28 16:51:38 +09:00),
                    level: AuditLevel::Info,
                    code,
                    transmitter: None,
                    fields,
                },
                AuditDurability::Buffered,
            )
            .unwrap();
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn run_level_lines_pass_the_audit_allowlist() {
        let started = append(
            "backup.run_started",
            &run_started_fields("device_detected", 2, "DJI-MIC-1,DJI-MIC-2"),
        );
        let complete = append(
            "backup.run_complete",
            &run_complete_fields(20, "nothing_new", 2, Duration::from_millis(162_281)),
        );

        assert!(started.ends_with(
            "backup.run_started trigger=\"device_detected\" sources=2 source=\"DJI-MIC-1,DJI-MIC-2\"\n"
        ));
        assert!(complete.ends_with(
            "backup.run_complete count=20 mode=\"nothing_new\" sources=2 duration_ms=162281\n"
        ));
    }

    #[test]
    fn a_run_is_named_by_the_prefix_of_its_ledger_id() {
        assert_eq!(
            short_run_id("3a8a0a38-0c19-4a21-9476-b188beec6bc9"),
            "3a8a0a38"
        );
        assert_eq!(short_run_id("run"), "run");
    }

    #[test]
    fn labels_that_look_like_identities_are_not_logged() {
        assert_eq!(loggable_label("DJI-MIC-2"), "DJI-MIC-2");
        assert_eq!(
            loggable_label("071FEE39-3941-33CD-AC05-532A6A258E2C"),
            "unnamed"
        );
        assert_eq!(loggable_label("/Volumes/DJI-MIC-2"), "unnamed");
        assert_eq!(loggable_label(""), "unnamed");
    }
}
