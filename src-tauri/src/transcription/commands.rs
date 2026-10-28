//! The transcription commands React may call.
//!
//! The same boundary as the backup commands: every setting has a typed command, and no command
//! takes a path. Folders and model files arrive only through a dialog Rust opens itself.

use backup_core::{
    audit_log::{AuditLevel, AuditValue},
    error::{CoreError, PublicError},
    transcription::{ModelChoice, TranscriptionSettings, whisper_model},
};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt as _;
use tauri_plugin_opener::OpenerExt as _;

use super::{
    TranscriptionService,
    dto::{TranscriptionFolder, TranscriptionOptionsDto, TranscriptionSnapshotDto},
    models::validate_custom_model,
    public_error,
};
use crate::clock;

pub const REGISTERED_COMMANDS: [&str; 10] = [
    "get_transcription_snapshot",
    "set_transcription_enabled",
    "save_transcription_options",
    "select_whisper_model",
    "choose_custom_whisper_model",
    "choose_transcription_folder",
    "reset_transcription_folder",
    "open_transcription_folder",
    "download_whisper_model",
    "cancel_whisper_model_download",
];

type CommandResult = Result<TranscriptionSnapshotDto, PublicError>;

#[tauri::command]
pub async fn get_transcription_snapshot(service: State<'_, TranscriptionService>) -> CommandResult {
    Ok(service.snapshot())
}

/// Turns transcription on or off. Switching it on records when, so only recordings verified
/// from then on are queued unless the user also asks for the existing ones.
#[tauri::command]
pub async fn set_transcription_enabled(
    app: AppHandle,
    service: State<'_, TranscriptionService>,
    enabled: bool,
) -> CommandResult {
    let now = clock::now_string();
    save(
        &app,
        service.inner(),
        "set_transcription_enabled",
        move |settings| {
            if enabled && !settings.enabled {
                settings.enabled_since = Some(now);
            }
            settings.enabled = enabled;
            Ok(())
        },
    )
    .await?;
    service.audit(
        AuditLevel::Info,
        "transcription.setting_saved",
        &[
            ("setting", AuditValue::Text("transcription")),
            ("enabled", AuditValue::Boolean(enabled)),
        ],
    );
    Ok(service.snapshot())
}

/// Saves the output formats and decoder options together, as the window edits them.
#[tauri::command]
pub async fn save_transcription_options(
    app: AppHandle,
    service: State<'_, TranscriptionService>,
    options: TranscriptionOptionsDto,
) -> CommandResult {
    save(
        &app,
        service.inner(),
        "save_transcription_options",
        move |settings| {
            options.whisper.validate()?;
            settings.include_existing_recordings = options.include_existing_recordings;
            settings.formats = options.formats;
            settings.whisper = options.whisper;
            Ok(())
        },
    )
    .await
}

#[tauri::command]
pub async fn select_whisper_model(
    app: AppHandle,
    service: State<'_, TranscriptionService>,
    model_id: String,
) -> CommandResult {
    let model = whisper_model(&model_id).ok_or_else(|| {
        refused(
            service.inner(),
            "select_whisper_model",
            "request_validation",
            CoreError::InvalidRequest,
        )
    })?;
    save(
        &app,
        service.inner(),
        "select_whisper_model",
        move |settings| {
            settings.model = ModelChoice::Catalog {
                id: model.id.to_owned(),
            };
            Ok(())
        },
    )
    .await
}

/// Lets the user point transcription at a ggml file of their own.
#[tauri::command]
pub async fn choose_custom_whisper_model(
    app: AppHandle,
    service: State<'_, TranscriptionService>,
) -> CommandResult {
    let operation = "choose_custom_whisper_model";
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Whisper 모델 파일 선택")
        .set_directory(service.models_directory())
        .add_filter("Whisper ggml 모델", &["bin"])
        .pick_file(move |selection| {
            let _ = sender.send(selection);
        });
    let selection = receiver.await.map_err(|_| {
        adapter(
            service.inner(),
            operation,
            "model_dialog",
            "model_dialog_failed",
        )
    })?;
    let Some(selection) = selection else {
        return Ok(service.snapshot());
    };
    let path = selection
        .into_path()
        .ok()
        .filter(|path| validate_custom_model(path).is_ok())
        .and_then(|path| std::fs::canonicalize(path).ok())
        .ok_or_else(|| {
            adapter(
                service.inner(),
                operation,
                "model_validation",
                "whisper_model_invalid",
            )
        })?;
    save(&app, service.inner(), operation, move |settings| {
        settings.model = ModelChoice::Custom { path };
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn choose_transcription_folder(
    app: AppHandle,
    service: State<'_, TranscriptionService>,
    folder: TranscriptionFolder,
) -> CommandResult {
    let operation = "choose_transcription_folder";
    refuse_models_change_while_downloading(service.inner(), operation, folder)?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let title = match folder {
        TranscriptionFolder::Output => "전사 결과 폴더 선택",
        TranscriptionFolder::Models => "Whisper 모델 폴더 선택",
    };
    let current = service.folder(folder);
    let mut dialog = app.dialog().file().set_title(title);
    if current.is_dir() {
        dialog = dialog.set_directory(current);
    }
    dialog.pick_folder(move |selection| {
        let _ = sender.send(selection);
    });
    let selection = receiver.await.map_err(|_| {
        adapter(
            service.inner(),
            operation,
            "folder_dialog",
            "folder_dialog_failed",
        )
    })?;
    let Some(selection) = selection else {
        return Ok(service.snapshot());
    };
    let path = selection
        .into_path()
        .ok()
        .and_then(|path| service.prepare_folder(&path).ok())
        .ok_or_else(|| {
            adapter(
                service.inner(),
                operation,
                "folder_validation",
                "transcription_folder_invalid",
            )
        })?;
    save(&app, service.inner(), operation, move |settings| {
        set_folder(settings, folder, Some(path));
        Ok(())
    })
    .await
}

/// Puts a folder back to its default.
#[tauri::command]
pub async fn reset_transcription_folder(
    app: AppHandle,
    service: State<'_, TranscriptionService>,
    folder: TranscriptionFolder,
) -> CommandResult {
    let operation = "reset_transcription_folder";
    refuse_models_change_while_downloading(service.inner(), operation, folder)?;
    save(&app, service.inner(), operation, move |settings| {
        set_folder(settings, folder, None);
        Ok(())
    })
    .await
}

/// Opens a folder in Finder, creating a default one that has not been used yet.
#[tauri::command]
pub async fn open_transcription_folder(
    app: AppHandle,
    service: State<'_, TranscriptionService>,
    folder: TranscriptionFolder,
) -> Result<(), PublicError> {
    let operation = "open_transcription_folder";
    let path = service.folder(folder);
    std::fs::create_dir_all(&path).map_err(|_| {
        adapter(
            service.inner(),
            operation,
            "folder_create",
            "open_folder_failed",
        )
    })?;
    app.opener()
        .open_path(path.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|_| {
            adapter(
                service.inner(),
                operation,
                "opener_adapter",
                "open_folder_failed",
            )
        })
}

#[tauri::command]
pub async fn download_whisper_model(
    app: AppHandle,
    service: State<'_, TranscriptionService>,
    model_id: String,
) -> CommandResult {
    service.start_download(&app, &model_id).map_err(|error| {
        let message_code = match error {
            CoreError::Busy => "model_download_busy",
            _ => "whisper_model_invalid",
        };
        let public = public_error(message_code, matches!(error, CoreError::Busy));
        service.app_state().report_public_failure(
            "download_whisper_model",
            "download_start",
            &public,
            None,
        );
        public
    })?;
    Ok(service.snapshot())
}

#[tauri::command]
pub async fn cancel_whisper_model_download(
    service: State<'_, TranscriptionService>,
) -> CommandResult {
    service.cancel_download();
    Ok(service.snapshot())
}

/// Persists one settings change in a blocking task and publishes the result.
async fn save(
    app: &AppHandle,
    service: &TranscriptionService,
    operation: &'static str,
    change: impl FnOnce(&mut TranscriptionSettings) -> Result<(), CoreError> + Send + 'static,
) -> CommandResult {
    let _save_guard = service.lock_save().await;
    let worker = service.clone();
    let occurred_at = clock::now_string();
    tauri::async_runtime::spawn_blocking(move || worker.update_settings(&occurred_at, change))
        .await
        .map_err(|_| {
            adapter(
                service,
                operation,
                "setting_background_join",
                "setting_save_failed",
            )
        })?
        .map_err(|error| refused(service, operation, "setting_persistence", error))?;
    Ok(service.publish(app))
}

fn set_folder(
    settings: &mut TranscriptionSettings,
    folder: TranscriptionFolder,
    path: Option<std::path::PathBuf>,
) {
    match folder {
        TranscriptionFolder::Output => settings.output_directory = path,
        TranscriptionFolder::Models => settings.models_directory = path,
    }
}

/// A model download writes into the models folder it started with, so that folder cannot move
/// underneath it.
fn refuse_models_change_while_downloading(
    service: &TranscriptionService,
    operation: &'static str,
    folder: TranscriptionFolder,
) -> Result<(), PublicError> {
    if folder == TranscriptionFolder::Models && service.download_active() {
        return Err(adapter(
            service,
            operation,
            "download_active",
            "model_download_busy",
        ));
    }
    Ok(())
}

fn refused(
    service: &TranscriptionService,
    operation: &'static str,
    stage: &'static str,
    error: CoreError,
) -> PublicError {
    service
        .app_state()
        .report_failure(operation, stage, &error, None, None);
    error.public(None)
}

fn adapter(
    service: &TranscriptionService,
    operation: &'static str,
    stage: &'static str,
    message_code: &str,
) -> PublicError {
    let public = public_error(message_code, true);
    service
        .app_state()
        .report_public_failure(operation, stage, &public, None);
    public
}

#[cfg(test)]
mod tests {
    use super::REGISTERED_COMMANDS;

    #[test]
    fn transcription_commands_take_no_paths_or_arbitrary_settings() {
        let source = include_str!("commands.rs");
        let command_source = source.split("#[cfg(test)]").next().unwrap();
        for forbidden in [
            "set_setting",
            "setting_key",
            "path: String",
            "path: PathBuf",
            "Command::",
            "std::process",
        ] {
            assert!(
                !command_source.contains(forbidden),
                "transcription commands must not contain {forbidden}"
            );
        }
        for command in REGISTERED_COMMANDS {
            assert!(
                command_source.contains(&format!("pub async fn {command}(")),
                "{command} is listed but not defined as an async command"
            );
        }
    }
}
