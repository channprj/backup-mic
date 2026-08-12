pub const PRODUCT_NAME: &str = "Backup Mic";

pub mod app_state;
pub mod artifact_pipeline;
pub mod commands;
pub mod dto;
pub mod failure_reporter;
pub mod legacy_app_data;
pub mod lifecycle;
pub mod orchestrator;
pub mod pairing;
pub mod platform;
pub mod rescan;
pub mod rule_runtime;
pub mod tray;
pub mod window;

use backup_core::{error::CoreError, ledger::Ledger};
use commands::{DESTINATION_SETTING, persisted_destination};
use failure_reporter::{FailureEvent, FailureReporter};
use legacy_app_data::{LEGACY_BUNDLE_IDENTIFIER, prepare_app_data};
use lifecycle::AppLifecycle;
use tauri::Manager;
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as _};
use tauri_plugin_notification::{NotificationExt as _, PermissionState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(AppLifecycle::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_app_snapshot,
            commands::backup_now,
            commands::cancel_backup,
            commands::choose_destination,
            commands::save_backup_rule,
            commands::archive_backup_rule,
            commands::restore_dji_rule,
            commands::test_backup_rule,
            commands::prepare_trash,
            commands::confirm_trash,
            commands::set_autostart,
            commands::open_destination,
            commands::quit_app,
            commands::show_settings,
            commands::set_automatic_backup,
            commands::set_m4a_conversion,
            commands::set_automatic_trash,
            commands::complete_initial_setup,
            commands::open_logs,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let failure_root = app.path().app_log_dir()?;
            let setup_reporter = FailureReporter::new(&failure_root);
            let app_data = app.path().app_data_dir().inspect_err(|_| {
                report_setup_adapter_failure(&setup_reporter, "app_data_resolution");
            })?;
            let base_directories = directories::BaseDirs::new().ok_or_else(|| {
                report_setup_adapter_failure(&setup_reporter, "legacy_app_data_resolution");
                "user application support directory is unavailable"
            })?;
            let legacy_app_data = base_directories.data_dir().join(LEGACY_BUNDLE_IDENTIFIER);
            prepare_app_data(&app_data, &legacy_app_data)
                .map_err(|error| report_setup_core_failure(&setup_reporter, error))?;
            let documents = app.path().document_dir().inspect_err(|_| {
                report_setup_adapter_failure(&setup_reporter, "documents_resolution");
            })?;
            let mut ledger = Ledger::open(app_data.join("ledger.sqlite3"))
                .map_err(|error| report_setup_core_failure(&setup_reporter, error))?;
            ledger
                .mark_interrupted_runs(&orchestrator::now_string())
                .map_err(|error| report_setup_core_failure(&setup_reporter, error))?;
            let (destination, destination_configured) = persisted_destination(
                ledger
                    .setting(DESTINATION_SETTING)
                    .map_err(|error| report_setup_core_failure(&setup_reporter, error))?,
                documents.join(PRODUCT_NAME),
            );
            let autostart_enabled = match app.autolaunch().is_enabled() {
                Ok(enabled) => enabled,
                Err(_) => {
                    report_setup_adapter_failure(&setup_reporter, "autostart_read");
                    false
                }
            };
            let state = app_state::AppState::new_with_failure_root(
                ledger,
                destination,
                destination_configured,
                autostart_enabled,
                failure_root,
            )
            .map_err(|error| report_setup_core_failure(&setup_reporter, error))?;
            let notification_status = match app.notification().permission_state() {
                Ok(PermissionState::Granted) => dto::NotificationStatusDto::Granted,
                Ok(PermissionState::Denied) => dto::NotificationStatusDto::Denied,
                Ok(_) => dto::NotificationStatusDto::Unknown,
                Err(_) => {
                    report_setup_adapter_failure(&setup_reporter, "notification_permission");
                    dto::NotificationStatusDto::Unknown
                }
            };
            app.manage(state.clone());
            state.set_notification_status(app.handle(), notification_status);
            let initial_setup = state.snapshot().setup_state;
            let show_initial_setup = state.should_keep_window_open();
            let destination_dialog_state = state.clone();
            let orchestrator = orchestrator::DeviceOrchestrator::start(app.handle().clone(), state)
                .map_err(|error| {
                    report_setup_adapter_failure(&setup_reporter, "device_monitor_start");
                    format!("device monitor unavailable: {error}")
                })?;
            app.manage(orchestrator);
            tray::setup(app).inspect_err(|_| {
                report_setup_adapter_failure(&setup_reporter, "tray_setup");
            })?;
            if show_initial_setup {
                tray::show_popover(app.handle());
            }
            if initial_setup == dto::SetupStateDto::NeedsDestination {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
                    let _ = commands::choose_destination_for_state(
                        &app_handle,
                        &destination_dialog_state,
                    )
                    .await;
                });
            }
            Ok(())
        })
        .on_window_event(window::handle_event)
        .build(tauri::generate_context!())
        .expect("failed to build Backup Mic");
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event
            && !app.state::<AppLifecycle>().is_quitting()
        {
            api.prevent_exit();
        }
    });
}

fn report_setup_core_failure(reporter: &FailureReporter, error: CoreError) -> String {
    let public = error.public(None);
    reporter.report(
        None,
        local_now(),
        &FailureEvent {
            operation: "app_setup",
            stage: "startup",
            transmitter: None,
            item_name: None,
            error_code: error.diagnostic_code().to_owned(),
            os_kind: error.diagnostic_io_kind_code().map(str::to_owned),
            retryable: public.retryable,
        },
    );
    error.to_string()
}

fn report_setup_adapter_failure(reporter: &FailureReporter, stage: &'static str) {
    reporter.report(
        None,
        local_now(),
        &FailureEvent {
            operation: "app_setup",
            stage,
            transmitter: None,
            item_name: None,
            error_code: "startup_adapter_failed".to_owned(),
            os_kind: None,
            retryable: true,
        },
    );
}

fn local_now() -> time::OffsetDateTime {
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    time::OffsetDateTime::now_utc().to_offset(offset)
}

#[cfg(test)]
mod tests {
    use super::PRODUCT_NAME;

    #[test]
    fn exposes_the_product_name() {
        assert_eq!(PRODUCT_NAME, "Backup Mic");
    }
}
