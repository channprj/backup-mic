//! Safe macOS device-monitor facade. The C FFI lives in `ffi`.

pub mod audio;

mod ffi;

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Sender, sync_channel},
    },
    thread::{self, JoinHandle},
};

use super::device_registry::NativeDiskEvent;

pub struct DiskArbitrationMonitor {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl DiskArbitrationMonitor {
    pub fn start(sender: Sender<NativeDiskEvent>) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let (ready_sender, ready_receiver) = sync_channel(1);
        let thread = thread::Builder::new()
            .name("dji-disk-arbitration".to_owned())
            .spawn(move || {
                let result = ffi::run(sender, thread_stop, &ready_sender);
                if let Err(error) = result {
                    let _ = ready_sender.send(Err(error));
                }
            })
            .map_err(|error| error.to_string())?;
        ready_receiver.recv().map_err(|error| error.to_string())??;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for DiskArbitrationMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, path::PathBuf, sync::mpsc, time::Duration};

    use backup_core::{
        device::{DeviceMatch, match_volume},
        ledger::Ledger,
        state::Transmitter,
    };

    use crate::platform::device_registry::{DeviceRegistry, VolumeLifecycleEvent};

    use super::*;

    #[test]
    #[ignore = "requires a connected DJI Mic transmitter volume"]
    fn observes_a_current_dji_volume_without_polling() {
        let (sender, receiver) = mpsc::channel();
        let _monitor = DiskArbitrationMonitor::start(sender).unwrap();
        let mut registry = DeviceRegistry::default();
        let mut found = false;
        for _ in 0..40 {
            let Ok(event) = receiver.recv_timeout(Duration::from_millis(250)) else {
                continue;
            };
            for event in registry.apply(event) {
                if let VolumeLifecycleEvent::Mounted(mounted) = event
                    && match_volume(&mounted.descriptor, &[]) == DeviceMatch::UnpairedCandidate
                {
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        assert!(found, "no connected DJI-shaped volume was observed");
    }

    #[test]
    #[ignore = "requires connected paired transmitters and DJI_MIC_ACCEPTANCE_LEDGER"]
    fn current_dji_volumes_match_the_acceptance_ledger() {
        let ledger_path = std::env::var_os("DJI_MIC_ACCEPTANCE_LEDGER")
            .map(PathBuf::from)
            .expect("DJI_MIC_ACCEPTANCE_LEDGER is required");
        let ledger = Ledger::open(ledger_path).unwrap();
        let paired = ledger.paired_devices().unwrap();
        assert_eq!(
            paired.len(),
            2,
            "acceptance ledger must contain both pairings"
        );

        let (sender, receiver) = mpsc::channel();
        let _monitor = DiskArbitrationMonitor::start(sender).unwrap();
        let mut registry = DeviceRegistry::default();
        let mut trusted = HashSet::new();
        for _ in 0..40 {
            let Ok(event) = receiver.recv_timeout(Duration::from_millis(250)) else {
                continue;
            };
            for event in registry.apply(event) {
                if let VolumeLifecycleEvent::Mounted(mounted) = event
                    && let DeviceMatch::Trusted(transmitter) =
                        match_volume(&mounted.descriptor, &paired)
                {
                    trusted.insert(transmitter);
                }
            }
            if trusted.len() == 2 {
                break;
            }
        }
        assert_eq!(
            trusted,
            HashSet::from([Transmitter::Tx01, Transmitter::Tx02]),
            "both connected volumes must match the stored physical identities"
        );
    }
}
