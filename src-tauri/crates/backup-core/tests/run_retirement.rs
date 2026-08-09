use std::{collections::BTreeSet, fs, path::Path, sync::Mutex, time::Duration};

use backup_core::{
    additional_file::{AdditionalFileClass, VerifiedAdditionalFile},
    artifact::{
        ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact, VerifiedAudioProperties,
    },
    batch::{BatchPhase, FrozenPreferences, M4A_PROFILE_ID},
    deletion::{
        AdditionalDeletionCandidate, CompleteDeletionSnapshot, DeletionCandidate,
        DeletionConfirmation, DeletionContext, DeletionOutcome, DeletionProposalStore,
        NoDeletionFaults, TrashAdapter,
    },
    error::CoreError,
    filesystem::modified_nanos,
    hash::hash_file,
    ledger::{Ledger, VerifiedRecording},
    state::Transmitter,
};
use tempfile::{TempDir, tempdir};

const SESSION: &str = "TX_MIC001_20260810_001116";
const RUN: &str = "verified-run";

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
    let session = source.path().join(SESSION);
    fs::create_dir(&session).unwrap();
    let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
    ledger
        .begin_batch_run(
            RUN,
            "2026-08-10T00:00:00Z",
            0,
            FrozenPreferences {
                automatic_backup: true,
                m4a_conversion: true,
                automatic_trash: false,
            },
        )
        .unwrap();
    ledger
        .advance_batch_phase(RUN, BatchPhase::Copying)
        .unwrap();

    let mut recordings = Vec::new();
    for (index, name) in [
        "TX01_MIC001_20260810_001116.wav",
        "TX01_MIC002_20260810_001117.wav",
    ]
    .into_iter()
    .enumerate()
    {
        let source_relative = Path::new(SESSION).join(name);
        let source_path = source.path().join(&source_relative);
        fs::write(
            &source_path,
            vec![u8::try_from(index + 1).unwrap(); 32 + index],
        )
        .unwrap();
        let source_digest = hash_file(&source_path).unwrap();
        let destination_relative = Path::new("2026/2026-08-10/TX01")
            .join(name)
            .with_extension("m4a");
        let destination_path = destination.path().join(&destination_relative);
        fs::create_dir_all(destination_path.parent().unwrap()).unwrap();
        fs::write(
            &destination_path,
            vec![0xa0 + u8::try_from(index).unwrap(); 12],
        )
        .unwrap();
        let destination_digest = hash_file(&destination_path).unwrap();
        let id = format!("recording-{index}");
        ledger
            .commit_verified_recording(&VerifiedRecording {
                id: id.clone(),
                transmitter: Transmitter::Tx01,
                source_relative_path: source_relative.clone(),
                source_size: source_digest.size,
                source_mtime_ns: modified_nanos(&fs::metadata(&source_path).unwrap()).unwrap(),
                source_sha256: source_digest.sha256.clone(),
                artifact: VerifiedArtifact {
                    relative_path: destination_relative.clone(),
                    format: OutputFormat::M4a,
                    byte_count: destination_digest.size,
                    sha256: destination_digest.sha256.clone(),
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
                verified_at: "2026-08-10T00:01:00Z".to_owned(),
                backup_run_id: RUN.to_owned(),
            })
            .unwrap();
        recordings.push(DeletionCandidate {
            recording_id: id,
            source_relative_path: source_relative,
            source_size: source_digest.size,
            source_mtime_ns: modified_nanos(&fs::metadata(&source_path).unwrap()).unwrap(),
            source_sha256: source_digest.sha256,
            destination_relative_path: destination_relative,
            destination_size: destination_digest.size,
            destination_sha256: destination_digest.sha256,
        });
    }

    let mut additional_files = Vec::new();
    for (index, (name, classification, bytes)) in [
        (
            "TX01_MIC001_20260810_001116.m4a",
            AdditionalFileClass::M4a,
            b"external m4a".as_slice(),
        ),
        (
            "._TX01_MIC001_20260810_001116.m4a",
            AdditionalFileClass::AppleDouble,
            b"apple double".as_slice(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let source_relative = Path::new(SESSION).join(name);
        let source_path = source.path().join(&source_relative);
        fs::write(&source_path, bytes).unwrap();
        let digest = hash_file(&source_path).unwrap();
        let destination_relative =
            Path::new("source-extras/2026/2026-08-10/TX01").join(&source_relative);
        let destination_path = destination.path().join(&destination_relative);
        fs::create_dir_all(destination_path.parent().unwrap()).unwrap();
        fs::write(&destination_path, bytes).unwrap();
        let id = format!("additional-{index}");
        let mtime = modified_nanos(&fs::metadata(&source_path).unwrap()).unwrap();
        ledger
            .commit_verified_additional_file(&VerifiedAdditionalFile {
                id: id.clone(),
                transmitter: Transmitter::Tx01,
                source_relative_path: source_relative.clone(),
                source_size: digest.size,
                source_mtime_ns: mtime,
                source_sha256: digest.sha256.clone(),
                artifact_relative_path: destination_relative.clone(),
                artifact_size: digest.size,
                artifact_sha256: digest.sha256.clone(),
                classification,
                backup_run_id: RUN.to_owned(),
            })
            .unwrap();
        additional_files.push(AdditionalDeletionCandidate {
            additional_file_id: id,
            source_relative_path: source_relative,
            source_size: digest.size,
            source_mtime_ns: mtime,
            source_sha256: digest.sha256.clone(),
            destination_relative_path: destination_relative,
            destination_size: digest.size,
            destination_sha256: digest.sha256,
        });
    }
    ledger
        .advance_batch_phase(RUN, BatchPhase::CopiesVerified)
        .unwrap();
    let recording_ids = recordings
        .iter()
        .map(|candidate| candidate.recording_id.clone())
        .collect::<Vec<_>>();
    ledger
        .begin_conversion_cohort(RUN, &recording_ids, M4A_PROFILE_ID)
        .unwrap();
    for id in &recording_ids {
        ledger.mark_conversion_item_verified(RUN, id).unwrap();
    }
    ledger.commit_m4a_barrier(RUN).unwrap();
    let current_source_paths = recordings
        .iter()
        .map(|candidate| candidate.source_relative_path.clone())
        .chain(
            additional_files
                .iter()
                .map(|candidate| candidate.source_relative_path.clone()),
        )
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
        candidates: recordings,
        additional_files,
        current_source_paths,
        m4a_barrier_run_id: RUN.to_owned(),
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
    moved: Mutex<Vec<std::path::PathBuf>>,
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
        fs::rename(absolute_path, self.root.path().join(SESSION)).map_err(CoreError::CopyFailed)?;
        self.moved.lock().unwrap().push(absolute_path.to_path_buf());
        Ok(())
    }
}

fn prepare_and_confirm(
    fixture: &mut Fixture,
    trash: &FakeTrash,
) -> Result<backup_core::deletion::DeletionReport, CoreError> {
    let mut store = DeletionProposalStore::default();
    let proposal = store.prepare(
        fixture.snapshot.clone(),
        Duration::from_secs(1),
        true,
        &NoDeletionFaults,
    )?;
    store.confirm(
        &proposal.proposal_id,
        DeletionConfirmation {
            current_context: &fixture.snapshot.context,
            now: Duration::from_secs(2),
            started_at: "2026-08-10T00:02:00Z",
            finished_at: "2026-08-10T00:03:00Z",
        },
        &mut fixture.ledger,
        trash,
        &NoDeletionFaults,
    )
}

#[test]
fn verified_wavs_external_m4a_and_sidecar_move_as_one_complete_session() {
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
    assert_eq!(proposal.file_count, 4);
    let report = store
        .confirm(
            &proposal.proposal_id,
            DeletionConfirmation {
                current_context: &fixture.snapshot.context,
                now: Duration::from_secs(2),
                started_at: "2026-08-10T00:02:00Z",
                finished_at: "2026-08-10T00:03:00Z",
            },
            &mut fixture.ledger,
            &trash,
            &NoDeletionFaults,
        )
        .unwrap();

    assert_eq!(report.outcome, DeletionOutcome::Deleted);
    assert_eq!(report.deleted_files, 4);
    assert_eq!(trash.moved.lock().unwrap().len(), 1);
    assert!(!fixture.source.path().join(SESSION).exists());
    assert!(
        fixture
            .destination
            .path()
            .join("2026/2026-08-10/TX01")
            .exists()
    );
    assert!(fixture.destination.path().join("source-extras").exists());
    assert_eq!(
        fixture
            .ledger
            .additional_deletion_outcomes(&report.run_id)
            .unwrap(),
        [
            ("additional-0".to_owned(), "moved_to_trash".to_owned()),
            ("additional-1".to_owned(), "moved_to_trash".to_owned()),
        ]
    );
}

#[test]
fn any_live_backup_or_barrier_mutation_refuses_before_trash() {
    for case in 0..6 {
        let mut fixture = fixture();
        match case {
            0 => fs::write(
                fixture.source.path().join(SESSION).join("unbacked.bin"),
                b"new",
            )
            .unwrap(),
            1 => fs::write(
                fixture
                    .source
                    .path()
                    .join(&fixture.snapshot.additional_files[0].source_relative_path),
                b"changed external",
            )
            .unwrap(),
            2 => {
                let sidecar = fixture
                    .source
                    .path()
                    .join(&fixture.snapshot.additional_files[1].source_relative_path);
                fs::remove_file(&sidecar).unwrap();
                std::os::unix::fs::symlink(
                    fixture
                        .source
                        .path()
                        .join(&fixture.snapshot.candidates[0].source_relative_path),
                    sidecar,
                )
                .unwrap();
            }
            3 => fs::write(
                fixture
                    .destination
                    .path()
                    .join(&fixture.snapshot.candidates[0].destination_relative_path),
                b"changed backup",
            )
            .unwrap(),
            4 => fixture.snapshot.m4a_barrier_run_id = "missing-run".to_owned(),
            5 => {
                let run = "conversion-disabled";
                fixture
                    .ledger
                    .begin_batch_run(
                        run,
                        "2026-08-10T01:00:00Z",
                        0,
                        FrozenPreferences {
                            automatic_backup: true,
                            m4a_conversion: false,
                            automatic_trash: false,
                        },
                    )
                    .unwrap();
                fixture
                    .ledger
                    .advance_batch_phase(run, BatchPhase::Copying)
                    .unwrap();
                fixture
                    .ledger
                    .advance_batch_phase(run, BatchPhase::CopiesVerified)
                    .unwrap();
                let ids = fixture
                    .snapshot
                    .candidates
                    .iter()
                    .map(|candidate| candidate.recording_id.clone())
                    .collect::<Vec<_>>();
                fixture
                    .ledger
                    .begin_conversion_cohort(run, &ids, M4A_PROFILE_ID)
                    .unwrap();
                for id in ids {
                    fixture
                        .ledger
                        .mark_conversion_item_verified(run, &id)
                        .unwrap();
                }
                fixture.ledger.commit_m4a_barrier(run).unwrap();
                fixture.snapshot.m4a_barrier_run_id = run.to_owned();
            }
            _ => unreachable!(),
        }
        let trash = FakeTrash::new();
        assert!(matches!(
            prepare_and_confirm(&mut fixture, &trash),
            Err(CoreError::DeletionPreflightRefused)
        ));
        assert!(trash.moved.lock().unwrap().is_empty());
        assert!(fixture.source.path().join(SESSION).exists());
    }
}
