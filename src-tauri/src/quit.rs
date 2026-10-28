//! Quitting without abandoning a backup in flight.
//!
//! macOS sends Cmd+Q from the default app menu straight to `terminate:`, which tao does not route
//! through `ExitRequested`, so a quit killed a copy midway and left no trace of it. The app menu
//! therefore carries its own Quit item. Every way out, whether that item, the popover button, or
//! a system terminate, cancels the run, waits briefly for it to record where it stopped, and logs
//! the quit.

use std::time::{Duration, Instant};

use backup_core::audit_log::{AuditLevel, AuditValue};
use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager as _};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};

use crate::app_state::AppState;
use crate::lifecycle::{AppLifecycle, append_lifecycle};

pub const QUIT_MENU_ID: &str = "quit";

/// How long a quit waits for a cancelled backup to close its ledger run and log lines.
/// A copy checks cancellation between 1 MiB reads and stops well inside this; a conversion
/// running in `afconvert` may not, and the quit then proceeds without it.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(5);
const SETTLE_POLL: Duration = Duration::from_millis(50);

/// The default macOS app menu with Quit replaced by an item this app handles.
/// Edit and Window keep the standard shortcuts the settings fields and windows rely on.
pub fn app_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let package = app.package_info();
    let about = AboutMetadata {
        name: Some(package.name.clone()),
        version: Some(package.version.to_string()),
        ..Default::default()
    };
    let quit = MenuItem::with_id(
        app,
        QUIT_MENU_ID,
        format!("Quit {}", package.name),
        true,
        Some("CmdOrCtrl+Q"),
    )?;
    Menu::with_items(
        app,
        &[
            &Submenu::with_items(
                app,
                package.name.clone(),
                true,
                &[
                    &PredefinedMenuItem::about(app, None, Some(about))?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::hide(app, None)?,
                    &PredefinedMenuItem::hide_others(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &quit,
                ],
            )?,
            &Submenu::with_items(
                app,
                "Edit",
                true,
                &[
                    &PredefinedMenuItem::undo(app, None)?,
                    &PredefinedMenuItem::redo(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::cut(app, None)?,
                    &PredefinedMenuItem::copy(app, None)?,
                    &PredefinedMenuItem::paste(app, None)?,
                    &PredefinedMenuItem::select_all(app, None)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "Window",
                true,
                &[
                    &PredefinedMenuItem::minimize(app, None)?,
                    &PredefinedMenuItem::close_window(app, None)?,
                ],
            )?,
        ],
    )
}

/// Quits at once when nothing is running; otherwise asks first, because quitting cancels the
/// backup and the files not yet verified wait for the next run.
pub fn ask_to_quit(app: &AppHandle, origin: &'static str) {
    let lifecycle = app.state::<AppLifecycle>();
    if lifecycle.is_quitting() {
        return;
    }
    let Some(state) = app.try_state::<AppState>() else {
        quit(app, origin);
        return;
    };
    if !state.work_is_active() {
        quit(app, origin);
        return;
    }
    if !lifecycle.begin_quit_confirmation() {
        return;
    }
    if lifecycle.is_quitting() {
        lifecycle.end_quit_confirmation();
        return;
    }
    let handle = app.clone();
    app.dialog()
        .message(
            "진행 중인 백업이 있습니다. 지금 종료하면 백업을 취소합니다. \
             검증을 마친 파일은 그대로 남고, 나머지는 다음 백업에서 이어서 처리합니다.",
        )
        .title("백업 중에 종료할까요?")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "종료".to_owned(),
            "백업 계속".to_owned(),
        ))
        .show(move |confirmed| {
            // The quit is marked before the dialog claim is released, so a Quit pressed in
            // between sees it and does not open a second dialog.
            let lifecycle = handle.state::<AppLifecycle>();
            if confirmed {
                quit(&handle, origin);
                lifecycle.end_quit_confirmation();
                return;
            }
            lifecycle.end_quit_confirmation();
            if !lifecycle.is_quitting()
                && let Some(state) = handle.try_state::<AppState>()
            {
                append_lifecycle(
                    &state,
                    AuditLevel::Info,
                    "app.quit_declined",
                    &[("reason", AuditValue::Text(origin))],
                );
            }
        });
}

/// Cancels whatever runs, records the quit, and exits once the run has settled. Only the first
/// request proceeds; the log line and the wait happen off the caller's thread, which is the
/// main thread for the menu and the popover.
pub fn quit(app: &AppHandle, origin: &'static str) {
    if !app.state::<AppLifecycle>().request_quit() {
        return;
    }
    let Some(state) = app
        .try_state::<AppState>()
        .map(|state| state.inner().clone())
    else {
        app.exit(0);
        return;
    };
    let working = state.close_for_quit();
    let handle = app.clone();
    let waiter = std::thread::Builder::new()
        .name("backup-mic-quit".to_owned())
        .spawn(move || {
            record_quit(&state, origin, working);
            if working {
                settle(&state);
            }
            handle.state::<AppLifecycle>().mark_settled();
            handle.exit(0);
        });
    if waiter.is_err() {
        app.exit(0);
    }
}

/// Handles the exit itself. The process ends as soon as this returns, so any wait happens here
/// on the main thread: for a terminate this app did not start, such as logout or shutdown, and
/// for one that arrives while a quit is still waiting for its cancelled run.
pub fn settle_on_exit(app: &AppHandle) {
    if let Some(transcription) = app.try_state::<crate::transcription::TranscriptionService>() {
        transcription.shutdown();
    }
    let Some(lifecycle) = app.try_state::<AppLifecycle>() else {
        return;
    };
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    if lifecycle.request_quit() {
        let working = state.close_for_quit();
        record_quit(&state, "system_terminate", working);
        if working {
            settle(&state);
        }
    } else if !lifecycle.is_settled() && state.work_is_active() {
        settle(&state);
    }
    lifecycle.mark_settled();
}

/// Written after the quit was marked and the run cancelled, so the run's own lines say
/// `reason="app_quit"` whichever is written first.
fn record_quit(state: &AppState, origin: &'static str, working: bool) {
    append_lifecycle(
        state,
        if working {
            AuditLevel::Warning
        } else {
            AuditLevel::Info
        },
        "app.quit",
        &[
            ("reason", AuditValue::Text(origin)),
            ("active", AuditValue::Boolean(working)),
        ],
    );
}

fn settle(state: &AppState) {
    if !wait_until_idle(state, SETTLE_TIMEOUT, SETTLE_POLL) {
        append_lifecycle(
            state,
            AuditLevel::Warning,
            "app.quit_unsettled",
            &[(
                "duration_ms",
                AuditValue::Unsigned(u64::try_from(SETTLE_TIMEOUT.as_millis()).unwrap_or(u64::MAX)),
            )],
        );
    }
}

fn wait_until_idle(state: &AppState, timeout: Duration, poll: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while state.work_is_active() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(poll);
    }
    true
}

#[cfg(test)]
mod tests {
    use backup_core::ledger::Ledger;

    use super::*;

    fn state() -> (tempfile::TempDir, tempfile::TempDir, AppState) {
        let state_root = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(state_root.path().join("ledger.sqlite3")).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();
        (state_root, destination, state)
    }

    #[test]
    fn a_quit_waits_for_the_cancelled_run_to_release_the_operation() {
        let (_root, _destination, state) = state();
        let guard = state.begin_operation().unwrap();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(guard);
        });

        assert!(wait_until_idle(
            &state,
            Duration::from_secs(5),
            Duration::from_millis(10)
        ));
        worker.join().unwrap();
    }

    #[test]
    fn a_quit_gives_up_on_a_run_that_does_not_stop() {
        let (_root, _destination, state) = state();
        let _guard = state.begin_operation().unwrap();

        assert!(!wait_until_idle(
            &state,
            Duration::from_millis(120),
            Duration::from_millis(10)
        ));
    }

    #[test]
    fn a_quit_records_whether_it_cancelled_a_run() {
        let (_root, destination, state) = state();
        let guard = state.begin_operation().unwrap();

        let working = state.close_for_quit();
        record_quit(&state, "menu", working);
        assert!(guard.cancellation.is_cancelled());
        drop(guard);
        record_quit(&state, "system_terminate", state.close_for_quit());

        let log = std::fs::read_dir(destination.path().join("logs"))
            .unwrap()
            .flat_map(|year| std::fs::read_dir(year.unwrap().path()).unwrap())
            .flat_map(|month| std::fs::read_dir(month.unwrap().path()).unwrap())
            .map(|day| std::fs::read_to_string(day.unwrap().path()).unwrap())
            .collect::<String>();
        let lines = log
            .lines()
            .map(|line| line.split_once(' ').unwrap().1)
            .collect::<Vec<_>>();
        assert_eq!(
            lines,
            vec![
                "WARN app.quit reason=\"menu\" active=true",
                "INFO app.quit reason=\"system_terminate\" active=false",
            ]
        );
    }
}
