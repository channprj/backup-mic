use std::collections::BTreeSet;

use backup_core::source::SourceId;
use parking_lot::Mutex;

#[derive(Debug, Default)]
pub(crate) struct ManualBackupIntent {
    inner: Mutex<IntentState>,
}

#[derive(Debug, Default)]
struct IntentState {
    generation: u64,
    request: RequestState,
}

#[derive(Debug, Default)]
enum RequestState {
    #[default]
    None,
    PendingAny {
        resumed: bool,
    },
    PendingSources(BTreeSet<SourceId>),
    Running {
        generation: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManualBackupClaim {
    generation: u64,
    remaining: BTreeSet<SourceId>,
    resumed: bool,
}

impl ManualBackupClaim {
    pub(crate) fn resumed(&self) -> bool {
        self.resumed
    }
}

impl ManualBackupIntent {
    pub(crate) fn request(&self) {
        let mut inner = self.inner.lock();
        if matches!(inner.request, RequestState::None) {
            inner.request = RequestState::PendingAny { resumed: false };
        }
    }

    pub(crate) fn wait_for_device(&self) {
        let mut inner = self.inner.lock();
        if let RequestState::PendingAny { resumed } = &mut inner.request {
            *resumed = true;
        }
    }

    pub(crate) fn should_start_for(&self, source_id: &SourceId) -> bool {
        match &self.inner.lock().request {
            RequestState::PendingAny { .. } => true,
            RequestState::PendingSources(sources) => sources.contains(source_id),
            RequestState::None | RequestState::Running { .. } => false,
        }
    }

    pub(crate) fn claim(&self, mounted: &[SourceId]) -> Option<ManualBackupClaim> {
        if mounted.is_empty() {
            return None;
        }
        let mounted = mounted.iter().cloned().collect::<BTreeSet<_>>();
        let mut inner = self.inner.lock();
        let (remaining, resumed) = match &inner.request {
            RequestState::PendingAny { resumed } => (BTreeSet::new(), *resumed),
            RequestState::PendingSources(sources) => {
                let claimed = sources
                    .intersection(&mounted)
                    .cloned()
                    .collect::<BTreeSet<_>>();
                if claimed.is_empty() {
                    return None;
                }
                let remaining = sources.difference(&claimed).cloned().collect();
                (remaining, true)
            }
            RequestState::None | RequestState::Running { .. } => return None,
        };
        inner.generation = inner.generation.saturating_add(1);
        let generation = inner.generation;
        inner.request = RequestState::Running { generation };
        Some(ManualBackupClaim {
            generation,
            remaining,
            resumed,
        })
    }

    pub(crate) fn finish(&self, claim: ManualBackupClaim) {
        let mut inner = self.inner.lock();
        if !matches!(inner.request, RequestState::Running { generation } if generation == claim.generation)
        {
            return;
        }
        inner.request = if claim.remaining.is_empty() {
            RequestState::None
        } else {
            RequestState::PendingSources(claim.remaining)
        };
    }

    pub(crate) fn interrupt<I>(&self, sources: I)
    where
        I: IntoIterator<Item = SourceId>,
    {
        let interrupted = sources.into_iter().collect::<BTreeSet<_>>();
        if interrupted.is_empty() {
            return;
        }
        let mut inner = self.inner.lock();
        match &mut inner.request {
            RequestState::None => inner.request = RequestState::PendingSources(interrupted),
            RequestState::PendingAny { resumed } => *resumed = true,
            RequestState::PendingSources(pending) => pending.extend(interrupted),
            RequestState::Running { .. } => {}
        }
    }

    pub(crate) fn interrupt_claim<I>(
        &self,
        mut claim: ManualBackupClaim,
        sources: I,
        cancelled: bool,
    ) where
        I: IntoIterator<Item = SourceId>,
    {
        if cancelled {
            return;
        }
        claim.remaining.extend(sources);
        let mut inner = self.inner.lock();
        if !matches!(inner.request, RequestState::Running { generation } if generation == claim.generation)
        {
            return;
        }
        inner.request = if claim.remaining.is_empty() {
            RequestState::None
        } else {
            RequestState::PendingSources(claim.remaining)
        };
    }

    pub(crate) fn cancel(&self) {
        let mut inner = self.inner.lock();
        inner.generation = inner.generation.saturating_add(1);
        inner.request = RequestState::None;
    }

    pub(crate) fn is_waiting(&self) -> bool {
        matches!(
            self.inner.lock().request,
            RequestState::PendingAny { resumed: true } | RequestState::PendingSources(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use backup_core::source::SourceId;

    use super::ManualBackupIntent;

    fn source(value: u128) -> SourceId {
        SourceId::parse(&uuid::Uuid::from_u128(value).hyphenated().to_string()).unwrap()
    }

    #[test]
    fn disconnected_request_waits_until_a_source_can_be_claimed() {
        let intent = ManualBackupIntent::default();
        intent.request();
        intent.wait_for_device();

        assert!(intent.is_waiting());
        assert!(intent.should_start_for(&source(1)));
        let claim = intent.claim(&[source(1)]).unwrap();
        assert!(claim.resumed());
        assert!(!intent.should_start_for(&source(1)));
    }

    #[test]
    fn interrupted_sources_remain_pending_until_each_reconnects() {
        let intent = ManualBackupIntent::default();
        intent.interrupt([source(1), source(2)]);

        let first = intent.claim(&[source(1)]).unwrap();
        assert!(first.resumed());
        intent.finish(first);
        assert!(!intent.should_start_for(&source(1)));
        assert!(intent.should_start_for(&source(2)));

        let second = intent.claim(&[source(2)]).unwrap();
        intent.finish(second);
        assert!(!intent.is_waiting());
    }

    #[test]
    fn unrelated_mount_cannot_consume_a_targeted_retry() {
        let intent = ManualBackupIntent::default();
        intent.interrupt([source(1)]);

        assert!(intent.claim(&[source(2)]).is_none());
        assert!(intent.should_start_for(&source(1)));
    }

    #[test]
    fn duplicate_claim_is_rejected_while_request_is_running() {
        let intent = ManualBackupIntent::default();
        intent.request();
        let claim = intent.claim(&[source(1)]).unwrap();

        assert!(intent.claim(&[source(1)]).is_none());
        intent.finish(claim);
        assert!(!intent.is_waiting());
    }

    #[test]
    fn cancellation_prevents_a_late_interruption_from_restoring_wait() {
        let intent = ManualBackupIntent::default();
        intent.request();
        let claim = intent.claim(&[source(1)]).unwrap();

        intent.cancel();
        intent.interrupt_claim(claim, [source(1)], true);

        assert!(!intent.is_waiting());
        assert!(!intent.should_start_for(&source(1)));
    }
}
