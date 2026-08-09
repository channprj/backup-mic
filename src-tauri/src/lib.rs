pub const PRODUCT_NAME: &str = "DJI Mic Backup";

pub mod app_state;
pub mod artifact_pipeline;
pub mod commands;
pub mod dto;
pub mod lifecycle;
pub mod orchestrator;
pub mod pairing;
pub mod platform;
pub mod rescan;
pub mod tray;
pub mod window;

use backup_core::ledger::Ledger;
use commands::{DESTINATION_SETTING, persisted_destination};
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
            commands::choose_destination,
            commands::pair_devices,
            commands::prepare_deletion,
            commands::confirm_deletion,
            commands::set_autostart,
            commands::open_destination,
            commands::quit_app,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let app_data = app.path().app_data_dir()?;
            let documents = app.path().document_dir()?;
            let mut ledger =
                Ledger::open(app_data.join("ledger.sqlite3")).map_err(|error| error.to_string())?;
            ledger
                .mark_interrupted_runs(&orchestrator::now_string())
                .map_err(|error| error.to_string())?;
            let (destination, destination_configured) = persisted_destination(
                ledger
                    .setting(DESTINATION_SETTING)
                    .map_err(|error| error.to_string())?,
                documents.join("DJI-Mic-Mini-2S"),
            );
            let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
            let state = app_state::AppState::new(
                ledger,
                destination,
                destination_configured,
                autostart_enabled,
            )
            .map_err(|error| error.to_string())?;
            let notification_status = match app.notification().permission_state() {
                Ok(PermissionState::Granted) => dto::NotificationStatusDto::Granted,
                Ok(PermissionState::Denied) => dto::NotificationStatusDto::Denied,
                _ => dto::NotificationStatusDto::Unknown,
            };
            app.manage(state.clone());
            state.set_notification_status(app.handle(), notification_status);
            let initial_setup = state.snapshot().setup_state;
            let show_initial_setup = state.should_keep_window_open();
            let destination_dialog_state = state.clone();
            let orchestrator = orchestrator::DeviceOrchestrator::start(app.handle().clone(), state)
                .map_err(|error| format!("device monitor unavailable: {error}"))?;
            app.manage(orchestrator);
            tray::setup(app)?;
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
        .expect("failed to build DJI Mic Backup");
    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event
            && !app.state::<AppLifecycle>().is_quitting()
        {
            api.prevent_exit();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::PRODUCT_NAME;

    #[test]
    fn exposes_the_product_name() {
        assert_eq!(PRODUCT_NAME, "DJI Mic Backup");
    }
}
