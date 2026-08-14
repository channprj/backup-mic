use std::path::{Path, PathBuf};

use backup_core::{
    audit_log::{AuditDurability, AuditEvent, AuditLevel, AuditValue},
    error::{CoreError, PublicError},
    events::{ActivityEntry, ActivitySeverity},
    preferences::PreferenceKey,
    rule::BackupRuleDraft,
    source::SourceId,
    state::Transmitter,
};
use tauri::{AppHandle, Manager as _, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::DialogExt as _;
use tauri_plugin_opener::OpenerExt as _;

use crate::{
    app_state::AppState,
    dto::{AppSnapshotDto, RuleTestResultDto, TrashProposalSummaryDto},
    lifecycle::AppLifecycle,
    orchestrator,
};

pub(crate) use backup_core::initial_setup::DESTINATION_SETTING;

pub const REGISTERED_COMMANDS: [&str; 19] = [
    "get_app_snapshot",
    "backup_now",
    "cancel_backup",
    "choose_destination",
    "save_backup_rule",
    "archive_backup_rule",
    "restore_dji_rule",
    "test_backup_rule",
    "prepare_trash",
    "confirm_trash",
    "set_autostart",
    "open_destination",
    "quit_app",
    "show_settings",
    "set_automatic_backup",
    "set_m4a_conversion",
    "set_automatic_trash",
    "complete_initial_setup",
    "open_logs",
];

#[tauri::command]
pub fn get_app_snapshot(state: State<'_, AppState>) -> Result<AppSnapshotDto, PublicError> {
    state.snapshot_with_activity().map_err(|error| {
        reported_core_error(
            state.inner(),
            "get_app_snapshot",
            "ledger_read",
            error,
            None,
        )
    })
}

#[tauri::command]
pub fn backup_now(app: AppHandle, state: State<'_, AppState>) -> Result<(), PublicError> {
    orchestrator::request_backup(app, state.inner().clone()).map_err(|error| {
        reported_core_error(state.inner(), "backup_now", "operation_start", error, None)
    })
}

#[tauri::command]
pub fn cancel_backup(app: AppHandle, state: State<'_, AppState>) -> Result<(), PublicError> {
    if !state.cancel_active_operation() {
        state.settle_cancelled_operation(&app);
    }
    Ok(())
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
    let reservation = state.reserve_operation().map_err(|error| {
        reported_core_error(
            state,
            "choose_destination",
            "operation_reservation",
            error,
            None,
        )
    })?;
    let guard = reservation.acquire().await;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Backup Mic 폴더 선택")
        .set_directory(state.destination())
        .pick_folder(move |selection| {
            let _ = sender.send(selection);
        });
    let selection = receiver.await.map_err(|_| {
        reported_public_error(
            state,
            "choose_destination",
            "destination_dialog",
            adapter_error("destination_dialog_failed", true),
        )
    })?;
    let Some(selection) = selection else {
        return state.snapshot_with_activity().map_err(|error| {
            reported_core_error(state, "choose_destination", "ledger_read", error, None)
        });
    };
    let _save_guard = state.preference_save.lock().await;
    let selected = selection.into_path().map_err(|_| {
        reported_public_error(
            state,
            "choose_destination",
            "destination_validation",
            adapter_error("destination_invalid", false),
        )
    })?;
    let destination = validate_destination(&selected, state).map_err(|error| {
        reported_public_error(state, "choose_destination", "destination_validation", error)
    })?;
    state
        .persist_destination_for_state(app, destination, &orchestrator::now_string())
        .map_err(|error| {
            reported_core_error(
                state,
                "choose_destination",
                "destination_persistence",
                error,
                None,
            )
        })?;
    if let Err(error) = state.record_activity(
        app,
        activity("destination_changed", None, None, ActivitySeverity::Info),
    ) {
        state.report_failure(
            "choose_destination",
            "activity_persistence",
            &error,
            None,
            None,
        );
    }
    drop(guard);
    if state.automatic_backup_enabled() && state.backup_is_ready() {
        match orchestrator::start_backup(
            app.clone(),
            state.clone(),
            orchestrator::BackupTrigger::Automatic,
        ) {
            Ok(())
            | Err(backup_core::error::CoreError::Busy)
            | Err(backup_core::error::CoreError::InvalidRequest) => {}
            Err(error) => {
                return Err(reported_core_error(
                    state,
                    "choose_destination",
                    "backup_start",
                    error,
                    None,
                ));
            }
        }
    }
    state.snapshot_with_activity().map_err(|error| {
        reported_core_error(state, "choose_destination", "ledger_read", error, None)
    })
}

#[tauri::command]
pub fn save_backup_rule(
    state: State<'_, AppState>,
    draft: BackupRuleDraft,
) -> Result<AppSnapshotDto, PublicError> {
    state
        .save_backup_rule_for_state(draft, &orchestrator::now_string())
        .map_err(|error| {
            reported_core_error(
                state.inner(),
                "save_backup_rule",
                "rule_persistence",
                error,
                None,
            )
        })?;
    state.snapshot_with_activity().map_err(|error| {
        reported_core_error(
            state.inner(),
            "save_backup_rule",
            "ledger_read",
            error,
            None,
        )
    })
}

#[tauri::command]
pub fn archive_backup_rule(
    state: State<'_, AppState>,
    rule_id: String,
) -> Result<AppSnapshotDto, PublicError> {
    state
        .archive_backup_rule_for_state(&rule_id, &orchestrator::now_string())
        .map_err(|error| {
            reported_core_error(
                state.inner(),
                "archive_backup_rule",
                "rule_persistence",
                error,
                None,
            )
        })?;
    state.snapshot_with_activity().map_err(|error| {
        reported_core_error(
            state.inner(),
            "archive_backup_rule",
            "ledger_read",
            error,
            None,
        )
    })
}

#[tauri::command]
pub fn restore_dji_rule(state: State<'_, AppState>) -> Result<AppSnapshotDto, PublicError> {
    state
        .restore_dji_rule_for_state(&orchestrator::now_string())
        .map_err(|error| {
            reported_core_error(
                state.inner(),
                "restore_dji_rule",
                "rule_persistence",
                error,
                None,
            )
        })?;
    state.snapshot_with_activity().map_err(|error| {
        reported_core_error(
            state.inner(),
            "restore_dji_rule",
            "ledger_read",
            error,
            None,
        )
    })
}

#[tauri::command]
pub fn test_backup_rule(
    state: State<'_, AppState>,
    draft: BackupRuleDraft,
) -> Result<RuleTestResultDto, PublicError> {
    state.test_backup_rule_for_state(draft).map_err(|error| {
        reported_core_error(state.inner(), "test_backup_rule", "rule_test", error, None)
    })
}

#[tauri::command]
pub async fn prepare_trash(
    app: AppHandle,
    state: State<'_, AppState>,
    source_id: String,
) -> Result<TrashProposalSummaryDto, PublicError> {
    let source_id = SourceId::parse(&source_id).map_err(|error| {
        reported_core_error(
            state.inner(),
            "prepare_trash",
            "request_validation",
            error,
            None,
        )
    })?;
    let state = state.inner().clone();
    let join_state = state.clone();
    tauri::async_runtime::spawn_blocking(move || {
        match orchestrator::prepare_trash_for_source(&app, &state, &source_id) {
            Ok(summary) => Ok(summary),
            Err(error) => {
                state.report_failure("prepare_trash", "source_revalidation", &error, None, None);
                state.set_source_deletion_error(&app, &error, &source_id);
                Err(state.public_error_for_source(error.public(None), &source_id))
            }
        }
    })
    .await
    .map_err(|_| {
        reported_public_error(
            &join_state,
            "prepare_trash",
            "background_join",
            adapter_error("operation_failed", true),
        )
    })?
}

#[tauri::command]
pub async fn confirm_trash(
    app: AppHandle,
    state: State<'_, AppState>,
    proposal_id: String,
) -> Result<AppSnapshotDto, PublicError> {
    if proposal_id.len() != 36 || !proposal_id.is_ascii() {
        return Err(reported_core_error(
            state.inner(),
            "confirm_trash",
            "request_validation",
            CoreError::InvalidRequest,
            None,
        ));
    }
    let state = state.inner().clone();
    let join_state = state.clone();
    let source_id = state.awaiting_deletion_source();
    let error_source_id = source_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        match orchestrator::confirm_trash(&app, &state, &proposal_id) {
            Ok(()) => state.snapshot_with_activity(),
            Err(error) => {
                state.report_failure("confirm_trash", "trash", &error, None, None);
                if let Some(source_id) = &source_id {
                    state.set_source_deletion_error(&app, &error, source_id);
                }
                Err(error)
            }
        }
    })
    .await
    .map_err(|_| {
        reported_public_error(
            &join_state,
            "confirm_trash",
            "background_join",
            adapter_error("operation_failed", true),
        )
    })?
    .map_err(|error| {
        error_source_id.as_ref().map_or_else(
            || error.public(None),
            |source_id| join_state.public_error_for_source(error.public(None), source_id),
        )
    })
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
    result.map_err(|_| {
        reported_public_error(
            state.inner(),
            "set_autostart",
            "autostart_adapter",
            adapter_error("autostart_failed", true),
        )
    })?;
    let actual = app.autolaunch().is_enabled().map_err(|_| {
        reported_public_error(
            state.inner(),
            "set_autostart",
            "autostart_verification",
            adapter_error("autostart_failed", true),
        )
    })?;
    state.set_autostart(&app, actual);
    state.snapshot_with_activity().map_err(|error| {
        reported_core_error(state.inner(), "set_autostart", "ledger_read", error, None)
    })
}

#[tauri::command]
pub fn open_destination(app: AppHandle, state: State<'_, AppState>) -> Result<(), PublicError> {
    let destination = state.destination();
    app.opener()
        .open_path(destination.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|_| {
            reported_public_error(
                state.inner(),
                "open_destination",
                "opener_adapter",
                adapter_error("open_destination_failed", true),
            )
        })
}

#[tauri::command]
pub fn show_settings(app: AppHandle, state: State<'_, AppState>) -> Result<(), PublicError> {
    let settings = app.get_webview_window("settings").ok_or_else(|| {
        reported_public_error(
            state.inner(),
            "show_settings",
            "window_adapter",
            adapter_error("settings_window_unavailable", true),
        )
    })?;
    if let Some(main) = app.get_webview_window("main") {
        main.hide().map_err(|_| {
            reported_public_error(
                state.inner(),
                "show_settings",
                "window_adapter",
                adapter_error("settings_window_unavailable", true),
            )
        })?;
    }
    settings
        .show()
        .and_then(|()| settings.set_focus())
        .map_err(|_| {
            reported_public_error(
                state.inner(),
                "show_settings",
                "window_adapter",
                adapter_error("settings_window_unavailable", true),
            )
        })
}

#[tauri::command]
pub async fn set_automatic_backup(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<AppSnapshotDto, PublicError> {
    set_preference_for_state(
        state.inner(),
        "set_automatic_backup",
        PreferenceKey::AutomaticBackup,
        enabled,
        orchestrator::now_string(),
    )
    .await
}

#[tauri::command]
pub async fn set_m4a_conversion(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<AppSnapshotDto, PublicError> {
    set_m4a_conversion_for_state(state.inner(), enabled, orchestrator::now_string()).await
}

#[tauri::command]
pub async fn set_automatic_trash(
    state: State<'_, AppState>,
    enabled: bool,
    acknowledged: bool,
) -> Result<AppSnapshotDto, PublicError> {
    set_automatic_trash_for_state(
        state.inner(),
        enabled,
        acknowledged,
        orchestrator::now_string(),
    )
    .await
}

#[tauri::command]
pub async fn complete_initial_setup(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshotDto, PublicError> {
    let _save_guard = state.preference_save.lock().await;
    state
        .complete_initial_setup_for_state(&app, &orchestrator::now_string())
        .map_err(|error| {
            reported_core_error(
                state.inner(),
                "complete_initial_setup",
                "setup_persistence",
                error,
                None,
            )
        })?;
    if state.automatic_backup_enabled() && state.backup_is_ready() {
        match orchestrator::start_backup(
            app,
            state.inner().clone(),
            orchestrator::BackupTrigger::Automatic,
        ) {
            Ok(())
            | Err(backup_core::error::CoreError::Busy)
            | Err(backup_core::error::CoreError::InvalidRequest) => {}
            Err(error) => {
                return Err(reported_core_error(
                    state.inner(),
                    "complete_initial_setup",
                    "backup_start",
                    error,
                    None,
                ));
            }
        }
    }
    state.snapshot_with_activity().map_err(|error| {
        reported_core_error(
            state.inner(),
            "complete_initial_setup",
            "ledger_read",
            error,
            None,
        )
    })
}

#[tauri::command]
pub fn open_logs(app: AppHandle, state: State<'_, AppState>) -> Result<(), PublicError> {
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let logs = state
        .log_directory(time::OffsetDateTime::now_utc().to_offset(offset))
        .map_err(|error| {
            reported_core_error(state.inner(), "open_logs", "log_resolution", error, None)
        })?;
    std::fs::create_dir_all(&logs)
        .map_err(backup_core::error::CoreError::AuditLogUnavailable)
        .map_err(|error| {
            reported_core_error(state.inner(), "open_logs", "log_creation", error, None)
        })?;
    app.opener()
        .open_path(logs.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|_| {
            reported_public_error(
                state.inner(),
                "open_logs",
                "opener_adapter",
                adapter_error("open_logs_failed", true),
            )
        })
}

pub(crate) async fn set_preference_for_state(
    state: &AppState,
    operation: &'static str,
    key: PreferenceKey,
    enabled: bool,
    occurred_at: String,
) -> Result<AppSnapshotDto, PublicError> {
    let _save_guard = state.preference_save.lock().await;
    let worker_state = state.clone();
    let joined = tauri::async_runtime::spawn_blocking(move || {
        let mut ledger = worker_state.ledger.lock();
        ledger.set_preference(key, enabled, &occurred_at)?;
        ledger.read_preferences()
    })
    .await
    .map_err(|_| {
        reported_public_error(
            state,
            operation,
            "setting_background_join",
            adapter_error("setting_save_failed", true),
        )
    })?;
    let preferences = joined.map_err(|error| {
        reported_core_error(state, operation, "setting_persistence", error, None)
    })?;
    let snapshot = state.apply_persisted_preferences(preferences);
    let setting = match key {
        PreferenceKey::AutomaticBackup => "automatic_backup",
        PreferenceKey::M4aConversion => "m4a_conversion",
        PreferenceKey::AutomaticTrash => "automatic_trash",
    };
    let applies = if state.operation_is_active() {
        "next_run"
    } else {
        "next_operation"
    };
    let fields = [
        ("setting", AuditValue::Text(setting)),
        ("enabled", AuditValue::Boolean(enabled)),
        ("applies", AuditValue::Text(applies)),
    ];
    if let Err(error) = state.append_audit(
        &AuditEvent {
            occurred_at: local_now(),
            level: AuditLevel::Info,
            code: "setting.saved",
            transmitter: None,
            fields: &fields,
        },
        AuditDurability::SyncData,
    ) {
        state.report_failure(operation, "setting_audit_log", &error, None, None);
    }
    Ok(snapshot)
}

pub async fn set_m4a_conversion_for_state(
    state: &AppState,
    enabled: bool,
    occurred_at: String,
) -> Result<AppSnapshotDto, PublicError> {
    set_preference_for_state(
        state,
        "set_m4a_conversion",
        PreferenceKey::M4aConversion,
        enabled,
        occurred_at,
    )
    .await
}

async fn set_automatic_trash_for_state(
    state: &AppState,
    enabled: bool,
    acknowledged: bool,
    occurred_at: String,
) -> Result<AppSnapshotDto, PublicError> {
    if enabled && !acknowledged {
        return Err(reported_core_error(
            state,
            "set_automatic_trash",
            "request_validation",
            CoreError::InvalidRequest,
            None,
        ));
    }
    set_preference_for_state(
        state,
        "set_automatic_trash",
        PreferenceKey::AutomaticTrash,
        enabled,
        occurred_at,
    )
    .await
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
    if !state.destination_is_separate_from_mounted_sources(&canonical) {
        return Err(adapter_error("destination_invalid", false));
    }
    Ok(canonical)
}

fn destination_path_is_allowed(path: &Path) -> bool {
    if !path.is_absolute()
        || path.parent().is_none()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return false;
    }
    path.strip_prefix("/Volumes")
        .map_or(true, |relative| relative.components().count() >= 2)
}

fn adapter_error(message_code: &str, retryable: bool) -> PublicError {
    PublicError {
        code: backup_core::error::PublicErrorCode::Internal,
        message_code: message_code.to_owned(),
        retryable,
        transmitter: None,
        source_id: None,
        source_label: None,
    }
}

fn local_now() -> time::OffsetDateTime {
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    time::OffsetDateTime::now_utc().to_offset(offset)
}

fn reported_core_error(
    state: &AppState,
    operation: &'static str,
    stage: &'static str,
    error: CoreError,
    transmitter: Option<Transmitter>,
) -> PublicError {
    state.report_failure(operation, stage, &error, transmitter, None);
    error.public(transmitter)
}

fn reported_public_error(
    state: &AppState,
    operation: &'static str,
    stage: &'static str,
    error: PublicError,
) -> PublicError {
    state.report_public_failure(operation, stage, &error, None);
    error
}

fn activity(
    code: &str,
    source_id: Option<backup_core::source::SourceId>,
    count_value: Option<u64>,
    severity: ActivitySeverity,
) -> ActivityEntry {
    ActivityEntry {
        occurred_at: orchestrator::now_string(),
        code: code.to_owned(),
        source_id,
        source_label: None,
        count_value,
        byte_value: None,
        severity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_exactly_the_nineteen_approved_commands() {
        assert_eq!(REGISTERED_COMMANDS.len(), 19);
        assert!(REGISTERED_COMMANDS.contains(&"cancel_backup"));
        assert!(REGISTERED_COMMANDS.contains(&"complete_initial_setup"));
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

    #[tokio::test]
    async fn automatic_trash_requires_acknowledgement_before_persistence() {
        let state_directory = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let ledger_path = state_directory.path().join("ledger.sqlite3");
        let ledger = backup_core::ledger::Ledger::open(&ledger_path).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();

        let result =
            set_automatic_trash_for_state(&state, true, false, "2026-08-09T00:00:00Z".to_owned())
                .await;

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
        let selected = PathBuf::from("/Users/example/Backups/Backup Mic");
        let persisted = serde_json::to_string(&selected.to_string_lossy()).unwrap();

        assert_eq!(
            persisted_destination(Some(persisted), fallback),
            (selected, true)
        );
    }

    #[test]
    fn external_volume_subdirectory_is_allowed_but_volume_roots_are_not() {
        let fallback = PathBuf::from("/Users/example/Documents/Backup Mic");
        let persisted = serde_json::to_string("/Volumes/External/Backups").unwrap();
        assert_eq!(
            persisted_destination(Some(persisted), fallback),
            (PathBuf::from("/Volumes/External/Backups"), true)
        );
        assert!(!destination_path_is_allowed(Path::new("/Volumes")));
        assert!(!destination_path_is_allowed(Path::new("/Volumes/External")));
        assert!(destination_path_is_allowed(Path::new(
            "/Volumes/990EVO+/labs/audio-records/dji"
        )));
    }
}
