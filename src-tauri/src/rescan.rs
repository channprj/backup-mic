use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use backup_core::{rule_scanner::RuleScanFingerprint, source::SourceId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RescanDecision {
    Unchanged,
    RequestBackup,
    KeepPending,
}

#[derive(Debug)]
pub struct RescanScheduler {
    interval: Duration,
    next_due: HashMap<SourceId, Instant>,
    fingerprints: HashMap<SourceId, RuleScanFingerprint>,
    pending: HashSet<SourceId>,
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

    pub fn mount(&mut self, source_id: SourceId, fingerprint: RuleScanFingerprint, now: Instant) {
        self.fingerprints.insert(source_id.clone(), fingerprint);
        self.next_due
            .insert(source_id, now.checked_add(self.interval).unwrap_or(now));
    }

    pub fn unmount(&mut self, source_id: &SourceId) {
        self.next_due.remove(source_id);
        self.fingerprints.remove(source_id);
        self.pending.remove(source_id);
    }

    pub fn is_mounted(&self, source_id: &SourceId) -> bool {
        self.fingerprints.contains_key(source_id)
    }

    pub fn due_sources(&self, now: Instant) -> Vec<SourceId> {
        let mut due = self
            .next_due
            .iter()
            .filter_map(|(source_id, deadline)| (*deadline <= now).then_some(source_id.clone()))
            .collect::<Vec<_>>();
        due.sort();
        due
    }

    pub fn defer(&mut self, source_id: &SourceId, now: Instant) {
        if self.next_due.contains_key(source_id) {
            self.next_due.insert(
                source_id.clone(),
                now.checked_add(self.interval).unwrap_or(now),
            );
        }
    }

    pub fn observe(
        &mut self,
        source_id: SourceId,
        fingerprint: RuleScanFingerprint,
        now: Instant,
        operation_busy: bool,
    ) -> RescanDecision {
        self.defer(&source_id, now);
        let Some(previous) = self.fingerprints.get_mut(&source_id) else {
            return RescanDecision::Unchanged;
        };
        if *previous == fingerprint {
            return RescanDecision::Unchanged;
        }
        *previous = fingerprint;
        let already_pending = !self.pending.is_empty();
        self.pending.insert(source_id);
        if operation_busy || already_pending {
            RescanDecision::KeepPending
        } else {
            RescanDecision::RequestBackup
        }
    }

    pub fn has_pending_backup(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn pending_sources(&self) -> Vec<SourceId> {
        let mut sources = self.pending.iter().cloned().collect::<Vec<_>>();
        sources.sort();
        sources
    }

    pub fn mounted_sources(&self) -> Vec<SourceId> {
        let mut sources = self.fingerprints.keys().cloned().collect::<Vec<_>>();
        sources.sort();
        sources
    }

    pub fn mark_backup_started(&mut self) {
        self.pending.clear();
    }
}
