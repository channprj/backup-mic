use std::path::{Path, PathBuf};

use backup_core::{
    error::PublicError,
    events::{ActivityEntry, ActivitySeverity},
    preferences::PreferenceKey,
    state::Transmitter,
};
use tauri::{AppHandle, Manager as _, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::DialogExt as _;
use tauri_plugin_opener::OpenerExt as _;

use crate::{
    app_state::AppState,
    dto::{AppSnapshotDto, DeletionProposalSummaryDto},
    lifecycle::AppLifecycle,
    orchestrator,
    pairing::PairingAssignment,
};

pub const REGISTERED_COMMANDS: [&str; 14] = [
    "get_app_snapshot",
    "backup_now",
    "choose_destination",
    "pair_devices",
    "prepare_trash",
    "confirm_trash",
    "set_autostart",
    "open_destination",
    "quit_app",
    "show_settings",
    "set_automatic_backup",
    "set_m4a_conversion",
    "set_automatic_trash",
    "open_logs",
];

pub(crate) const DESTINATION_SETTING: &str = "destination_path";

#[tauri::command]
pub fn get_app_snapshot(state: State<'_, AppState>) -> Result<AppSnapshotDto, PublicError> {
    state
        .snapshot_with_activity()
        .map_err(|error| error.public(None))
}

#[tauri::command]
pub fn backup_now(app: AppHandle, state: State<'_, AppState>) -> Result<(), PublicError> {
    orchestrator::start_backup(app, state.inner().clone()).map_err(|error| error.public(None))
}

#[tauri::command]
pub async fn choose_destination(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshotDto, PublicError> {
    choose_destination_for_state(&app, state.inner()).await
}

pub(crate) async fn choose_destination_for_state(
    app: &AppHandle,
    state: &AppState,
) -> Result<AppSnapshotDto, PublicError> {
    let guard = state
        .begin_operation()
        .map_err(|error| error.public(None))?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("DJI Mic Backup 폴더 선택")
        .set_directory(state.destination())
        .pick_folder(move |selection| {
            let _ = sender.send(selection);
        });
    let selection = receiver
        .await
        .map_err(|_| adapter_error("destination_dialog_failed", true))?;
    let Some(selection) = selection else {
        return state
            .snapshot_with_activity()
            .map_err(|error| error.public(None));
    };
    let selected = selection
        .into_path()
        .map_err(|_| adapter_error("destination_invalid", false))?;
    let destination = validate_destination(&selected, state)?;
    let encoded = serde_json::to_string(&destination.to_string_lossy())
        .map_err(|_| adapter_error("destination_invalid", false))?;
    state
        .ledger
        .lock()
        .set_setting(DESTINATION_SETTING, &encoded, &orchestrator::now_string())
        .map_err(|error| error.public(None))?;
    state.set_destination(app, destination);
    let _ = state.record_activity(
        app,
        activity("destination_changed", None, None, ActivitySeverity::Info),
    );
    drop(guard);
    if state.automatic_backup_enabled() && state.backup_is_ready() {
        match orchestrator::start_backup(app.clone(), state.clone()) {
            Ok(())
            | Err(backup_core::error::CoreError::Busy)
            | Err(backup_core::error::CoreError::InvalidRequest) => {}
            Err(error) => return Err(error.public(None)),
        }
    }
    state
        .snapshot_with_activity()
        .map_err(|error| error.public(None))
}

#[tauri::command]
pub fn pair_devices(
    app: AppHandle,
    state: State<'_, AppState>,
    assignments: Vec<PairingAssignment>,
) -> Result<AppSnapshotDto, PublicError> {
    let guard = state
        .begin_operation()
        .map_err(|error| error.public(None))?;
    let mounted = state
        .pair_devices(&app, &assignments, &orchestrator::now_string())
        .map_err(|error| error.public(None))?;
    let _ = state.record_activity(
        &app,
        activity(
            "devices_paired",
            None,
            u64::try_from(assignments.len()).ok(),
            ActivitySeverity::Success,
        ),
    );
    drop(guard);
    if state.automatic_backup_enabled() && !mounted.is_empty() {
        match orchestrator::start_backup(app, state.inner().clone()) {
            Ok(())
            | Err(backup_core::error::CoreError::Busy)
            | Err(backup_core::error::CoreError::InvalidRequest) => {}
            Err(error) => return Err(error.public(None)),
        }
    }
    state
        .snapshot_with_activity()
        .map_err(|error| error.public(None))
}

#[tauri::command]
pub async fn prepare_trash(
    app: AppHandle,
    state: State<'_, AppState>,
    transmitter: Transmitter,
) -> Result<DeletionProposalSummaryDto, PublicError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        match orchestrator::prepare_trash(&app, &state, transmitter) {
            Ok(summary) => Ok(summary),
            Err(error) => {
                state.set_deletion_error(&app, &error, transmitter);
                Err(error.public(Some(transmitter)))
            }
        }
    })
    .await
    .map_err(|_| adapter_error("operation_failed", true))?
}

#[tauri::command]
pub async fn confirm_trash(
    app: AppHandle,
    state: State<'_, AppState>,
    proposal_id: String,
) -> Result<AppSnapshotDto, PublicError> {
    if proposal_id.len() != 36 || !proposal_id.is_ascii() {
        return Err(backup_core::error::CoreError::InvalidRequest.public(None));
    }
    let state = state.inner().clone();
    let transmitter = state.awaiting_deletion_transmitter();
    tauri::async_runtime::spawn_blocking(move || {
        match orchestrator::confirm_trash(&app, &state, &proposal_id) {
            Ok(()) => state.snapshot_with_activity(),
            Err(error) => {
                if let Some(transmitter) = transmitter {
                    state.set_deletion_error(&app, &error, transmitter);
                }
                Err(error)
            }
        }
    })
    .await
    .map_err(|_| adapter_error("operation_failed", true))?
    .map_err(|error| error.public(None))
}

#[tauri::command]
pub fn set_autostart(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<AppSnapshotDto, PublicError> {
    let result = if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    };
    result.map_err(|_| adapter_error("autostart_failed", true))?;
    let actual = app
        .autolaunch()
        .is_enabled()
        .map_err(|_| adapter_error("autostart_failed", true))?;
    state.set_autostart(&app, actual);
    state
        .snapshot_with_activity()
        .map_err(|error| error.public(None))
}

#[tauri::command]
pub fn open_destination(app: AppHandle, state: State<'_, AppState>) -> Result<(), PublicError> {
    let destination = state.destination();
    app.opener()
        .open_path(destination.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|_| adapter_error("open_destination_failed", true))
}

#[tauri::command]
pub fn show_settings(app: AppHandle) -> Result<(), PublicError> {
    let settings = app
        .get_webview_window("settings")
        .ok_or_else(|| adapter_error("settings_window_unavailable", true))?;
    if let Some(main) = app.get_webview_window("main") {
        main.hide()
            .map_err(|_| adapter_error("settings_window_unavailable", true))?;
    }
    settings
        .show()
        .and_then(|()| settings.set_focus())
        .map_err(|_| adapter_error("settings_window_unavailable", true))
}

#[tauri::command]
pub fn set_automatic_backup(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<AppSnapshotDto, PublicError> {
    set_preference_for_state(
        state.inner(),
        PreferenceKey::AutomaticBackup,
        enabled,
        &orchestrator::now_string(),
    )
}

#[tauri::command]
pub fn set_m4a_conversion(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<AppSnapshotDto, PublicError> {
    set_preference_for_state(
        state.inner(),
        PreferenceKey::M4aConversion,
        enabled,
        &orchestrator::now_string(),
    )
}

#[tauri::command]
pub fn set_automatic_trash(
    state: State<'_, AppState>,
    enabled: bool,
    acknowledged: bool,
) -> Result<AppSnapshotDto, PublicError> {
    set_automatic_trash_for_state(
        state.inner(),
        enabled,
        acknowledged,
        &orchestrator::now_string(),
    )
}

#[tauri::command]
pub fn open_logs(app: AppHandle, state: State<'_, AppState>) -> Result<(), PublicError> {
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let logs = state
        .log_directory(time::OffsetDateTime::now_utc().to_offset(offset))
        .map_err(|error| error.public(None))?;
    std::fs::create_dir_all(&logs)
        .map_err(backup_core::error::CoreError::AuditLogUnavailable)
        .map_err(|error| error.public(None))?;
    app.opener()
        .open_path(logs.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|_| adapter_error("open_logs_failed", true))
}

fn set_preference_for_state(
    state: &AppState,
    key: PreferenceKey,
    enabled: bool,
    occurred_at: &str,
) -> Result<AppSnapshotDto, PublicError> {
    state
        .set_preference(key, enabled, occurred_at)
        .map_err(|error| error.public(None))
}

fn set_automatic_trash_for_state(
    state: &AppState,
    enabled: bool,
    acknowledged: bool,
    occurred_at: &str,
) -> Result<AppSnapshotDto, PublicError> {
    if enabled && !acknowledged {
        return Err(backup_core::error::CoreError::InvalidRequest.public(None));
    }
    set_preference_for_state(state, PreferenceKey::AutomaticTrash, enabled, occurred_at)
}

#[tauri::command]
pub fn quit_app(app: AppHandle, state: State<'_, AppState>, lifecycle: State<'_, AppLifecycle>) {
    state.cancel_active_operation();
    lifecycle.request_quit();
    app.exit(0);
}

pub fn persisted_destination(value: Option<String>, default: PathBuf) -> (PathBuf, bool) {
    let persisted = value
        .and_then(|value| serde_json::from_str::<String>(&value).ok())
        .map(PathBuf::from)
        .filter(|path| destination_path_is_allowed(path));
    persisted.map_or((default, false), |path| (path, true))
}

fn validate_destination(selected: &Path, state: &AppState) -> Result<PathBuf, PublicError> {
    if !destination_path_is_allowed(selected) {
        return Err(adapter_error("destination_invalid", false));
    }
    if let Ok(metadata) = std::fs::symlink_metadata(selected) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(adapter_error("destination_invalid", false));
        }
    } else {
        std::fs::create_dir_all(selected)
            .map_err(|_| adapter_error("destination_unavailable", true))?;
    }
    let canonical = std::fs::canonicalize(selected)
        .map_err(|_| adapter_error("destination_unavailable", true))?;
    if !destination_path_is_allowed(&canonical) {
        return Err(adapter_error("destination_invalid", false));
    }
    let overlaps_source = state.runtime.lock().mounted.values().any(|mounted| {
        std::fs::canonicalize(&mounted.descriptor.mount_root)
            .is_ok_and(|source| canonical.starts_with(&source) || source.starts_with(&canonical))
    });
    if overlaps_source {
        return Err(adapter_error("destination_invalid", false));
    }
    Ok(canonical)
}

fn destination_path_is_allowed(path: &Path) -> bool {
    path.is_absolute() && path.parent().is_some() && !path.starts_with(Path::new("/Volumes"))
}

fn adapter_error(message_code: &str, retryable: bool) -> PublicError {
    PublicError {
        code: backup_core::error::PublicErrorCode::Internal,
        message_code: message_code.to_owned(),
        retryable,
        transmitter: None,
    }
}

fn activity(
    code: &str,
    transmitter: Option<Transmitter>,
    count_value: Option<u64>,
    severity: ActivitySeverity,
) -> ActivityEntry {
    ActivityEntry {
        occurred_at: orchestrator::now_string(),
        code: code.to_owned(),
        transmitter,
        count_value,
        byte_value: None,
        severity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_exactly_the_fourteen_approved_commands() {
        assert_eq!(REGISTERED_COMMANDS.len(), 14);
        let serialized = serde_json::to_string(&REGISTERED_COMMANDS).unwrap();
        for forbidden in [
            "read_file",
            "write_file",
            "delete_paths",
            "run_shell",
            "open_url",
        ] {
            assert!(!serialized.contains(forbidden));
        }
    }

    #[test]
    fn automatic_trash_requires_acknowledgement_before_persistence() {
        let state_directory = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let ledger_path = state_directory.path().join("ledger.sqlite3");
        let ledger = backup_core::ledger::Ledger::open(&ledger_path).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();

        let result = set_automatic_trash_for_state(&state, true, false, "2026-08-09T00:00:00Z");

        assert_eq!(
            result.unwrap_err().code,
            backup_core::error::PublicErrorCode::InvalidRequest
        );
        assert!(!state.snapshot().settings.automatic_trash);
        drop(state);
        let reopened = backup_core::ledger::Ledger::open(ledger_path).unwrap();
        assert!(!reopened.read_preferences().unwrap().automatic_trash);
    }

    #[test]
    fn invalid_persisted_destination_falls_back_without_panicking() {
        let fallback = PathBuf::from("/tmp/fallback");
        assert_eq!(
            persisted_destination(Some("not-json".to_owned()), fallback.clone()),
            (fallback, false)
        );
    }

    #[test]
    fn valid_persisted_destination_skips_first_run_confirmation() {
        let fallback = PathBuf::from("/tmp/fallback");
        let selected = PathBuf::from("/Users/example/Backups/DJI-Mic-Mini-2S");
        let persisted = serde_json::to_string(&selected.to_string_lossy()).unwrap();

        assert_eq!(
            persisted_destination(Some(persisted), fallback),
            (selected, true)
        );
    }

    #[test]
    fn removable_volume_hierarchy_is_never_a_destination() {
        let fallback = PathBuf::from("/Users/example/Documents/DJI-Mic-Mini-2S");
        let persisted = serde_json::to_string("/Volumes/External/Backups").unwrap();
        assert_eq!(
            persisted_destination(Some(persisted), fallback.clone()),
            (fallback, false)
        );
        assert!(!destination_path_is_allowed(Path::new("/Volumes")));
        assert!(!destination_path_is_allowed(Path::new("/Volumes/DJI-MIC")));
    }
}
