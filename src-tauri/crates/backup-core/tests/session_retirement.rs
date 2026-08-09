use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use backup_core::{
    artifact::{
        ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact, VerifiedAudioProperties,
    },
    batch::{BatchPhase, FrozenPreferences, M4A_PROFILE_ID},
    deletion::{
        CompleteDeletionSnapshot, DeletionCandidate, DeletionConfirmation, DeletionContext,
        DeletionOutcome, DeletionProposalStore, NoDeletionFaults, TrashAdapter,
        reconcile_legacy_empty_sessions,
    },
    error::CoreError,
    filesystem::modified_nanos,
    hash::hash_file,
    ledger::{Ledger, VerifiedRecording},
    state::Transmitter,
};
use tempfile::{TempDir, tempdir};

struct Fixture {
    source: TempDir,
    destination: TempDir,
    _state: TempDir,
    ledger: Ledger,
    snapshot: CompleteDeletionSnapshot,
}

fn fixture() -> Fixture {
    let source = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let state = tempdir().unwrap();
    let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
    ledger
        .begin_batch_run(
            "backup-run",
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
    let session = Path::new("TX_MIC001_20260809_021747");
    fs::create_dir(source.path().join(session)).unwrap();
    let mut candidates = Vec::new();
    for (index, name) in [
        "TX01_MIC001_20260809_021747.wav",
        "TX01_MIC002_20260809_021748.wav",
    ]
    .iter()
    .enumerate()
    {
        let source_relative = session.join(name);
        let destination_relative = Path::new("2026/2026-08-09/TX01")
            .join(name)
            .with_extension("m4a");
        let bytes = vec![u8::try_from(index + 1).unwrap(); 1024 + index];
        fs::write(source.path().join(&source_relative), &bytes).unwrap();
        fs::create_dir_all(destination.path().join("2026/2026-08-09/TX01")).unwrap();
        fs::write(destination.path().join(&destination_relative), &bytes).unwrap();
        let source_metadata = fs::metadata(source.path().join(&source_relative)).unwrap();
        let digest = hash_file(&source.path().join(&source_relative)).unwrap();
        let recording_id = format!("recording-{index}");
        ledger
            .commit_verified_recording(&VerifiedRecording {
                id: recording_id.clone(),
                transmitter: Transmitter::Tx01,
                source_relative_path: source_relative.clone(),
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
            recording_id,
            source_relative_path: source_relative,
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
        .collect::<BTreeSet<_>>();
    let snapshot = CompleteDeletionSnapshot {
        context: DeletionContext {
            transmitter: Transmitter::Tx01,
            paired_volume_uuid: "fixture-volume".to_owned(),
            mount_generation: 1,
            scan_generation: 1,
            destination_generation: 1,
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
        ledger,
        snapshot,
    }
}

struct FakeTrash {
    root: TempDir,
    moved: Mutex<Vec<PathBuf>>,
}

impl FakeTrash {
    fn new() -> Self {
        Self {
            root: tempdir().unwrap(),
            moved: Mutex::new(Vec::new()),
        }
    }
}

impl TrashAdapter for FakeTrash {
    fn move_to_trash(&self, absolute_path: &Path) -> Result<(), CoreError> {
        let file_name = absolute_path.file_name().ok_or(CoreError::InvalidRequest)?;
        fs::rename(absolute_path, self.root.path().join(file_name))
            .map_err(CoreError::CopyFailed)?;
        self.moved.lock().unwrap().push(absolute_path.to_path_buf());
        Ok(())
    }
}

#[test]
fn a_complete_recognized_session_moves_to_trash_as_one_recoverable_item() {
    let mut fixture = fixture();
    let trash = FakeTrash::new();
    let mut store = DeletionProposalStore::default();
    let proposal = store
        .prepare(
            fixture.snapshot.clone(),
            Duration::from_secs(1),
            true,
            &NoDeletionFaults,
        )
        .unwrap();
    assert_eq!(proposal.session_count, 1);
    assert_eq!(proposal.file_count, 2);
    let report = store
        .confirm(
            &proposal.proposal_id,
            DeletionConfirmation {
                current_context: &fixture.snapshot.context,
                now: Duration::from_secs(2),
                started_at: "2026-08-09T00:02:00Z",
                finished_at: "2026-08-09T00:03:00Z",
            },
            &mut fixture.ledger,
            &trash,
            &NoDeletionFaults,
        )
        .unwrap();

    assert_eq!(report.outcome, DeletionOutcome::Deleted);
    assert_eq!(report.deleted_files, 2);
    assert_eq!(trash.moved.lock().unwrap().len(), 1);
    assert!(
        !fixture
            .source
            .path()
            .join("TX_MIC001_20260809_021747")
            .exists()
    );
    assert!(trash.root.path().join("TX_MIC001_20260809_021747").exists());
    assert!(
        fixture
            .destination
            .path()
            .join("2026/2026-08-09/TX01")
            .exists()
    );
}

#[test]
fn a_new_root_recording_after_proposal_refuses_before_the_first_trash_move() {
    let mut fixture = fixture();
    let trash = FakeTrash::new();
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
            .source
            .path()
            .join("TX01_MIC099_20260809_022000.wav"),
        b"new recording",
    )
    .unwrap();

    assert!(matches!(
        store.confirm(
            &proposal.proposal_id,
            DeletionConfirmation {
                current_context: &fixture.snapshot.context,
                now: Duration::from_secs(2),
                started_at: "2026-08-09T00:02:00Z",
                finished_at: "2026-08-09T00:03:00Z",
            },
            &mut fixture.ledger,
            &trash,
            &NoDeletionFaults,
        ),
        Err(CoreError::DeletionPreflightRefused)
    ));
    assert!(
        fixture
            .source
            .path()
            .join("TX_MIC001_20260809_021747")
            .exists()
    );
    assert!(trash.moved.lock().unwrap().is_empty());
}

#[test]
fn an_unknown_hidden_entry_refuses_the_whole_session_before_any_move() {
    let fixture = fixture();
    fs::write(
        fixture
            .source
            .path()
            .join("TX_MIC001_20260809_021747/.unknown"),
        b"do not move",
    )
    .unwrap();
    let mut store = DeletionProposalStore::default();

    assert!(matches!(
        store.prepare(
            fixture.snapshot,
            Duration::from_secs(1),
            true,
            &NoDeletionFaults,
        ),
        Err(CoreError::DeletionPreflightRefused)
    ));
}

#[test]
fn session_inventory_refuses_unknown_nested_symlink_missing_and_unanchored_entries() {
    for case in 0..5 {
        let mut fixture = fixture();
        let session = fixture.source.path().join("TX_MIC001_20260809_021747");
        match case {
            0 => fs::write(session.join("unknown.txt"), b"unknown").unwrap(),
            1 => fs::create_dir(session.join("nested")).unwrap(),
            2 => std::os::unix::fs::symlink(
                fixture
                    .source
                    .path()
                    .join(&fixture.snapshot.candidates[0].source_relative_path),
                session.join("linked.wav"),
            )
            .unwrap(),
            3 => fs::remove_file(
                fixture
                    .source
                    .path()
                    .join(&fixture.snapshot.candidates[0].source_relative_path),
            )
            .unwrap(),
            4 => {
                let invalid = fixture
                    .source
                    .path()
                    .join("TX_MIC001_20260809_021747-extra");
                fs::rename(&session, &invalid).unwrap();
                for candidate in &mut fixture.snapshot.candidates {
                    candidate.source_relative_path = Path::new("TX_MIC001_20260809_021747-extra")
                        .join(candidate.source_relative_path.file_name().unwrap());
                }
                fixture.snapshot.current_source_paths = fixture
                    .snapshot
                    .candidates
                    .iter()
                    .map(|candidate| candidate.source_relative_path.clone())
                    .collect();
            }
            _ => unreachable!(),
        }
        let mut store = DeletionProposalStore::default();
        assert!(matches!(
            store.prepare(
                fixture.snapshot,
                Duration::from_secs(1),
                true,
                &NoDeletionFaults,
            ),
            Err(CoreError::DeletionPreflightRefused)
        ));
    }
}

#[test]
fn an_empty_legacy_session_moves_only_with_exact_ledger_evidence() {
    let mut fixture = fixture();
    let session = Path::new("TX_MIC009_20260809_235959");
    fs::create_dir(fixture.source.path().join(session)).unwrap();
    fixture
        .ledger
        .commit_verified_recording(&VerifiedRecording {
            id: "legacy-recording".to_owned(),
            transmitter: Transmitter::Tx01,
            source_relative_path: session.join("TX01_MIC009_20260809_235959.wav"),
            source_size: 4,
            source_mtime_ns: 1,
            source_sha256: "0".repeat(64),
            artifact: VerifiedArtifact {
                relative_path: PathBuf::from("legacy.wav"),
                format: OutputFormat::Wav,
                byte_count: 4,
                sha256: "0".repeat(64),
                audio: None,
            },
            conversion_status: ConversionStatus::NotRequired,
            conversion_error_code: None,
            retirement_status: RetirementStatus::LegacyDeleted,
            retired_session_relative_path: None,
            verified_at: "2026-08-09T00:01:00Z".to_owned(),
            backup_run_id: "backup-run".to_owned(),
        })
        .unwrap();
    let trash = FakeTrash::new();

    assert_eq!(
        reconcile_legacy_empty_sessions(
            &fixture.snapshot.context,
            &mut fixture.ledger,
            &trash,
            "2026-08-09T00:04:00Z",
        )
        .unwrap(),
        1
    );
    assert!(!fixture.source.path().join(session).exists());
    assert!(trash.root.path().join(session).exists());
    assert_eq!(
        fixture
            .ledger
            .verified_recording("legacy-recording")
            .unwrap()
            .unwrap()
            .retirement_status,
        RetirementStatus::MovedToTrash
    );
}
