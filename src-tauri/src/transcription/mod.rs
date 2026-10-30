//! Transcribing backed-up recordings with whisper.cpp: settings, models, and their state.
//!
//! Transcription stays outside the backup run on purpose. A backup's guarantees — copy, verify,
//! convert, retire — must not wait on a model that can take longer than the recording itself,
//! and a failed transcript must never make a verified backup look unfinished.

pub mod commands;
pub mod dto;
mod engine;
pub mod models;
mod output;
mod queue;
mod timeline;
mod translate;
mod wakenote;
mod worker;

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use backup_core::{
    audit_log::{AuditDurability, AuditEvent, AuditLevel, AuditValue},
    error::{CoreError, PublicError, PublicErrorCode},
    transcription::{
        ModelChoice, TranscriptionSettings, WHISPER_MODELS, WhisperModel, whisper_model,
    },
};
use parking_lot::Mutex;
use tauri::{AppHandle, Emitter as _};

use crate::{app_state::AppState, clock, dto::destination_display_for};
use dto::{
    ModelDownloadDto, ModelSourceDto, SelectedModelDto, TranscriptionFolder, TranscriptionJobDto,
    TranscriptionSnapshotDto, WhisperModelDto,
};
use models::{
    CurlFetcher, DownloadError, ModelFetcher, catalog_model_installed, catalog_model_path,
    install_catalog_model, regular_file_len,
};

pub const TRANSCRIPTION_SNAPSHOT_EVENT: &str = "transcription-snapshot-changed";
const DOWNLOAD_PUBLISH_INTERVAL: Duration = Duration::from_millis(500);

/// Where models and transcripts go until the user picks somewhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptionDefaults {
    pub models_directory: PathBuf,
    pub output_directory: PathBuf,
}

#[derive(Clone)]
pub struct TranscriptionService {
    inner: Arc<Inner>,
}

struct Inner {
    state: AppState,
    defaults: TranscriptionDefaults,
    home: Option<PathBuf>,
    wakenote: wakenote::WakeNoteModels,
    runtime: Mutex<Runtime>,
    /// Serialises settings writes, so two quick edits cannot each persist half of a change.
    save: tokio::sync::Mutex<()>,
    /// Set when the queue should look for work; the worker sleeps on `wake_signal` otherwise.
    wake: std::sync::Mutex<bool>,
    wake_signal: std::sync::Condvar,
    shutting_down: AtomicBool,
    /// Recordings that failed during this app session, so one bad file cannot loop forever.
    /// Cleared whenever the settings change or the user asks for a run.
    skipped: Mutex<HashSet<String>>,
}

struct Runtime {
    settings: TranscriptionSettings,
    revision: u64,
    download: Option<ActiveDownload>,
    job: TranscriptionJobDto,
    job_cancel: Arc<AtomicBool>,
    error: Option<PublicError>,
}

struct ActiveDownload {
    model_id: &'static str,
    received_bytes: u64,
    total_bytes: u64,
    cancel: Arc<AtomicBool>,
}

impl TranscriptionService {
    /// Loads the stored settings. A corrupt row is reported and replaced by the defaults — with
    /// transcription off — rather than stopping the backup app from starting.
    pub fn new(state: AppState, defaults: TranscriptionDefaults) -> Self {
        let stored = state.ledger.lock().read_transcription_settings();
        let settings = stored.unwrap_or_else(|error| {
            state.report_failure("transcription_setup", "settings_read", &error, None, None);
            TranscriptionSettings::default()
        });
        let home =
            directories::BaseDirs::new().map(|directories| directories.home_dir().to_owned());
        Self {
            inner: Arc::new(Inner {
                state,
                defaults,
                wakenote: wakenote::WakeNoteModels::new(home.as_deref()),
                home,
                runtime: Mutex::new(Runtime {
                    settings,
                    revision: 1,
                    download: None,
                    job: TranscriptionJobDto::default(),
                    job_cancel: Arc::new(AtomicBool::new(false)),
                    error: None,
                }),
                save: tokio::sync::Mutex::new(()),
                wake: std::sync::Mutex::new(false),
                wake_signal: std::sync::Condvar::new(),
                shutting_down: AtomicBool::new(false),
                skipped: Mutex::new(HashSet::new()),
            }),
        }
    }

    pub fn app_state(&self) -> &AppState {
        &self.inner.state
    }

    pub fn settings(&self) -> TranscriptionSettings {
        self.inner.runtime.lock().settings.clone()
    }

    pub fn models_directory(&self) -> PathBuf {
        self.folder(TranscriptionFolder::Models)
    }

    pub fn output_directory(&self) -> PathBuf {
        self.folder(TranscriptionFolder::Output)
    }

    pub fn folder(&self, folder: TranscriptionFolder) -> PathBuf {
        let runtime = self.inner.runtime.lock();
        let (chosen, default) = match folder {
            TranscriptionFolder::Output => (
                &runtime.settings.output_directory,
                &self.inner.defaults.output_directory,
            ),
            TranscriptionFolder::Models => (
                &runtime.settings.models_directory,
                &self.inner.defaults.models_directory,
            ),
        };
        chosen.clone().unwrap_or_else(|| default.clone())
    }

    fn available_catalog_model(&self, model: &WhisperModel) -> Option<PathBuf> {
        let directory = self.models_directory();
        catalog_model_installed(&directory, model)
            .then(|| catalog_model_path(&directory, model))
            .or_else(|| self.inner.wakenote.verified_path(model))
    }

    fn install_or_reuse_model(
        &self,
        model: &WhisperModel,
        fetcher: &dyn ModelFetcher,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<PathBuf, DownloadError> {
        if let Some(path) = self.available_catalog_model(model) {
            return Ok(path);
        }
        // This method runs on the download thread, including the initial discovery/hash.
        self.inner.wakenote.refresh(&[*model]);
        if cancel.load(Ordering::SeqCst) {
            return Err(DownloadError::Cancelled);
        }
        if let Some(path) = self.available_catalog_model(model) {
            return Ok(path);
        }
        install_catalog_model(&self.models_directory(), model, fetcher, cancel, progress)
    }

    fn refresh_shared_models(&self) {
        let directory = self.models_directory();
        let missing: Vec<_> = WHISPER_MODELS
            .iter()
            .copied()
            .filter(|model| !catalog_model_installed(&directory, model))
            .collect();
        self.inner.wakenote.refresh(&missing);
    }

    /// The model file transcription would load, whether or not it is there yet.
    pub fn selected_model_path(&self) -> PathBuf {
        let model = self.inner.runtime.lock().settings.model.clone();
        match model {
            ModelChoice::Catalog { id } => whisper_model(&id).map_or_else(
                || self.models_directory().join(format!("ggml-{id}.bin")),
                |model| {
                    self.available_catalog_model(model)
                        .unwrap_or_else(|| catalog_model_path(&self.models_directory(), model))
                },
            ),
            ModelChoice::Custom { path } => path,
        }
    }

    /// Whether the selected model can be loaded now: a catalog model must be locally installed
    /// or verified in WakeNote, and a picked file must still be a regular file.
    pub fn selected_model_ready(&self) -> bool {
        let model = self.inner.runtime.lock().settings.model.clone();
        match model {
            ModelChoice::Catalog { id } => whisper_model(&id)
                .is_some_and(|model| self.available_catalog_model(model).is_some()),
            ModelChoice::Custom { path } => regular_file_len(&path).is_some_and(|len| len > 0),
        }
    }

    pub fn snapshot(&self) -> TranscriptionSnapshotDto {
        let (settings, revision, download, job, error) = {
            let runtime = self.inner.runtime.lock();
            (
                runtime.settings.clone(),
                runtime.revision,
                runtime.download.as_ref().map(|download| ModelDownloadDto {
                    model_id: download.model_id.to_owned(),
                    received_bytes: download.received_bytes,
                    total_bytes: download.total_bytes,
                }),
                runtime.job.clone(),
                runtime.error.clone(),
            )
        };
        let models_directory = self.models_directory();
        let output_directory = self.output_directory();
        let models = WHISPER_MODELS
            .iter()
            .map(|model| WhisperModelDto {
                id: model.id.to_owned(),
                label: model.label.to_owned(),
                byte_count: model.byte_count,
                english_only: model.english_only,
                quantized: model.quantized,
                installed: self.available_catalog_model(model).is_some(),
            })
            .collect();
        let selected_model = match &settings.model {
            ModelChoice::Catalog { id } => SelectedModelDto {
                source: ModelSourceDto::Catalog,
                id: Some(id.clone()),
                label: whisper_model(id).map_or_else(|| id.clone(), |model| model.label.into()),
                file_display: self.display(&self.selected_model_path()),
                ready: self.selected_model_ready(),
            },
            ModelChoice::Custom { path } => SelectedModelDto {
                source: ModelSourceDto::Custom,
                id: None,
                label: path
                    .file_name()
                    .map(|name| crate::dto::safe_label(&name.to_string_lossy()))
                    .unwrap_or_default(),
                file_display: self.display(path),
                ready: self.selected_model_ready(),
            },
        };
        TranscriptionSnapshotDto {
            revision,
            enabled: settings.enabled,
            include_existing_recordings: settings.include_existing_recordings,
            output_directory_display: self.display(&output_directory),
            output_directory_is_default: settings.output_directory.is_none(),
            models_directory_display: self.display(&models_directory),
            models_directory_is_default: settings.models_directory.is_none(),
            selected_model,
            models,
            download,
            formats: settings.formats,
            whisper: settings.whisper,
            dictionary_enabled: settings.dictionary_enabled,
            dictionary: settings.dictionary,
            translation: settings.translation,
            job,
            error,
        }
    }

    fn display(&self, path: &Path) -> String {
        destination_display_for(path, self.inner.home.as_deref(), true).unwrap_or_default()
    }

    /// Moves the revision forward and sends the new snapshot to every window.
    pub fn publish(&self, app: &AppHandle) -> TranscriptionSnapshotDto {
        {
            let mut runtime = self.inner.runtime.lock();
            runtime.revision = runtime.revision.saturating_add(1);
        }
        let snapshot = self.snapshot();
        let _ = app.emit(TRANSCRIPTION_SNAPSHOT_EVENT, snapshot.clone());
        snapshot
    }

    pub(crate) async fn lock_save(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.inner.save.lock().await
    }

    /// Applies `change` to a copy of the settings, persists the copy, and only then makes it
    /// current — so the window can never show a value the ledger refused.
    ///
    /// Blocking: it writes SQLite under the ledger lock. Callers on the async runtime run it in
    /// a blocking task while holding [`Self::lock_save`].
    pub fn update_settings(
        &self,
        occurred_at: &str,
        change: impl FnOnce(&mut TranscriptionSettings) -> Result<(), CoreError>,
    ) -> Result<(), CoreError> {
        let mut next = self.settings();
        change(&mut next)?;
        self.inner
            .state
            .ledger
            .lock()
            .set_transcription_settings(&next, occurred_at)?;
        {
            let mut runtime = self.inner.runtime.lock();
            runtime.settings = next;
            runtime.error = None;
        }
        self.inner.skipped.lock().clear();
        self.wake();
        Ok(())
    }

    pub fn download_active(&self) -> bool {
        self.inner.runtime.lock().download.is_some()
    }

    /// Starts downloading a catalog model in the background. One download runs at a time.
    pub fn start_download(&self, app: &AppHandle, model_id: &str) -> Result<(), CoreError> {
        let model = whisper_model(model_id).ok_or(CoreError::InvalidRequest)?;
        let available = self.available_catalog_model(model).is_some();
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut runtime = self.inner.runtime.lock();
            if runtime.download.is_some() {
                return Err(CoreError::Busy);
            }
            if available {
                runtime.error = None;
                drop(runtime);
                self.wake();
                self.publish(app);
                return Ok(());
            }
            runtime.download = Some(ActiveDownload {
                model_id: model.id,
                received_bytes: 0,
                total_bytes: model.byte_count,
                cancel: cancel.clone(),
            });
            runtime.error = None;
        }
        let service = self.clone();
        let thread_app = app.clone();
        let spawned = std::thread::Builder::new()
            .name("whisper-model-download".to_owned())
            .spawn(move || {
                let mut last_published: Option<Instant> = None;
                let result =
                    service.install_or_reuse_model(model, &CurlFetcher, &cancel, &mut |received| {
                        if let Some(download) = service.inner.runtime.lock().download.as_mut() {
                            download.received_bytes = received;
                        }
                        if last_published.is_none_or(|published| {
                            published.elapsed() >= DOWNLOAD_PUBLISH_INTERVAL
                        }) {
                            service.publish(&thread_app);
                            last_published = Some(Instant::now());
                        }
                    });
                service.finish_download(&thread_app, model.id, result.map(|_| ()));
            });
        if spawned.is_err() {
            self.inner.runtime.lock().download = None;
            return Err(CoreError::Busy);
        }
        self.publish(app);
        Ok(())
    }

    fn finish_download(
        &self,
        app: &AppHandle,
        model_id: &'static str,
        result: Result<(), DownloadError>,
    ) {
        self.inner.runtime.lock().download = None;
        match result {
            Ok(()) => {
                self.audit(
                    AuditLevel::Info,
                    if whisper_model(model_id).is_some_and(|model| {
                        !catalog_model_installed(&self.models_directory(), model)
                            && self.inner.wakenote.verified_path(model).is_some()
                    }) {
                        "transcription.model_reused"
                    } else {
                        "transcription.model_downloaded"
                    },
                    &[("model", AuditValue::Text(model_id))],
                );
                self.wake();
            }
            Err(DownloadError::Cancelled) => self.audit(
                AuditLevel::Info,
                "transcription.model_download_cancelled",
                &[("model", AuditValue::Text(model_id))],
            ),
            Err(error) => {
                let public = public_error(error.message_code(), error.retryable());
                self.inner.state.report_public_failure(
                    "download_whisper_model",
                    "model_download",
                    &public,
                    None,
                );
                self.audit(
                    AuditLevel::Warning,
                    "transcription.model_download_failed",
                    &[
                        ("model", AuditValue::Text(model_id)),
                        ("reason", AuditValue::Text(error.message_code())),
                    ],
                );
                self.inner.runtime.lock().error = Some(public);
            }
        }
        self.publish(app);
    }

    /// Stops the running download. Its partial file stays so the next attempt resumes.
    pub fn cancel_download(&self) -> bool {
        self.inner
            .runtime
            .lock()
            .download
            .as_ref()
            .map(|download| download.cancel.store(true, Ordering::SeqCst))
            .is_some()
    }

    /// Validates a folder the user picked and returns the canonical path to store.
    ///
    /// The same rules as the backup folder: an absolute path that is not a bare volume root,
    /// not a link, and not on a connected recorder.
    pub fn prepare_folder(&self, selected: &Path) -> Result<PathBuf, CoreError> {
        if !crate::commands::destination_path_is_allowed(selected) {
            return Err(CoreError::InvalidRequest);
        }
        match fs::symlink_metadata(selected) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(CoreError::InvalidRequest);
            }
            Ok(_) => {}
            Err(_) => fs::create_dir_all(selected).map_err(CoreError::CopyFailed)?,
        }
        let canonical = fs::canonicalize(selected).map_err(CoreError::CopyFailed)?;
        if !crate::commands::destination_path_is_allowed(&canonical)
            || !self
                .inner
                .state
                .destination_is_separate_from_mounted_sources(&canonical)
        {
            return Err(CoreError::InvalidRequest);
        }
        Ok(canonical)
    }

    /// Records a transcription event in the daily audit log next to the backup events.
    pub fn audit(
        &self,
        level: AuditLevel,
        code: &'static str,
        fields: &[(&'static str, AuditValue<'_>)],
    ) {
        let result = self.inner.state.append_audit(
            &AuditEvent {
                occurred_at: clock::local_now(),
                level,
                code,
                transmitter: None,
                fields,
            },
            AuditDurability::Buffered,
        );
        if let Err(error) = result
            && !matches!(error, CoreError::InvalidRequest)
        {
            self.inner
                .state
                .report_failure("transcription", "audit_log", &error, None, None);
        }
    }
}

pub(crate) fn public_error(message_code: &str, retryable: bool) -> PublicError {
    PublicError {
        code: PublicErrorCode::Internal,
        message_code: message_code.to_owned(),
        retryable,
        transmitter: None,
        source_id: None,
        source_label: None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use backup_core::{ledger::Ledger, transcription::WhisperOptions};

    use super::*;

    fn service() -> (TranscriptionService, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(root.path().join("ledger.sqlite3")).unwrap();
        let destination = root.path().join("Backup Mic");
        fs::create_dir_all(&destination).unwrap();
        let state = AppState::new(ledger, destination, true, false).unwrap();
        let mut service = TranscriptionService::new(
            state,
            TranscriptionDefaults {
                models_directory: root.path().join("whisper-models"),
                output_directory: root.path().join("Transcripts"),
            },
        );
        Arc::get_mut(&mut service.inner).unwrap().wakenote =
            wakenote::WakeNoteModels::new(Some(root.path()));
        (service, root)
    }

    fn shared_fixture(root: &Path) -> (WhisperModel, PathBuf) {
        let body = b"lmgg shared service fixture";
        let digest = backup_core::hash::hash_reader(body.as_slice(), |_| {}).unwrap();
        let directory = root.join("Library/Application Support/WakeNote/models");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("whisper-medium.bin");
        fs::write(&path, body).unwrap();
        (
            WhisperModel {
                id: "medium",
                label: "Medium",
                byte_count: digest.size,
                sha256: Box::leak(digest.sha256.into_boxed_str()),
                english_only: false,
                quantized: false,
            },
            path,
        )
    }

    struct NoNetwork;
    impl ModelFetcher for NoNetwork {
        fn fetch(
            &self,
            _: &str,
            _: &Path,
            _: &AtomicBool,
            _: &mut dyn FnMut(u64),
        ) -> Result<(), DownloadError> {
            panic!("an installed WakeNote model must not be downloaded");
        }
    }

    #[test]
    fn a_verified_wakenote_model_is_available_at_its_original_path() {
        let (service, root) = service();
        let (model, path) = shared_fixture(root.path());
        service.inner.wakenote.refresh(&[model]);
        assert_eq!(service.available_catalog_model(&model), Some(path));
        assert!(!service.models_directory().exists());
    }

    #[test]
    fn a_download_request_discovers_wakenote_before_creating_local_files_or_using_the_network() {
        let (service, root) = service();
        let (model, path) = shared_fixture(root.path());
        let result = service
            .install_or_reuse_model(&model, &NoNetwork, &AtomicBool::new(false), &mut |_| {})
            .unwrap();
        assert_eq!(result, path);
        assert!(!service.models_directory().exists());
    }

    #[test]
    fn the_users_model_directory_takes_priority_over_wakenote() {
        let (service, root) = service();
        let (model, _) = shared_fixture(root.path());
        service.inner.wakenote.refresh(&[model]);
        let local = root.path().join("chosen-models");
        fs::create_dir(&local).unwrap();
        let path = local.join("ggml-medium.bin");
        fs::write(&path, b"lmgg shared service fixture").unwrap();
        service
            .update_settings("2026-10-30T04:30:00Z", |settings| {
                settings.models_directory = Some(local);
                Ok(())
            })
            .unwrap();
        assert_eq!(service.available_catalog_model(&model), Some(path.clone()));
        assert_eq!(
            service
                .install_or_reuse_model(&model, &NoNetwork, &AtomicBool::new(false), &mut |_| {})
                .unwrap(),
            path
        );
    }

    #[test]
    fn manually_selected_files_are_preserved_even_when_wakenote_has_a_model() {
        let (service, root) = service();
        let (model, _) = shared_fixture(root.path());
        service.inner.wakenote.refresh(&[model]);
        let custom = root.path().join("my-model.bin");
        fs::write(&custom, b"lmgg custom model").unwrap();
        models::validate_custom_model(&custom).unwrap();
        service
            .update_settings("2026-10-30T04:30:00Z", |settings| {
                settings.model = ModelChoice::Custom {
                    path: custom.clone(),
                };
                Ok(())
            })
            .unwrap();
        assert_eq!(service.selected_model_path(), custom);
        assert!(service.selected_model_ready());
        assert_eq!(
            service.snapshot().selected_model.source,
            ModelSourceDto::Custom
        );
        fs::remove_file(custom).unwrap();
        assert!(
            !service.selected_model_ready(),
            "do not silently replace a missing custom model"
        );
    }

    #[test]
    fn a_fresh_install_selects_medium_and_reports_it_missing() {
        let (service, root) = service();
        let snapshot = service.snapshot();

        assert!(!snapshot.enabled);
        assert_eq!(snapshot.selected_model.id.as_deref(), Some("medium"));
        assert!(!snapshot.selected_model.ready);
        assert!(snapshot.models_directory_is_default);
        assert!(snapshot.output_directory_is_default);
        assert_eq!(snapshot.models.len(), WHISPER_MODELS.len());
        assert!(snapshot.models.iter().all(|model| !model.installed));
        assert_eq!(
            service.selected_model_path(),
            root.path().join("whisper-models/ggml-medium.bin")
        );
    }

    #[test]
    fn a_model_at_its_catalog_size_counts_as_installed() {
        let (service, root) = service();
        let model = whisper_model("medium").unwrap();
        let models = root.path().join("whisper-models");
        fs::create_dir_all(&models).unwrap();
        let file = fs::File::create(models.join("ggml-medium.bin")).unwrap();
        file.set_len(model.byte_count - 1).unwrap();
        assert!(
            !service.selected_model_ready(),
            "a short file is a partial download"
        );

        file.set_len(model.byte_count).unwrap();
        assert!(service.selected_model_ready());
        assert!(service.snapshot().selected_model.ready);
    }

    #[test]
    fn settings_changes_persist_and_a_refused_change_leaves_them_unchanged() {
        let (service, root) = service();
        service
            .update_settings("2026-10-28T01:00:00Z", |settings| {
                settings.enabled = true;
                settings.whisper.language = "ko".to_owned();
                Ok(())
            })
            .unwrap();
        let refused = service.update_settings("2026-10-28T01:01:00Z", |settings| {
            settings.whisper = WhisperOptions {
                beam_size: 0,
                ..WhisperOptions::default()
            };
            Ok(())
        });
        assert!(matches!(refused, Err(CoreError::InvalidRequest)));
        assert_eq!(service.settings().whisper.language, "ko");

        let reopened = Ledger::open(root.path().join("ledger.sqlite3")).unwrap();
        let stored = reopened.read_transcription_settings().unwrap();
        assert!(stored.enabled);
        assert_eq!(stored.whisper.language, "ko");
    }

    #[test]
    fn picked_folders_are_canonical_and_never_links_or_relative() {
        let (service, root) = service();
        let created = service
            .prepare_folder(&root.path().join("new/Transcripts"))
            .unwrap();
        assert!(created.is_dir());
        assert_eq!(created, fs::canonicalize(&created).unwrap());

        assert!(service.prepare_folder(Path::new("relative")).is_err());
        assert!(service.prepare_folder(&root.path().join("a/../b")).is_err());
        #[cfg(unix)]
        {
            let link = root.path().join("link");
            std::os::unix::fs::symlink(&created, &link).unwrap();
            assert!(service.prepare_folder(&link).is_err());
        }
    }

    #[test]
    fn cancelling_without_a_download_reports_nothing_to_cancel() {
        let (service, _root) = service();
        assert!(!service.cancel_download());
        assert!(!service.download_active());
    }

    /// Read-only acceptance against an existing WakeNote installation; never downloads a model.
    /// `BACKUP_MIC_WAKENOTE_HOME=... cargo test --lib -- --ignored reuses_a_real_wakenote_model`
    #[test]
    #[ignore = "requires an installed WakeNote medium model and BACKUP_MIC_WAKENOTE_HOME"]
    fn reuses_a_real_wakenote_model_through_the_service() {
        let home = std::env::var_os("BACKUP_MIC_WAKENOTE_HOME")
            .map(PathBuf::from)
            .expect("set BACKUP_MIC_WAKENOTE_HOME to the WakeNote user's home");
        let (mut service, _root) = service();
        Arc::get_mut(&mut service.inner).unwrap().wakenote =
            wakenote::WakeNoteModels::new(Some(&home));
        service.refresh_shared_models();
        let model = whisper_model("medium").unwrap();
        let expected = home.join("Library/Application Support/WakeNote/models/whisper-medium.bin");
        assert_eq!(service.selected_model_path(), expected);
        assert!(service.selected_model_ready());
        let snapshot = service.snapshot();
        assert!(snapshot.selected_model.ready);
        assert!(
            snapshot
                .models
                .iter()
                .find(|model| model.id == "medium")
                .unwrap()
                .installed
        );
        assert_eq!(
            snapshot.selected_model.file_display,
            service.display(&expected)
        );
        assert_eq!(
            service
                .install_or_reuse_model(model, &NoNetwork, &AtomicBool::new(false), &mut |_| {})
                .unwrap(),
            expected
        );
        assert!(!service.models_directory().exists());
    }

    /// Downloads the real tiny model through `curl`, so it needs the network and is opt-in:
    /// `cargo test -p backup-mic --lib -- --ignored downloads_the_tiny_model`.
    #[test]
    #[ignore = "downloads 78 MB from Hugging Face"]
    fn downloads_the_tiny_model_from_the_pinned_revision() {
        let directory = tempfile::tempdir().unwrap();
        let model = whisper_model("tiny").unwrap();
        let path = install_catalog_model(
            directory.path(),
            model,
            &CurlFetcher,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert!(catalog_model_installed(directory.path(), model));
        models::validate_custom_model(&path).expect("a catalog model starts like a ggml file");
    }
}
