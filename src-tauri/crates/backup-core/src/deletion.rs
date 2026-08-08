use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use uuid::Uuid;

use crate::{
    error::CoreError,
    filesystem::{is_safe_relative_path, modified_nanos},
    hash::hash_file,
    ledger::{Ledger, PendingDeletionItem},
    state::Transmitter,
};

pub const PROPOSAL_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionContext {
    pub transmitter: Transmitter,
    pub paired_volume_uuid: String,
    pub mount_generation: u64,
    pub scan_generation: u64,
    pub destination_generation: u64,
    pub source_root: PathBuf,
    pub destination_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionCandidate {
    pub recording_id: String,
    pub source_relative_path: PathBuf,
    pub source_size: u64,
    pub source_mtime_ns: i128,
    pub source_sha256: String,
    pub destination_relative_path: PathBuf,
    pub destination_size: u64,
    pub destination_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteDeletionSnapshot {
    pub context: DeletionContext,
    pub candidates: Vec<DeletionCandidate>,
    pub current_source_paths: BTreeSet<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionProposalSummary {
    pub proposal_id: String,
    pub transmitter: Transmitter,
    pub file_count: u64,
    pub byte_count: u64,
    pub expires_in_seconds: u64,
}

pub struct DeletionConfirmation<'a> {
    pub current_context: &'a DeletionContext,
    pub now: Duration,
    pub started_at: &'a str,
    pub finished_at: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalInvalidation {
    DeviceDisappeared,
    MountGenerationChanged,
    NewScanResults,
    BackupStarted,
    DestinationChanged,
    AnotherDeletionAttempt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletionFaultPoint {
    RevalidateSource(usize),
    RevalidateDestination(usize),
    Unlink(usize),
}

pub trait DeletionFaults: Send + Sync {
    fn check(&self, point: DeletionFaultPoint) -> Result<(), CoreError>;
}

#[derive(Debug, Default)]
pub struct NoDeletionFaults;

impl DeletionFaults for NoDeletionFaults {
    fn check(&self, _point: DeletionFaultPoint) -> Result<(), CoreError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletionOutcome {
    Deleted,
    Refused,
    PartiallyDeleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionReport {
    pub run_id: String,
    pub outcome: DeletionOutcome,
    pub deleted_files: u64,
    pub deleted_bytes: u64,
    pub remaining_files: u64,
    pub first_failure_code: Option<String>,
}

#[derive(Debug, Clone)]
struct PrivateProposal {
    context: DeletionContext,
    candidates: Vec<DeletionCandidate>,
    expires_at: Duration,
}

#[derive(Debug, Default)]
pub struct DeletionProposalStore {
    proposals: HashMap<String, PrivateProposal>,
}

impl DeletionProposalStore {
    pub fn prepare(
        &mut self,
        snapshot: CompleteDeletionSnapshot,
        now: Duration,
        deletion_allowed: bool,
        faults: &dyn DeletionFaults,
    ) -> Result<DeletionProposalSummary, CoreError> {
        self.invalidate(ProposalInvalidation::AnotherDeletionAttempt);
        if !deletion_allowed || snapshot.candidates.is_empty() {
            return Err(CoreError::DeletionPreflightRefused);
        }
        let candidate_ids: BTreeSet<&str> = snapshot
            .candidates
            .iter()
            .map(|candidate| candidate.recording_id.as_str())
            .collect();
        let candidate_paths: BTreeSet<PathBuf> = snapshot
            .candidates
            .iter()
            .map(|candidate| candidate.source_relative_path.clone())
            .collect();
        if candidate_ids.len() != snapshot.candidates.len()
            || candidate_paths.len() != snapshot.candidates.len()
            || candidate_paths != snapshot.current_source_paths
        {
            return Err(CoreError::DeletionPreflightRefused);
        }
        let _ = verify_complete_snapshot(&snapshot.context, &snapshot.candidates, faults)
            .map_err(|_| CoreError::DeletionPreflightRefused)?;
        let byte_count = snapshot
            .candidates
            .iter()
            .try_fold(0_u64, |total, candidate| {
                total
                    .checked_add(candidate.source_size)
                    .ok_or(CoreError::DeletionPreflightRefused)
            })?;
        let file_count =
            u64::try_from(snapshot.candidates.len()).map_err(|_| CoreError::InvalidRequest)?;
        let expires_at = now
            .checked_add(PROPOSAL_TTL)
            .ok_or(CoreError::InvalidRequest)?;
        let proposal_id = Uuid::new_v4().to_string();
        let transmitter = snapshot.context.transmitter;
        self.proposals.insert(
            proposal_id.clone(),
            PrivateProposal {
                context: snapshot.context,
                candidates: snapshot.candidates,
                expires_at,
            },
        );
        Ok(DeletionProposalSummary {
            proposal_id,
            transmitter,
            file_count,
            byte_count,
            expires_in_seconds: PROPOSAL_TTL.as_secs(),
        })
    }

    pub fn confirm(
        &mut self,
        proposal_id: &str,
        confirmation: DeletionConfirmation<'_>,
        ledger: &mut Ledger,
        faults: &dyn DeletionFaults,
    ) -> Result<DeletionReport, CoreError> {
        let Some(proposal) = self.proposals.remove(proposal_id) else {
            self.invalidate(ProposalInvalidation::AnotherDeletionAttempt);
            return Err(CoreError::ProposalInvalidated);
        };
        self.invalidate(ProposalInvalidation::AnotherDeletionAttempt);
        if confirmation.now >= proposal.expires_at {
            return Err(CoreError::ProposalExpired);
        }
        if &proposal.context != confirmation.current_context {
            return Err(CoreError::ProposalInvalidated);
        }
        let verified_source_paths =
            verify_complete_snapshot(&proposal.context, &proposal.candidates, faults)
                .map_err(|_| CoreError::DeletionPreflightRefused)?;

        let run_id = Uuid::new_v4().to_string();
        let pending_items: Vec<PendingDeletionItem> = proposal
            .candidates
            .iter()
            .map(|candidate| PendingDeletionItem {
                recording_id: candidate.recording_id.clone(),
                source_size: candidate.source_size,
            })
            .collect();
        ledger.begin_deletion_run(
            &run_id,
            proposal.context.transmitter,
            confirmation.started_at,
            &pending_items,
        )?;

        let total_files =
            u64::try_from(proposal.candidates.len()).map_err(|_| CoreError::InvalidRequest)?;
        let mut deleted_files = 0_u64;
        let mut deleted_bytes = 0_u64;
        for (index, (candidate, source_path)) in proposal
            .candidates
            .iter()
            .zip(verified_source_paths)
            .enumerate()
        {
            let unlink_result = faults
                .check(DeletionFaultPoint::Unlink(index))
                .and_then(|()| fs::remove_file(source_path).map_err(CoreError::CopyFailed));
            if unlink_result.is_err() {
                ledger.record_deletion_failure(
                    &run_id,
                    &candidate.recording_id,
                    "unlink_failed",
                )?;
                let outcome = if deleted_files == 0 {
                    DeletionOutcome::Refused
                } else {
                    DeletionOutcome::PartiallyDeleted
                };
                let outcome_name = if outcome == DeletionOutcome::Refused {
                    "refused"
                } else {
                    "partially_deleted"
                };
                ledger.finish_deletion_run(
                    &run_id,
                    confirmation.finished_at,
                    outcome_name,
                    Some("unlink_failed"),
                )?;
                return Ok(DeletionReport {
                    run_id,
                    outcome,
                    deleted_files,
                    deleted_bytes,
                    remaining_files: total_files.saturating_sub(deleted_files),
                    first_failure_code: Some("unlink_failed".to_owned()),
                });
            }
            ledger.record_deletion_success(
                &run_id,
                &candidate.recording_id,
                confirmation.finished_at,
            )?;
            deleted_files = deleted_files
                .checked_add(1)
                .ok_or(CoreError::InvalidRequest)?;
            deleted_bytes = deleted_bytes
                .checked_add(candidate.source_size)
                .ok_or(CoreError::InvalidRequest)?;
        }
        ledger.finish_deletion_run(&run_id, confirmation.finished_at, "deleted", None)?;
        Ok(DeletionReport {
            run_id,
            outcome: DeletionOutcome::Deleted,
            deleted_files,
            deleted_bytes,
            remaining_files: 0,
            first_failure_code: None,
        })
    }

    pub fn invalidate(&mut self, _reason: ProposalInvalidation) {
        self.proposals.clear();
    }
}

fn verify_complete_snapshot(
    context: &DeletionContext,
    candidates: &[DeletionCandidate],
    faults: &dyn DeletionFaults,
) -> Result<Vec<PathBuf>, CoreError> {
    let source_root = fs::canonicalize(&context.source_root).map_err(CoreError::CopyFailed)?;
    let destination_root =
        fs::canonicalize(&context.destination_root).map_err(CoreError::CopyFailed)?;
    let mut source_paths = Vec::with_capacity(candidates.len());
    for (index, candidate) in candidates.iter().enumerate() {
        if candidate.source_sha256 != candidate.destination_sha256
            || candidate.source_size != candidate.destination_size
            || !is_safe_relative_path(&candidate.source_relative_path)
            || !is_safe_relative_path(&candidate.destination_relative_path)
        {
            return Err(CoreError::DeletionPreflightRefused);
        }
        faults.check(DeletionFaultPoint::RevalidateSource(index))?;
        source_paths.push(verify_source(&source_root, candidate)?);
        faults.check(DeletionFaultPoint::RevalidateDestination(index))?;
        verify_destination(&destination_root, candidate)?;
    }
    Ok(source_paths)
}

fn verify_source(root: &Path, candidate: &DeletionCandidate) -> Result<PathBuf, CoreError> {
    let path = root.join(&candidate.source_relative_path);
    let metadata = fs::symlink_metadata(&path).map_err(CoreError::CopyFailed)?;
    if !metadata.file_type().is_file()
        || metadata.len() != candidate.source_size
        || modified_nanos(&metadata)? != candidate.source_mtime_ns
    {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let canonical_path = fs::canonicalize(&path).map_err(CoreError::CopyFailed)?;
    if !canonical_path.starts_with(root) {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let digest = hash_file(&canonical_path)?;
    if digest.size != candidate.source_size || digest.sha256 != candidate.source_sha256 {
        return Err(CoreError::DeletionPreflightRefused);
    }
    Ok(canonical_path)
}

fn verify_destination(root: &Path, candidate: &DeletionCandidate) -> Result<(), CoreError> {
    let path = root.join(&candidate.destination_relative_path);
    let metadata = fs::symlink_metadata(&path).map_err(CoreError::CopyFailed)?;
    if !metadata.file_type().is_file() || metadata.len() != candidate.destination_size {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let canonical_path = fs::canonicalize(&path).map_err(CoreError::CopyFailed)?;
    if !canonical_path.starts_with(root) {
        return Err(CoreError::DeletionPreflightRefused);
    }
    let digest = hash_file(&canonical_path)?;
    if digest.size != candidate.destination_size || digest.sha256 != candidate.destination_sha256 {
        return Err(CoreError::DeletionPreflightRefused);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use proptest::prelude::*;
    use tempfile::tempdir;

    use crate::{filesystem::modified_nanos, hash::hash_file, ledger::VerifiedRecording};

    use super::*;

    struct Fixture {
        source: tempfile::TempDir,
        destination: tempfile::TempDir,
        _state: tempfile::TempDir,
        snapshot: CompleteDeletionSnapshot,
        ledger: Ledger,
    }

    fn fixture(file_count: usize) -> Fixture {
        let source = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let state = tempdir().unwrap();
        let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
        ledger
            .begin_backup_run("backup-run", "2026-08-09T00:00:00Z", 0)
            .unwrap();
        let mut candidates = Vec::new();
        for index in 0..file_count {
            let relative = PathBuf::from(format!("TX01_MIC{index:03}_20260809_010203.wav"));
            let destination_relative = PathBuf::from("2026/2026-08-09/TX01").join(&relative);
            let bytes = vec![u8::try_from(index).unwrap_or(0x55); 1024 + index];
            fs::write(source.path().join(&relative), &bytes).unwrap();
            fs::create_dir_all(
                destination
                    .path()
                    .join(&destination_relative)
                    .parent()
                    .unwrap(),
            )
            .unwrap();
            fs::write(destination.path().join(&destination_relative), &bytes).unwrap();
            let source_metadata = fs::metadata(source.path().join(&relative)).unwrap();
            let digest = hash_file(&source.path().join(&relative)).unwrap();
            let id = format!("recording-{index}");
            ledger
                .commit_verified_recording(&VerifiedRecording {
                    id: id.clone(),
                    transmitter: Transmitter::Tx01,
                    source_relative_path: relative.clone(),
                    source_size: digest.size,
                    source_mtime_ns: modified_nanos(&source_metadata).unwrap(),
                    source_sha256: digest.sha256.clone(),
                    destination_relative_path: destination_relative.clone(),
                    destination_size: digest.size,
                    destination_sha256: digest.sha256.clone(),
                    verified_at: "2026-08-09T00:01:00Z".to_owned(),
                    backup_run_id: "backup-run".to_owned(),
                })
                .unwrap();
            candidates.push(DeletionCandidate {
                recording_id: id,
                source_relative_path: relative,
                source_size: digest.size,
                source_mtime_ns: modified_nanos(&source_metadata).unwrap(),
                source_sha256: digest.sha256.clone(),
                destination_relative_path: destination_relative,
                destination_size: digest.size,
                destination_sha256: digest.sha256,
            });
        }
        let current_source_paths = candidates
            .iter()
            .map(|candidate| candidate.source_relative_path.clone())
            .collect();
        let snapshot = CompleteDeletionSnapshot {
            context: DeletionContext {
                transmitter: Transmitter::Tx01,
                paired_volume_uuid: "test-volume-uuid".to_owned(),
                mount_generation: 4,
                scan_generation: 7,
                destination_generation: 2,
                source_root: source.path().to_path_buf(),
                destination_root: destination.path().to_path_buf(),
            },
            candidates,
            current_source_paths,
        };
        Fixture {
            source,
            destination,
            _state: state,
            snapshot,
            ledger,
        }
    }

    fn confirm(
        store: &mut DeletionProposalStore,
        proposal_id: &str,
        context: &DeletionContext,
        ledger: &mut Ledger,
        faults: &dyn DeletionFaults,
    ) -> Result<DeletionReport, CoreError> {
        store.confirm(
            proposal_id,
            DeletionConfirmation {
                current_context: context,
                now: Duration::from_secs(2),
                started_at: "2026-08-09T00:02:00Z",
                finished_at: "2026-08-09T00:03:00Z",
            },
            ledger,
            faults,
        )
    }

    #[test]
    fn confirmation_revalidates_the_complete_snapshot_then_deletes_sources_only() {
        let mut fixture = fixture(2);
        let mut store = DeletionProposalStore::default();
        let proposal = store
            .prepare(
                fixture.snapshot.clone(),
                Duration::from_secs(1),
                true,
                &NoDeletionFaults,
            )
            .unwrap();
        assert_eq!(proposal.file_count, 2);
        let report = confirm(
            &mut store,
            &proposal.proposal_id,
            &fixture.snapshot.context,
            &mut fixture.ledger,
            &NoDeletionFaults,
        )
        .unwrap();

        assert_eq!(report.outcome, DeletionOutcome::Deleted);
        assert_eq!(report.deleted_files, 2);
        for candidate in &fixture.snapshot.candidates {
            assert!(
                !fixture
                    .source
                    .path()
                    .join(&candidate.source_relative_path)
                    .exists()
            );
            assert!(
                fixture
                    .destination
                    .path()
                    .join(&candidate.destination_relative_path)
                    .exists()
            );
        }
        assert_eq!(
            fixture
                .ledger
                .deletion_item_outcomes(&report.run_id)
                .unwrap(),
            ["deleted", "deleted"]
        );
    }

    #[test]
    fn changed_destination_refuses_before_the_first_unlink() {
        let mut fixture = fixture(2);
        let mut store = DeletionProposalStore::default();
        let proposal = store
            .prepare(
                fixture.snapshot.clone(),
                Duration::from_secs(1),
                true,
                &NoDeletionFaults,
            )
            .unwrap();
        fs::write(
            fixture
                .destination
                .path()
                .join(&fixture.snapshot.candidates[1].destination_relative_path),
            b"changed",
        )
        .unwrap();
        let result = confirm(
            &mut store,
            &proposal.proposal_id,
            &fixture.snapshot.context,
            &mut fixture.ledger,
            &NoDeletionFaults,
        );
        assert!(matches!(result, Err(CoreError::DeletionPreflightRefused)));
        for candidate in &fixture.snapshot.candidates {
            assert!(
                fixture
                    .source
                    .path()
                    .join(&candidate.source_relative_path)
                    .exists()
            );
        }
    }

    #[test]
    fn every_external_change_invalidates_the_opaque_proposal() {
        for reason in [
            ProposalInvalidation::DeviceDisappeared,
            ProposalInvalidation::MountGenerationChanged,
            ProposalInvalidation::NewScanResults,
            ProposalInvalidation::BackupStarted,
            ProposalInvalidation::DestinationChanged,
            ProposalInvalidation::AnotherDeletionAttempt,
        ] {
            let mut fixture = fixture(1);
            let mut store = DeletionProposalStore::default();
            let proposal = store
                .prepare(
                    fixture.snapshot.clone(),
                    Duration::from_secs(1),
                    true,
                    &NoDeletionFaults,
                )
                .unwrap();
            store.invalidate(reason);
            let result = confirm(
                &mut store,
                &proposal.proposal_id,
                &fixture.snapshot.context,
                &mut fixture.ledger,
                &NoDeletionFaults,
            );
            assert!(matches!(result, Err(CoreError::ProposalInvalidated)));
            assert!(
                fixture
                    .source
                    .path()
                    .join(&fixture.snapshot.candidates[0].source_relative_path)
                    .exists()
            );
        }
    }

    #[test]
    fn expired_replayed_incomplete_and_corrupt_ledger_proposals_are_refused() {
        let mut fixture = fixture(1);
        let mut store = DeletionProposalStore::default();
        let proposal = store
            .prepare(
                fixture.snapshot.clone(),
                Duration::ZERO,
                true,
                &NoDeletionFaults,
            )
            .unwrap();
        let expired = store.confirm(
            &proposal.proposal_id,
            DeletionConfirmation {
                current_context: &fixture.snapshot.context,
                now: PROPOSAL_TTL,
                started_at: "start",
                finished_at: "finish",
            },
            &mut fixture.ledger,
            &NoDeletionFaults,
        );
        assert!(matches!(expired, Err(CoreError::ProposalExpired)));

        let mut incomplete = fixture.snapshot.clone();
        incomplete
            .current_source_paths
            .insert(PathBuf::from("new-unverified-recording.wav"));
        assert!(matches!(
            store.prepare(incomplete, Duration::ZERO, true, &NoDeletionFaults),
            Err(CoreError::DeletionPreflightRefused)
        ));
        assert!(matches!(
            store.prepare(
                fixture.snapshot.clone(),
                Duration::ZERO,
                false,
                &NoDeletionFaults
            ),
            Err(CoreError::DeletionPreflightRefused)
        ));
    }

    struct FailUnlinkAt {
        index: usize,
        seen: Mutex<Vec<DeletionFaultPoint>>,
    }

    impl DeletionFaults for FailUnlinkAt {
        fn check(&self, point: DeletionFaultPoint) -> Result<(), CoreError> {
            self.seen.lock().unwrap().push(point);
            if point == DeletionFaultPoint::Unlink(self.index) {
                return Err(CoreError::CopyFailed(std::io::Error::other(
                    "injected unlink failure",
                )));
            }
            Ok(())
        }
    }

    #[test]
    fn first_unlink_failure_stops_and_records_exact_partial_outcomes() {
        let mut fixture = fixture(3);
        let mut store = DeletionProposalStore::default();
        let proposal = store
            .prepare(
                fixture.snapshot.clone(),
                Duration::from_secs(1),
                true,
                &NoDeletionFaults,
            )
            .unwrap();
        let faults = FailUnlinkAt {
            index: 1,
            seen: Mutex::new(Vec::new()),
        };
        let report = confirm(
            &mut store,
            &proposal.proposal_id,
            &fixture.snapshot.context,
            &mut fixture.ledger,
            &faults,
        )
        .unwrap();
        assert_eq!(report.outcome, DeletionOutcome::PartiallyDeleted);
        assert_eq!(report.deleted_files, 1);
        assert_eq!(report.remaining_files, 2);
        assert_eq!(
            fixture
                .ledger
                .deletion_item_outcomes(&report.run_id)
                .unwrap(),
            ["deleted", "failed", "not_attempted"]
        );
        assert!(
            !fixture
                .source
                .path()
                .join(&fixture.snapshot.candidates[0].source_relative_path)
                .exists()
        );
        assert!(
            fixture
                .source
                .path()
                .join(&fixture.snapshot.candidates[1].source_relative_path)
                .exists()
        );
        assert!(
            fixture
                .source
                .path()
                .join(&fixture.snapshot.candidates[2].source_relative_path)
                .exists()
        );
        assert!(
            !faults
                .seen
                .lock()
                .unwrap()
                .contains(&DeletionFaultPoint::Unlink(2))
        );
    }

    #[test]
    fn confirmation_consumes_the_proposal_and_context_changes_fail_closed() {
        let mut fixture = fixture(1);
        let mut store = DeletionProposalStore::default();
        let proposal = store
            .prepare(
                fixture.snapshot.clone(),
                Duration::from_secs(1),
                true,
                &NoDeletionFaults,
            )
            .unwrap();
        let mut changed = fixture.snapshot.context.clone();
        changed.mount_generation += 1;
        assert!(matches!(
            confirm(
                &mut store,
                &proposal.proposal_id,
                &changed,
                &mut fixture.ledger,
                &NoDeletionFaults
            ),
            Err(CoreError::ProposalInvalidated)
        ));
        assert!(matches!(
            confirm(
                &mut store,
                &proposal.proposal_id,
                &fixture.snapshot.context,
                &mut fixture.ledger,
                &NoDeletionFaults
            ),
            Err(CoreError::ProposalInvalidated)
        ));
    }

    #[derive(Default)]
    struct CountUnlinks(AtomicUsize);

    impl DeletionFaults for CountUnlinks {
        fn check(&self, point: DeletionFaultPoint) -> Result<(), CoreError> {
            if matches!(point, DeletionFaultPoint::Unlink(_)) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    proptest! {
        #[test]
        fn arbitrary_operation_sequences_never_unlink_without_current_authority(
            operations in prop::collection::vec(0_u8..6, 0..24)
        ) {
            let mut fixture = fixture(1);
            let mut store = DeletionProposalStore::default();
            let faults = CountUnlinks::default();
            let mut current_id: Option<String> = None;
            let mut model_authority = false;

            for operation in operations {
                let before = faults.0.load(Ordering::SeqCst);
                let authorized_before = model_authority;
                match operation {
                    0 => {
                        if fixture.source.path().join(&fixture.snapshot.candidates[0].source_relative_path).exists() {
                            let proposal = store.prepare(
                                fixture.snapshot.clone(),
                                Duration::from_secs(1),
                                true,
                                &NoDeletionFaults,
                            ).unwrap();
                            current_id = Some(proposal.proposal_id);
                            model_authority = true;
                        }
                    }
                    1 => {
                        store.invalidate(ProposalInvalidation::NewScanResults);
                        model_authority = false;
                    }
                    2 => {
                        let _ = confirm(
                            &mut store,
                            "unknown-proposal",
                            &fixture.snapshot.context,
                            &mut fixture.ledger,
                            &faults,
                        );
                        model_authority = false;
                    }
                    3 => {
                        if let Some(id) = &current_id {
                            let mut changed = fixture.snapshot.context.clone();
                            changed.destination_generation += 1;
                            let _ = confirm(
                                &mut store,
                                id,
                                &changed,
                                &mut fixture.ledger,
                                &faults,
                            );
                        }
                        model_authority = false;
                    }
                    4 => {
                        if let Some(id) = &current_id {
                            let _ = store.confirm(
                                id,
                                DeletionConfirmation {
                                    current_context: &fixture.snapshot.context,
                                    now: Duration::from_secs(1) + PROPOSAL_TTL,
                                    started_at: "start",
                                    finished_at: "finish",
                                },
                                &mut fixture.ledger,
                                &faults,
                            );
                        }
                        model_authority = false;
                    }
                    _ => {
                        if let Some(id) = &current_id {
                            let result = confirm(
                                &mut store,
                                id,
                                &fixture.snapshot.context,
                                &mut fixture.ledger,
                                &faults,
                            );
                            if model_authority {
                                prop_assert!(result.is_ok());
                            }
                        }
                        model_authority = false;
                    }
                }
                let after = faults.0.load(Ordering::SeqCst);
                if after > before {
                    prop_assert!(operation == 5 && authorized_before);
                }
            }
        }
    }
}
