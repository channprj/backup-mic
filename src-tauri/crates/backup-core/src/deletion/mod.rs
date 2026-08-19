//! Source retirement: never permanent, never without a second live verification.
//!
//! The shape of this module is the safety argument. `proposal` is the only place that can grant
//! authority to touch a source, the `*_verify` modules re-prove the stored evidence against what
//! is on disk right now, and only `rule_execute` and the Trash adapter ever move anything. A file
//! is moved to the macOS Trash, so a mistake stays recoverable.

mod evidence;
mod legacy;
mod proposal;
mod rule_execute;
mod rule_verify;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use time::OffsetDateTime;

use crate::error::CoreError;
use crate::rule::{BackupRule, RuleId};
use crate::source::{MountedSourceAuthority, SourceId};
use crate::state::Transmitter;

pub use legacy::reconcile_legacy_empty_sessions;
pub use proposal::propose_rule_deletion;

pub const PROPOSAL_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy)]
pub struct RuleDeletionContext<'a> {
    pub source_id: &'a SourceId,
    pub rule_id: &'a RuleId,
    pub rule_updated_at: &'a str,
    pub authority: &'a MountedSourceAuthority,
    pub destination_generation: u64,
    pub scan_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSessionDeletionCandidate {
    pub relative_directory: PathBuf,
    pub files: Vec<DeletionCandidate>,
    pub additional_files: Vec<AdditionalDeletionCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteRuleDeletionSnapshot {
    pub files: Vec<DeletionCandidate>,
    pub additional_files: Vec<AdditionalDeletionCandidate>,
    pub sessions: Vec<RuleSessionDeletionCandidate>,
    pub destination_root: PathBuf,
    pub m4a_barrier_run_id: String,
}

#[derive(Debug, Clone)]
pub struct DeletionProposal {
    pub proposal_id: String,
    pub source_id: SourceId,
    pub session_count: u64,
    pub file_count: u64,
    pub byte_count: u64,
    pub expires_at: OffsetDateTime,
    frozen: FrozenRuleDeletionContext,
    snapshot: CompleteRuleDeletionSnapshot,
}

pub struct RuleDeletionConfirmation<'a> {
    pub current_context: RuleDeletionContext<'a>,
    pub current_rule: &'a BackupRule,
    pub now: OffsetDateTime,
    pub started_at: &'a str,
    pub finished_at: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FrozenRuleDeletionContext {
    pub(super) source_id: SourceId,
    pub(super) rule_id: RuleId,
    pub(super) rule_updated_at: String,
    pub(super) authority: MountedSourceAuthority,
    pub(super) destination_generation: u64,
    pub(super) scan_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionContext {
    pub source_id: SourceId,
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
pub struct AdditionalDeletionCandidate {
    pub additional_file_id: String,
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
    pub additional_files: Vec<AdditionalDeletionCandidate>,
    pub current_source_paths: BTreeSet<PathBuf>,
    pub m4a_barrier_run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionProposalSummary {
    pub proposal_id: String,
    pub transmitter: Transmitter,
    pub session_count: u64,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetirementTarget {
    Session {
        relative_directory: PathBuf,
        recording_ids: Vec<String>,
        additional_file_ids: Vec<String>,
    },
    RootFile {
        relative_path: PathBuf,
        recording_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetirementPlan {
    pub transmitter: Transmitter,
    pub targets: Vec<RetirementTarget>,
    pub recording_count: usize,
    pub additional_file_count: usize,
    pub byte_count: u64,
}

pub trait TrashAdapter: Send + Sync {
    fn move_to_trash(&self, absolute_path: &Path) -> Result<(), CoreError>;
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
    MoveToTrash(usize),
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
pub(super) struct PrivateProposal {
    pub(super) context: DeletionContext,
    pub(super) candidates: Vec<DeletionCandidate>,
    pub(super) additional_files: Vec<AdditionalDeletionCandidate>,
    pub(super) m4a_barrier_run_id: String,
    pub(super) expires_at: Duration,
}

#[derive(Debug, Default)]
pub struct DeletionProposalStore {
    proposals: HashMap<String, PrivateProposal>,
    rule_proposals: HashMap<String, DeletionProposal>,
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

    use crate::{
        artifact::{
            ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact,
            VerifiedAudioProperties,
        },
        batch::{BatchPhase, FrozenPreferences, M4A_PROFILE_ID},
        filesystem::modified_nanos,
        hash::hash_file,
        ledger::{Ledger, VerifiedRecording},
        source::{SourceId, SourceRecord},
    };
    use uuid::Uuid;

    use super::*;

    struct TestTrash;

    impl TrashAdapter for TestTrash {
        fn move_to_trash(&self, absolute_path: &Path) -> Result<(), CoreError> {
            let file_name = absolute_path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or(CoreError::InvalidRequest)?;
            let target = absolute_path
                .parent()
                .ok_or(CoreError::InvalidRequest)?
                .join(format!(".trashed-{file_name}-{}", Uuid::new_v4()));
            fs::rename(absolute_path, target).map_err(CoreError::CopyFailed)
        }
    }

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
        let source_record = SourceRecord {
            id: SourceId::new(),
            rule_id: ledger.dji_rule().unwrap().id,
            volume_uuid: "deletion-unit-tx01".to_owned(),
            legacy_slot: Some("TX01".to_owned()),
            display_name: "Deletion Unit TX01".to_owned(),
        };
        ledger
            .upsert_source(&source_record, "2026-08-09T00:00:00Z")
            .unwrap();
        ledger
            .begin_batch_run(
                "backup-run",
                &source_record.id,
                "2026-08-09T00:00:00Z",
                0,
                FrozenPreferences {
                    automatic_backup: true,
                    m4a_conversion: true,
                    automatic_trash: false,
                },
            )
            .unwrap();
        ledger
            .advance_batch_phase("backup-run", BatchPhase::Copying)
            .unwrap();
        let mut candidates = Vec::new();
        for index in 0..file_count {
            let relative = PathBuf::from(format!("TX01_MIC{index:03}_20260809_010203.wav"));
            let destination_relative = PathBuf::from("2026/2026-08-09/TX01")
                .join(&relative)
                .with_extension("m4a");
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
                    source_id: source_record.id.clone(),
                    source_relative_path: relative.clone(),
                    source_size: digest.size,
                    source_mtime_ns: modified_nanos(&source_metadata).unwrap(),
                    source_sha256: digest.sha256.clone(),
                    artifact: VerifiedArtifact {
                        relative_path: destination_relative.clone(),
                        format: OutputFormat::M4a,
                        byte_count: digest.size,
                        sha256: digest.sha256.clone(),
                        audio: Some(VerifiedAudioProperties {
                            codec: "aac".to_owned(),
                            sample_rate_hz: 48_000,
                            channel_count: 1,
                            valid_frames: 48_000,
                            duration_micros: 1_000_000,
                        }),
                    },
                    conversion_status: ConversionStatus::Complete,
                    conversion_error_code: None,
                    retirement_status: RetirementStatus::Present,
                    retired_session_relative_path: None,
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
        ledger
            .advance_batch_phase("backup-run", BatchPhase::CopiesVerified)
            .unwrap();
        let recording_ids = candidates
            .iter()
            .map(|candidate| candidate.recording_id.clone())
            .collect::<Vec<_>>();
        ledger
            .begin_conversion_cohort("backup-run", &recording_ids, M4A_PROFILE_ID)
            .unwrap();
        for recording_id in &recording_ids {
            ledger
                .mark_conversion_item_verified("backup-run", recording_id)
                .unwrap();
        }
        ledger.commit_m4a_barrier("backup-run").unwrap();
        let current_source_paths = candidates
            .iter()
            .map(|candidate| candidate.source_relative_path.clone())
            .collect();
        let snapshot = CompleteDeletionSnapshot {
            context: DeletionContext {
                source_id: source_record.id,
                transmitter: Transmitter::Tx01,
                paired_volume_uuid: "test-volume-uuid".to_owned(),
                mount_generation: 4,
                scan_generation: 7,
                destination_generation: 2,
                source_root: source.path().to_path_buf(),
                destination_root: destination.path().to_path_buf(),
            },
            candidates,
            additional_files: Vec::new(),
            current_source_paths,
            m4a_barrier_run_id: "backup-run".to_owned(),
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
            &TestTrash,
            faults,
        )
    }

    #[test]
    fn confirmation_revalidates_the_complete_snapshot_then_moves_sources_only() {
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
        assert_eq!(proposal.session_count, 0);
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
            ["moved_to_trash", "moved_to_trash"]
        );
    }

    #[test]
    fn changed_destination_refuses_before_the_first_trash_move() {
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
            &TestTrash,
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

    struct FailTrashAt {
        index: usize,
        seen: Mutex<Vec<DeletionFaultPoint>>,
    }

    impl DeletionFaults for FailTrashAt {
        fn check(&self, point: DeletionFaultPoint) -> Result<(), CoreError> {
            self.seen.lock().unwrap().push(point);
            if point == DeletionFaultPoint::MoveToTrash(self.index) {
                return Err(CoreError::CopyFailed(std::io::Error::other(
                    "injected Trash failure",
                )));
            }
            Ok(())
        }
    }

    #[test]
    fn first_trash_failure_stops_and_records_exact_partial_outcomes() {
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
        let faults = FailTrashAt {
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
            ["moved_to_trash", "failed", "not_attempted"]
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
                .contains(&DeletionFaultPoint::MoveToTrash(2))
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
    struct CountTrashMoves(AtomicUsize);

    impl DeletionFaults for CountTrashMoves {
        fn check(&self, point: DeletionFaultPoint) -> Result<(), CoreError> {
            if matches!(point, DeletionFaultPoint::MoveToTrash(_)) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    proptest! {
        #[test]
        fn arbitrary_operation_sequences_never_move_without_current_authority(
            operations in prop::collection::vec(0_u8..6, 0..24)
        ) {
            let mut fixture = fixture(1);
            let mut store = DeletionProposalStore::default();
            let faults = CountTrashMoves::default();
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
                                &TestTrash,
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
