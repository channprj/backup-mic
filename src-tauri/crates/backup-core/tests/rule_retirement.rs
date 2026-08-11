use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use backup_core::{
    artifact::{
        ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact, VerifiedAudioProperties,
    },
    batch::{BatchPhase, FrozenPreferences, M4A_PROFILE_ID},
    deletion::{
        CompleteRuleDeletionSnapshot, DeletionCandidate, DeletionOutcome, DeletionProposalStore,
        NoDeletionFaults, RuleDeletionConfirmation, RuleDeletionContext,
        RuleSessionDeletionCandidate, TrashAdapter,
    },
    device::VolumeDescriptor,
    error::CoreError,
    filesystem::modified_nanos,
    hash::hash_file,
    ledger::{Ledger, VerifiedRecording},
    rule::{BackupRule, BackupRuleDraft, RuleId},
    source::{MountedSourceAuthority, SourceId, SourceRecord},
};
use tempfile::{TempDir, tempdir};
use time::{Duration, OffsetDateTime};

const STARTED_AT: &str = "2026-08-10T00:00:00Z";
const FINISHED_AT: &str = "2026-08-10T00:01:00Z";

struct Fixture {
    source: TempDir,
    destination: TempDir,
    state: TempDir,
    ledger: Ledger,
    rule: BackupRule,
    authority: MountedSourceAuthority,
    snapshot: CompleteRuleDeletionSnapshot,
}

impl Fixture {
    fn context(
        &self,
        destination_generation: u64,
        scan_generation: u64,
    ) -> RuleDeletionContext<'_> {
        RuleDeletionContext {
            source_id: &self.authority.source.id,
            rule_id: &self.rule.id,
            rule_updated_at: &self.rule.updated_at,
            authority: &self.authority,
            destination_generation,
            scan_generation,
        }
    }

    fn prepare(&mut self, now: OffsetDateTime) -> (DeletionProposalStore, String) {
        let mut store = DeletionProposalStore::default();
        let proposal = store
            .prepare_rule(
                self.context(1, 1),
                &self.rule,
                self.snapshot.clone(),
                now,
                true,
                &self.ledger,
                &NoDeletionFaults,
            )
            .unwrap();
        (store, proposal.proposal_id)
    }

    fn confirm(
        &mut self,
        store: &mut DeletionProposalStore,
        proposal_id: &str,
        current: (
            &BackupRule,
            &MountedSourceAuthority,
            u64,
            u64,
            OffsetDateTime,
        ),
        trash: &FakeTrash,
    ) -> Result<backup_core::deletion::DeletionReport, CoreError> {
        let (current_rule, authority, destination_generation, scan_generation, now) = current;
        let context = RuleDeletionContext {
            source_id: &authority.source.id,
            rule_id: &current_rule.id,
            rule_updated_at: &current_rule.updated_at,
            authority,
            destination_generation,
            scan_generation,
        };
        store.confirm_rule(
            proposal_id,
            RuleDeletionConfirmation {
                current_context: context,
                current_rule,
                now,
                started_at: STARTED_AT,
                finished_at: FINISHED_AT,
            },
            &mut self.ledger,
            trash,
            &NoDeletionFaults,
        )
    }
}

struct FakeTrash {
    root: TempDir,
    calls: AtomicUsize,
    moved: Mutex<Vec<PathBuf>>,
}

impl FakeTrash {
    fn new() -> Self {
        Self {
            root: tempdir().unwrap(),
            calls: AtomicUsize::new(0),
            moved: Mutex::new(Vec::new()),
        }
    }
}

impl TrashAdapter for FakeTrash {
    fn move_to_trash(&self, absolute_path: &Path) -> Result<(), CoreError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let name = absolute_path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or(CoreError::InvalidRequest)?;
        let target = self
            .root
            .path()
            .join(format!("{}-{name}", self.calls.load(Ordering::SeqCst)));
        fs::rename(absolute_path, target).map_err(CoreError::CopyFailed)?;
        self.moved.lock().unwrap().push(absolute_path.to_path_buf());
        Ok(())
    }
}

#[test]
fn generic_files_move_individually_but_a_complete_rule_session_moves_once() {
    let now = OffsetDateTime::from_unix_timestamp(1_786_320_000).unwrap();

    let mut file_fixture = fixture(false);
    let file_trash = FakeTrash::new();
    let (mut file_store, proposal_id) = file_fixture.prepare(now);
    let rule = file_fixture.rule.clone();
    let authority = file_fixture.authority.clone();
    let report = file_fixture
        .confirm(
            &mut file_store,
            &proposal_id,
            (&rule, &authority, 1, 1, now + Duration::seconds(1)),
            &file_trash,
        )
        .unwrap();
    assert_eq!(report.outcome, DeletionOutcome::Deleted);
    assert_eq!(report.deleted_files, 2);
    assert_eq!(file_trash.calls.load(Ordering::SeqCst), 2);

    let mut session_fixture = fixture(true);
    let session_trash = FakeTrash::new();
    let (mut session_store, proposal_id) = session_fixture.prepare(now);
    let rule = session_fixture.rule.clone();
    let authority = session_fixture.authority.clone();
    let report = session_fixture
        .confirm(
            &mut session_store,
            &proposal_id,
            (&rule, &authority, 1, 1, now + Duration::seconds(1)),
            &session_trash,
        )
        .unwrap();
    assert_eq!(report.outcome, DeletionOutcome::Deleted);
    assert_eq!(report.deleted_files, 2);
    assert_eq!(session_trash.calls.load(Ordering::SeqCst), 1);
    assert!(
        !session_fixture
            .source
            .path()
            .join("RECORD/FOLDER01")
            .exists()
    );
}

#[test]
fn unsafe_or_incomplete_session_entries_preserve_the_whole_session() {
    enum UnsafeEntry {
        UnselectedFile,
        NestedDirectory,
        Symlink,
    }

    for unsafe_entry in [
        UnsafeEntry::UnselectedFile,
        UnsafeEntry::NestedDirectory,
        UnsafeEntry::Symlink,
    ] {
        let fixture = fixture(true);
        let session = fixture.source.path().join("RECORD/FOLDER01");
        match unsafe_entry {
            UnsafeEntry::UnselectedFile => fs::write(session.join("notes.tmp"), b"notes").unwrap(),
            UnsafeEntry::NestedDirectory => fs::create_dir(session.join("nested")).unwrap(),
            UnsafeEntry::Symlink => {
                #[cfg(unix)]
                std::os::unix::fs::symlink(session.join("one.wav"), session.join("link.wav"))
                    .unwrap();
                #[cfg(windows)]
                std::os::windows::fs::symlink_file(
                    session.join("one.wav"),
                    session.join("link.wav"),
                )
                .unwrap();
            }
        }
        let trash = FakeTrash::new();
        let mut store = DeletionProposalStore::default();
        let error = store
            .prepare_rule(
                fixture.context(1, 1),
                &fixture.rule,
                fixture.snapshot.clone(),
                OffsetDateTime::UNIX_EPOCH,
                true,
                &fixture.ledger,
                &NoDeletionFaults,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            CoreError::SourceChanged | CoreError::UnsafeSessionEntry
        ));
        assert!(session.exists());
        assert_eq!(trash.calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn confirmation_invalidates_every_frozen_rule_and_mount_authority_field_before_trash() {
    enum Mutation {
        RuleRevision,
        RuleDisabled,
        RuleId,
        VolumeUuid,
        MountGeneration,
        DestinationGeneration,
        ScanGeneration,
    }
    let now = OffsetDateTime::from_unix_timestamp(1_786_320_000).unwrap();

    for mutation in [
        Mutation::RuleRevision,
        Mutation::RuleDisabled,
        Mutation::RuleId,
        Mutation::VolumeUuid,
        Mutation::MountGeneration,
        Mutation::DestinationGeneration,
        Mutation::ScanGeneration,
    ] {
        let mut fixture = fixture(false);
        let (mut store, proposal_id) = fixture.prepare(now);
        let mut rule = fixture.rule.clone();
        let mut authority = fixture.authority.clone();
        let mut destination_generation = 1;
        let mut scan_generation = 1;
        match mutation {
            Mutation::RuleRevision => rule.updated_at = "2026-08-10T01:00:00Z".to_owned(),
            Mutation::RuleDisabled => rule.enabled = false,
            Mutation::RuleId => rule.id = RuleId::new(),
            Mutation::VolumeUuid => authority.descriptor.volume_uuid = "replacement".to_owned(),
            Mutation::MountGeneration => authority.descriptor.mount_generation += 1,
            Mutation::DestinationGeneration => destination_generation += 1,
            Mutation::ScanGeneration => scan_generation += 1,
        }
        let trash = FakeTrash::new();
        let error = fixture
            .confirm(
                &mut store,
                &proposal_id,
                (
                    &rule,
                    &authority,
                    destination_generation,
                    scan_generation,
                    now + Duration::seconds(1),
                ),
                &trash,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            CoreError::ProposalInvalidated | CoreError::DeletionPreflightRefused
        ));
        assert_eq!(trash.calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn source_destination_barrier_and_expiry_are_rechecked_before_trash() {
    enum Mutation {
        SourceBytes,
        DestinationBytes,
        BatchBarrier,
        Expired,
    }
    let now = OffsetDateTime::from_unix_timestamp(1_786_320_000).unwrap();

    for mutation in [
        Mutation::SourceBytes,
        Mutation::DestinationBytes,
        Mutation::BatchBarrier,
        Mutation::Expired,
    ] {
        let mut fixture = fixture(false);
        let (mut store, proposal_id) = fixture.prepare(now);
        match mutation {
            Mutation::SourceBytes => {
                fs::write(fixture.source.path().join("one.wav"), b"changed").unwrap()
            }
            Mutation::DestinationBytes => {
                fs::write(fixture.destination.path().join("one.m4a"), b"changed").unwrap()
            }
            Mutation::BatchBarrier => {
                let connection =
                    rusqlite::Connection::open(fixture.state.path().join("ledger.sqlite3"))
                        .unwrap();
                connection
                    .execute(
                        "UPDATE backup_runs SET batch_phase = 'copies_verified' WHERE id = 'backup-run'",
                        [],
                    )
                    .unwrap();
            }
            Mutation::Expired => {}
        }
        let rule = fixture.rule.clone();
        let authority = fixture.authority.clone();
        let trash = FakeTrash::new();
        let confirmation_time = if matches!(mutation, Mutation::Expired) {
            now + Duration::minutes(5)
        } else {
            now + Duration::seconds(1)
        };
        let error = fixture
            .confirm(
                &mut store,
                &proposal_id,
                (&rule, &authority, 1, 1, confirmation_time),
                &trash,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            CoreError::SourceChanged
                | CoreError::DeletionPreflightRefused
                | CoreError::ProposalExpired
        ));
        assert_eq!(trash.calls.load(Ordering::SeqCst), 0);
    }
}

fn fixture(session: bool) -> Fixture {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let state = tempdir().unwrap();
    let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
    let base = if session { "RECORD/FOLDER01" } else { "" };
    let rule = ledger
        .save_backup_rule(
            BackupRuleDraft {
                id: None,
                name: if session {
                    "Session recorder"
                } else {
                    "File recorder"
                }
                .to_owned(),
                archive_directory_name: if session { "Sessions" } else { "Files" }.to_owned(),
                enabled: true,
                volume_name_glob: "RULE-TEST".to_owned(),
                required_path_globs: Vec::new(),
                backup_file_globs: vec![if session {
                    "RECORD/FOLDER*/*.wav".to_owned()
                } else {
                    "*.wav".to_owned()
                }],
                session_directory_globs: if session {
                    vec!["RECORD/FOLDER*".to_owned()]
                } else {
                    Vec::new()
                },
                filename_prefix: String::new(),
                filename_suffix: String::new(),
                date_folder_layout: Default::default(),
            },
            "2026-08-10T00:00:00Z",
        )
        .unwrap();
    let source_record = SourceRecord {
        id: SourceId::new(),
        rule_id: rule.id.clone(),
        volume_uuid: "rule-volume".to_owned(),
        legacy_slot: None,
        display_name: "RULE-TEST".to_owned(),
    };
    ledger.upsert_source(&source_record, STARTED_AT).unwrap();
    let authority = MountedSourceAuthority {
        source: source_record.clone(),
        descriptor: VolumeDescriptor {
            volume_uuid: source_record.volume_uuid.clone(),
            mount_root: source.path().to_path_buf(),
            protocol: "USB".to_owned(),
            is_internal: false,
            is_removable: true,
            is_writable: true,
            media_name: "Recorder".to_owned(),
            nominal_capacity: 64_000_000,
            mount_generation: 9,
        },
    };
    ledger
        .begin_batch_run(
            "backup-run",
            &source_record.id,
            STARTED_AT,
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
    for name in ["one.wav", "two.wav"] {
        let relative = if base.is_empty() {
            PathBuf::from(name)
        } else {
            PathBuf::from(base).join(name)
        };
        let source_path = source.path().join(&relative);
        fs::create_dir_all(source_path.parent().unwrap()).unwrap();
        fs::write(&source_path, format!("source-{name}")).unwrap();
        let destination_relative = if base.is_empty() {
            PathBuf::from(name).with_extension("m4a")
        } else {
            PathBuf::from("archive").join(name).with_extension("m4a")
        };
        let destination_path = destination.path().join(&destination_relative);
        fs::create_dir_all(destination_path.parent().unwrap()).unwrap();
        fs::write(&destination_path, format!("artifact-{name}")).unwrap();
        let source_hash = hash_file(&source_path).unwrap();
        let destination_hash = hash_file(&destination_path).unwrap();
        let source_mtime_ns = modified_nanos(&fs::metadata(&source_path).unwrap()).unwrap();
        let recording_id = format!("recording-{name}");
        ledger
            .commit_verified_recording(&VerifiedRecording {
                id: recording_id.clone(),
                source_id: source_record.id.clone(),
                source_relative_path: relative.clone(),
                source_size: source_hash.size,
                source_mtime_ns,
                source_sha256: source_hash.sha256.clone(),
                artifact: VerifiedArtifact {
                    relative_path: destination_relative.clone(),
                    format: OutputFormat::M4a,
                    byte_count: destination_hash.size,
                    sha256: destination_hash.sha256.clone(),
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
                verified_at: STARTED_AT.to_owned(),
                backup_run_id: "backup-run".to_owned(),
            })
            .unwrap();
        candidates.push(DeletionCandidate {
            recording_id,
            source_relative_path: relative,
            source_size: source_hash.size,
            source_mtime_ns,
            source_sha256: source_hash.sha256,
            destination_relative_path: destination_relative,
            destination_size: destination_hash.size,
            destination_sha256: destination_hash.sha256,
        });
    }
    ledger
        .advance_batch_phase("backup-run", BatchPhase::CopiesVerified)
        .unwrap();
    let ids = candidates
        .iter()
        .map(|candidate| candidate.recording_id.clone())
        .collect::<Vec<_>>();
    ledger
        .begin_conversion_cohort("backup-run", &ids, M4A_PROFILE_ID)
        .unwrap();
    for id in &ids {
        ledger
            .mark_conversion_item_verified("backup-run", id)
            .unwrap();
    }
    ledger.commit_m4a_barrier("backup-run").unwrap();
    let snapshot = if session {
        CompleteRuleDeletionSnapshot {
            files: Vec::new(),
            additional_files: Vec::new(),
            sessions: vec![RuleSessionDeletionCandidate {
                relative_directory: PathBuf::from(base),
                files: candidates,
                additional_files: Vec::new(),
            }],
            destination_root: destination.path().to_path_buf(),
            m4a_barrier_run_id: "backup-run".to_owned(),
        }
    } else {
        CompleteRuleDeletionSnapshot {
            files: candidates,
            additional_files: Vec::new(),
            sessions: Vec::new(),
            destination_root: destination.path().to_path_buf(),
            m4a_barrier_run_id: "backup-run".to_owned(),
        }
    };
    Fixture {
        source,
        destination,
        state,
        ledger,
        rule,
        authority,
        snapshot,
    }
}
