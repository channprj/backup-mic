//! The Disk Arbitration thread: device events and the periodic rescan.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, mpsc};
use std::thread;
use std::thread::JoinHandle;
use std::time::Instant;

use backup_core::error::CoreError;
use backup_core::rule::compile_rule;
use backup_core::rule_scanner::scan_rule_once;
use tauri::AppHandle;

use crate::app_state::AppState;
use crate::clock;
use crate::platform::device_registry::DeviceRegistry;
use crate::platform::macos::DiskArbitrationMonitor;
use crate::rescan::{RescanDecision, RescanScheduler};

use super::run::{BackupTrigger, start_backup};
use super::shared::{adapter_public_error, matched_legacy_transmitter};

pub struct DeviceOrchestrator {
    stop: Arc<AtomicBool>,
    monitor: Option<DiskArbitrationMonitor>,
    thread: Option<JoinHandle<()>>,
}

impl DeviceOrchestrator {
    pub fn start(app: AppHandle, state: AppState) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        let monitor = DiskArbitrationMonitor::start(sender)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("backup-mic-device-events".to_owned())
            .spawn(move || {
                let mut registry = DeviceRegistry::default();
                let mut scheduler = RescanScheduler::new(state.rescan_interval());
                let mut backup_pending = false;
                while !thread_stop.load(Ordering::SeqCst) {
                    match receiver.recv_timeout(std::time::Duration::from_millis(250)) {
                        Ok(event) => {
                            for lifecycle in registry.apply(event) {
                                if state.handle_lifecycle(&app, lifecycle).is_some() {
                                    backup_pending = true;
                                }
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => {
                            let error = adapter_public_error("device_monitor_disconnected", true);
                            state.report_public_failure(
                                "device_lifecycle",
                                "event_channel",
                                &error,
                                None,
                            );
                            break;
                        }
                    }
                    let now = Instant::now();
                    scheduler.set_interval(state.rescan_interval(), now);
                    let matched_sources = state.matched_sources();
                    for source_id in scheduler.mounted_sources() {
                        if !matched_sources.contains_key(&source_id) {
                            scheduler.unmount(&source_id);
                        }
                    }
                    let local_offset = clock::local_offset();
                    for (source_id, matched) in &matched_sources {
                        if !scheduler.is_mounted(source_id) {
                            scheduler.mount(
                                source_id.clone(),
                                matched.initial_fingerprint.clone(),
                                now,
                            );
                        }
                    }
                    for source_id in scheduler.due_sources(now) {
                        if !state.automatic_backup_enabled() {
                            scheduler.defer(&source_id, now);
                            continue;
                        }
                        let Some(matched) = matched_sources.get(&source_id) else {
                            scheduler.unmount(&source_id);
                            continue;
                        };
                        let fingerprint = compile_rule(matched.rule.clone()).and_then(|rule| {
                            scan_rule_once(
                                &matched.authority.descriptor.mount_root,
                                &rule,
                                local_offset,
                            )
                            .map(|scan| scan.fingerprint)
                        });
                        match fingerprint {
                            Ok(fingerprint) => {
                                if matches!(
                                    scheduler.observe(
                                        source_id.clone(),
                                        fingerprint,
                                        now,
                                        state.operation_is_active(),
                                    ),
                                    RescanDecision::RequestBackup | RescanDecision::KeepPending
                                ) {
                                    backup_pending = true;
                                }
                            }
                            Err(error) => {
                                scheduler.defer(&source_id, now);
                                let transmitter = matched_legacy_transmitter(matched);
                                state.report_failure(
                                    "automatic_rescan",
                                    "metadata_scan",
                                    &error,
                                    transmitter,
                                    None,
                                );
                                state.set_error(&app, error, transmitter);
                            }
                        }
                    }
                    if matched_sources
                        .keys()
                        .any(|source_id| state.manual_backup_should_start_for(source_id))
                    {
                        backup_pending = true;
                    }
                    if matched_sources.is_empty() {
                        backup_pending = false;
                    }
                    if backup_pending {
                        let trigger = if matched_sources
                            .keys()
                            .any(|source_id| state.manual_backup_should_start_for(source_id))
                        {
                            BackupTrigger::Manual
                        } else {
                            BackupTrigger::Automatic
                        };
                        match start_backup(app.clone(), state.clone(), trigger) {
                            Ok(()) => {
                                scheduler.mark_backup_started();
                                backup_pending = false;
                            }
                            Err(CoreError::Busy) => {}
                            Err(CoreError::DeviceRemoved) => {
                                backup_pending = false;
                            }
                            Err(error) => {
                                state.report_failure(
                                    "automatic_backup",
                                    "operation_start",
                                    &error,
                                    None,
                                    None,
                                );
                                state.set_error(&app, error, None);
                                scheduler.mark_backup_started();
                                backup_pending = false;
                            }
                        }
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stop,
            monitor: Some(monitor),
            thread: Some(thread),
        })
    }
}

impl Drop for DeviceOrchestrator {
    fn drop(&mut self) {
        self.monitor.take();
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
