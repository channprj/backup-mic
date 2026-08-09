use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use backup_core::{scanner::ScanFingerprint, state::Transmitter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RescanDecision {
    Unchanged,
    RequestBackup,
    KeepPending,
}

#[derive(Debug)]
pub struct RescanScheduler {
    interval: Duration,
    next_due: HashMap<Transmitter, Instant>,
    fingerprints: HashMap<Transmitter, ScanFingerprint>,
    pending: HashSet<Transmitter>,
}

impl RescanScheduler {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            next_due: HashMap::new(),
            fingerprints: HashMap::new(),
            pending: HashSet::new(),
        }
    }

    pub fn mount(&mut self, transmitter: Transmitter, fingerprint: ScanFingerprint, now: Instant) {
        self.fingerprints.insert(transmitter, fingerprint);
        self.next_due
            .insert(transmitter, now.checked_add(self.interval).unwrap_or(now));
    }

    pub fn unmount(&mut self, transmitter: Transmitter) {
        self.next_due.remove(&transmitter);
        self.fingerprints.remove(&transmitter);
        self.pending.remove(&transmitter);
    }

    pub fn is_mounted(&self, transmitter: Transmitter) -> bool {
        self.fingerprints.contains_key(&transmitter)
    }

    pub fn due_transmitters(&self, now: Instant) -> Vec<Transmitter> {
        let mut due = self
            .next_due
            .iter()
            .filter_map(|(transmitter, deadline)| (*deadline <= now).then_some(*transmitter))
            .collect::<Vec<_>>();
        due.sort_by_key(|transmitter| match transmitter {
            Transmitter::Tx01 => 1,
            Transmitter::Tx02 => 2,
        });
        due
    }

    pub fn defer(&mut self, transmitter: Transmitter, now: Instant) {
        if self.next_due.contains_key(&transmitter) {
            self.next_due
                .insert(transmitter, now.checked_add(self.interval).unwrap_or(now));
        }
    }

    pub fn observe(
        &mut self,
        transmitter: Transmitter,
        fingerprint: ScanFingerprint,
        now: Instant,
        operation_busy: bool,
    ) -> RescanDecision {
        self.defer(transmitter, now);
        let Some(previous) = self.fingerprints.get_mut(&transmitter) else {
            return RescanDecision::Unchanged;
        };
        if *previous == fingerprint {
            return RescanDecision::Unchanged;
        }
        *previous = fingerprint;
        let already_pending = !self.pending.is_empty();
        self.pending.insert(transmitter);
        if operation_busy || already_pending {
            RescanDecision::KeepPending
        } else {
            RescanDecision::RequestBackup
        }
    }

    pub fn has_pending_backup(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn mark_backup_started(&mut self) {
        self.pending.clear();
    }
}
