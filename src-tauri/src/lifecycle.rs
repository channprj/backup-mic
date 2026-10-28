use std::sync::atomic::{AtomicBool, Ordering};

use backup_core::audit_log::{AuditDurability, AuditEvent, AuditLevel, AuditValue};
use backup_core::ledger::UnfinishedBackupRun;

use crate::app_state::{AppState, loggable_label};

#[derive(Default)]
pub struct AppLifecycle {
    quitting: AtomicBool,
}

impl AppLifecycle {
    pub fn request_quit(&self) {
        self.quitting.store(true, Ordering::SeqCst);
    }

    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }
}

pub fn build_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// What startup learned before the ledger forgot it: the build, and every run the previous
/// process left unfinished.
///
/// A run is left `running` only when the process ended without finishing it: a force quit,
/// a crash, or a quit that skipped cancellation. Startup marks those runs interrupted, so this
/// is the only place the log can say that a backup stopped and in which phase.
pub struct StartupRecord {
    pub version: String,
    pub unfinished: Vec<UnfinishedBackupRun>,
}

impl StartupRecord {
    /// Writes the record. Called from the device thread, so a slow destination such as an
    /// unreachable network share delays neither setup nor the menu bar icon.
    pub fn write(&self, state: &AppState) {
        record_start(state, &self.version, &self.unfinished);
    }
}

fn record_start(state: &AppState, version: &str, unfinished: &[UnfinishedBackupRun]) {
    let interrupted = u64::try_from(unfinished.len()).unwrap_or(u64::MAX);
    let fields = [
        ("version", AuditValue::Text(version)),
        ("build", AuditValue::Text(build_profile())),
        ("count", AuditValue::Unsigned(interrupted)),
    ];
    append_lifecycle(state, AuditLevel::Info, "app.started", &fields);
    for run in unfinished {
        let fields = [
            ("run", AuditValue::Text(run.id.get(..8).unwrap_or(&run.id))),
            (
                "source",
                AuditValue::Text(loggable_label(run.source_label.as_deref().unwrap_or(""))),
            ),
            ("stage", AuditValue::Text(&run.batch_phase)),
            ("started_at", AuditValue::Text(&run.started_at)),
            ("reason", AuditValue::Text("previous_process_ended")),
        ];
        append_lifecycle(
            state,
            AuditLevel::Warning,
            "backup.run_interrupted",
            &fields,
        );
    }
}

fn append_lifecycle(
    state: &AppState,
    level: AuditLevel,
    code: &'static str,
    fields: &[(&'static str, AuditValue<'_>)],
) {
    let event = AuditEvent {
        occurred_at: crate::clock::local_now(),
        level,
        code,
        transmitter: None,
        fields,
    };
    match state.append_audit(&event, AuditDurability::Buffered) {
        Ok(_) | Err(backup_core::error::CoreError::InvalidRequest) => {}
        Err(error) => {
            state.report_failure("app_lifecycle", "audit_log", &error, None, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use backup_core::ledger::Ledger;

    use super::*;

    fn daily_log(root: &std::path::Path) -> String {
        let year = std::fs::read_dir(root.join("logs"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let month = std::fs::read_dir(year)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let day = std::fs::read_dir(month)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        std::fs::read_to_string(day).unwrap()
    }

    #[test]
    fn startup_names_the_build_and_each_run_the_last_process_left_unfinished() {
        let state_root = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(state_root.path().join("ledger.sqlite3")).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();

        record_start(
            &state,
            "0.261028.0",
            &[UnfinishedBackupRun {
                id: "3a8a0a38-0c19-4a21-9476-b188beec6bc9".to_owned(),
                batch_phase: "copying".to_owned(),
                started_at: "2026-10-28T05:27:57.263571Z".to_owned(),
                source_label: Some("DJI-MIC-2".to_owned()),
            }],
        );

        let log = daily_log(destination.path());
        let lines = log
            .lines()
            .map(|line| line.split_once(' ').unwrap().1)
            .collect::<Vec<_>>();
        assert_eq!(
            lines,
            vec![
                format!(
                    "INFO app.started version=\"0.261028.0\" build=\"{}\" count=1",
                    build_profile()
                )
                .as_str(),
                "WARN backup.run_interrupted run=\"3a8a0a38\" source=\"DJI-MIC-2\" stage=\"copying\" started_at=\"2026-10-28T05:27:57.263571Z\" reason=\"previous_process_ended\"",
            ]
        );
    }
}
