//! The background queue: one thread transcribing pending recordings one at a time.
//!
//! The thread sleeps until something could have created work — a finished backup, a settings
//! change, a downloaded model, or the user pressing "transcribe now" — and re-derives the queue
//! from the ledger before every file, so a change made mid-run applies to the next recording.

use std::{
    fs,
    sync::{
        Arc, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use backup_core::{
    audit_log::{AuditLevel, AuditValue},
    error::CoreError,
    transcription::{ModelChoice, TranscriptionSettings, whisper_model},
};
use tauri::AppHandle;

use super::{
    TranscriptionService,
    dto::TranscriptionPhaseDto,
    engine::{AppleDecoder, EngineError, PcmDecoder, TranscriptionEngine, WhisperEngine},
    public_error,
    queue::{Candidate, Finished, Job, pending_candidates, transcribe_one},
};
use crate::clock;

/// How long the queue sleeps before checking again on its own, in case a wake was missed.
const RECHECK_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// How often a waiting queue checks whether the backup it is waiting for has finished.
const BACKUP_POLL_INTERVAL: Duration = Duration::from_secs(2);

impl TranscriptionService {
    /// Starts the queue thread. It runs until [`Self::shutdown`].
    pub fn start_worker(&self, app: AppHandle) -> std::io::Result<()> {
        let service = self.clone();
        std::thread::Builder::new()
            .name("transcription-queue".to_owned())
            .spawn(move || {
                // whisper.cpp logs every model load to stderr; route it nowhere.
                whisper_rs::install_logging_hooks();
                let mut engine = WhisperEngine::default();
                while service.wait_for_wake() {
                    service.run_pass(&app, &AppleDecoder, &mut engine);
                    engine.unload();
                }
            })?;
        Ok(())
    }

    /// Asks the queue to look for work.
    pub fn wake(&self) {
        let mut pending = self
            .inner
            .wake
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *pending = true;
        self.inner.wake_signal.notify_one();
    }

    /// Blocks until there may be work. Returns false once the app is shutting down.
    fn wait_for_wake(&self) -> bool {
        let mut pending = self
            .inner
            .wake
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        loop {
            if self.inner.shutting_down.load(Ordering::SeqCst) {
                return false;
            }
            if *pending {
                *pending = false;
                return true;
            }
            let (guard, wait) = self
                .inner
                .wake_signal
                .wait_timeout(pending, RECHECK_INTERVAL)
                .unwrap_or_else(PoisonError::into_inner);
            pending = guard;
            if wait.timed_out() {
                return true;
            }
        }
    }

    /// Clears this session's failures and runs the queue now.
    pub fn request_run(&self) {
        self.inner.skipped.lock().clear();
        self.inner.runtime.lock().error = None;
        self.wake();
    }

    /// Stops the recording being transcribed. Nothing is written for it, and the queue waits
    /// for the next wake instead of moving on.
    pub fn cancel_job(&self) -> bool {
        let runtime = self.inner.runtime.lock();
        runtime.job_cancel.store(true, Ordering::SeqCst);
        runtime.job.phase == TranscriptionPhaseDto::Transcribing
    }

    /// Called while the app quits: abort the current file and stop the thread.
    pub fn shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::SeqCst);
        self.cancel_job();
        self.cancel_download();
        self.wake();
    }

    fn run_pass(
        &self,
        app: &AppHandle,
        decoder: &dyn PcmDecoder,
        engine: &mut dyn TranscriptionEngine,
    ) {
        loop {
            if self.inner.shutting_down.load(Ordering::SeqCst) {
                return;
            }
            let settings = self.settings();
            if !settings.enabled {
                self.set_phase(app, TranscriptionPhaseDto::Idle, 0);
                return;
            }
            if self.inner.state.operation_is_active() {
                self.set_phase(app, TranscriptionPhaseDto::WaitingForBackup, 0);
                if !self.wait_for_backup() {
                    return;
                }
                continue;
            }
            let candidates = match self.pending(&settings) {
                Ok(candidates) => candidates,
                Err(error) => {
                    self.inner.state.report_failure(
                        "transcription",
                        "queue_read",
                        &error,
                        None,
                        None,
                    );
                    self.set_phase(app, TranscriptionPhaseDto::Idle, 0);
                    return;
                }
            };
            let queued = candidates.len() as u64;
            let Some(candidate) = candidates.into_iter().next() else {
                self.set_phase(app, TranscriptionPhaseDto::Idle, 0);
                return;
            };
            if !self.selected_model_ready() {
                self.set_phase(app, TranscriptionPhaseDto::WaitingForModel, queued);
                return;
            }
            let output_root = self.output_directory();
            if fs::create_dir_all(&output_root).is_err() {
                self.fail_pass(app, "transcription_output_failed");
                return;
            }
            let stop = self.transcribe_next(
                app,
                &settings,
                &candidate,
                queued,
                &output_root,
                decoder,
                engine,
            );
            if stop {
                return;
            }
        }
    }

    /// Waits for the running backup to release its slot. Returns false when the app quits
    /// first.
    fn wait_for_backup(&self) -> bool {
        while self.inner.state.operation_is_active() {
            if self.inner.shutting_down.load(Ordering::SeqCst) {
                return false;
            }
            std::thread::sleep(BACKUP_POLL_INTERVAL);
        }
        !self.inner.shutting_down.load(Ordering::SeqCst)
    }

    fn pending(&self, settings: &TranscriptionSettings) -> Result<Vec<Candidate>, CoreError> {
        let recordings = self.inner.state.ledger.lock().verified_recordings()?;
        let skipped = self.inner.skipped.lock().clone();
        Ok(pending_candidates(
            recordings,
            settings,
            &self.inner.state.destination(),
            &self.output_directory(),
            &skipped,
        ))
    }

    /// Transcribes one recording and records the outcome. Returns whether the pass must stop.
    #[allow(clippy::too_many_arguments)]
    fn transcribe_next(
        &self,
        app: &AppHandle,
        settings: &TranscriptionSettings,
        candidate: &Candidate,
        queued: u64,
        output_root: &std::path::Path,
        decoder: &dyn PcmDecoder,
        engine: &mut dyn TranscriptionEngine,
    ) -> bool {
        let cancel = Arc::new(AtomicBool::new(false));
        let item = candidate.display_name();
        {
            let mut runtime = self.inner.runtime.lock();
            runtime.job_cancel = cancel.clone();
            runtime.job.phase = TranscriptionPhaseDto::Transcribing;
            runtime.job.current_item = Some(item.clone());
            runtime.job.percent = 0;
            runtime.job.queued_count = queued;
        }
        self.publish(app);

        let (model_audit, model_label) = match &settings.model {
            ModelChoice::Catalog { id } => whisper_model(id)
                .map_or(("custom", id.clone()), |model| {
                    (model.id, model.label.to_owned())
                }),
            ModelChoice::Custom { .. } => ("custom", "custom".to_owned()),
        };
        let progress = {
            let service = self.clone();
            let app = app.clone();
            Box::new(move |percent: i32| service.set_progress(&app, percent))
        };
        let started = Instant::now();
        let result = transcribe_one(
            candidate,
            &Job {
                output_root,
                model_path: &self.selected_model_path(),
                model_label: &model_label,
                options: &settings.whisper,
                cancel,
            },
            decoder,
            engine,
            progress,
        );
        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.finish_item(app, candidate, &item, model_audit, elapsed, result)
    }

    fn set_progress(&self, app: &AppHandle, percent: i32) {
        let percent = u8::try_from(percent.clamp(0, 100)).unwrap_or(0);
        let changed = {
            let mut runtime = self.inner.runtime.lock();
            let changed = runtime.job.percent != percent;
            runtime.job.percent = percent;
            changed
        };
        if changed {
            self.publish(app);
        }
    }

    fn finish_item(
        &self,
        app: &AppHandle,
        candidate: &Candidate,
        item: &str,
        model: &str,
        duration_ms: u64,
        result: Result<Finished, EngineError>,
    ) -> bool {
        let item_field = crate::app_state::loggable_label(item);
        let stop = match result {
            Ok(finished) => {
                {
                    let mut runtime = self.inner.runtime.lock();
                    runtime.job.completed_count = runtime.job.completed_count.saturating_add(1);
                    runtime.job.last_completed_at = Some(clock::now_string());
                }
                self.audit(
                    AuditLevel::Info,
                    "transcription.completed",
                    &[
                        ("item", AuditValue::Text(item_field)),
                        ("model", AuditValue::Text(model)),
                        ("count", AuditValue::Unsigned(finished.segments as u64)),
                        ("files", AuditValue::Unsigned(finished.files_written as u64)),
                        ("duration_ms", AuditValue::Unsigned(duration_ms)),
                    ],
                );
                false
            }
            Err(EngineError::Cancelled) => {
                self.audit(
                    AuditLevel::Info,
                    "transcription.cancelled",
                    &[("item", AuditValue::Text(item_field))],
                );
                true
            }
            Err(error) => {
                let model_problem = error == EngineError::ModelUnavailable;
                if !model_problem {
                    self.inner
                        .skipped
                        .lock()
                        .insert(candidate.recording_id.clone());
                    let mut runtime = self.inner.runtime.lock();
                    runtime.job.failed_count = runtime.job.failed_count.saturating_add(1);
                }
                let public = public_error(error.message_code(), true);
                self.inner.state.report_public_failure(
                    "transcription",
                    "transcribe",
                    &public,
                    Some(item),
                );
                self.audit(
                    AuditLevel::Warning,
                    "transcription.failed",
                    &[
                        ("item", AuditValue::Text(item_field)),
                        ("model", AuditValue::Text(model)),
                        ("reason", AuditValue::Text(error.message_code())),
                    ],
                );
                self.inner.runtime.lock().error = Some(public);
                model_problem
            }
        };
        {
            let mut runtime = self.inner.runtime.lock();
            runtime.job.current_item = None;
            runtime.job.percent = 0;
            if stop {
                runtime.job.phase = TranscriptionPhaseDto::Idle;
            }
        }
        self.publish(app);
        stop
    }

    fn set_phase(&self, app: &AppHandle, phase: TranscriptionPhaseDto, queued: u64) {
        let changed = {
            let mut runtime = self.inner.runtime.lock();
            let changed = runtime.job.phase != phase || runtime.job.queued_count != queued;
            runtime.job.phase = phase;
            runtime.job.queued_count = queued;
            runtime.job.current_item = None;
            runtime.job.percent = 0;
            changed
        };
        if changed {
            self.publish(app);
        }
    }

    fn fail_pass(&self, app: &AppHandle, message_code: &str) {
        let public = public_error(message_code, true);
        self.inner
            .state
            .report_public_failure("transcription", "queue", &public, None);
        self.inner.runtime.lock().error = Some(public);
        self.set_phase(app, TranscriptionPhaseDto::Idle, 0);
        self.publish(app);
    }
}
