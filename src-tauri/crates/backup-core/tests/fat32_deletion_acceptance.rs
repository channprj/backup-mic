use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

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
    scanner::scan_once,
    state::Transmitter,
};
use tempfile::tempdir;
use time::UtcOffset;

const FIXTURE_ROOT: &str = "/Volumes/DJI-DELTEST";
const MARKER: &str = ".dji-mic-backup-delete-fixture";
const RUN_ID: &str = "fat32-acceptance-backup";

struct FixtureTrash {
    root: PathBuf,
}

impl TrashAdapter for FixtureTrash {
    fn move_to_trash(&self, absolute_path: &Path) -> Result<(), CoreError> {
        fs::create_dir_all(&self.root).map_err(CoreError::CopyFailed)?;
        let name = absolute_path.file_name().ok_or(CoreError::InvalidRequest)?;
        fs::rename(absolute_path, self.root.join(name)).map_err(CoreError::CopyFailed)
    }
}

#[test]
#[ignore = "requires scripts/accept-deletion-fixture.sh"]
fn moves_a_whole_session_to_recoverable_trash_on_an_isolated_fat32_volume() {
    let requested = std::env::var_os("DJI_MIC_DELETION_FIXTURE")
        .map(PathBuf::from)
        .expect("fixture path is required");
    assert_eq!(requested, PathBuf::from(FIXTURE_ROOT));
    let fixture = fs::canonicalize(&requested).expect("fixture must be mounted");
    assert_eq!(fixture, PathBuf::from(FIXTURE_ROOT));
    assert_eq!(
        fs::read_to_string(fixture.join(MARKER)).expect("fixture marker is required"),
        "isolated-fat32-trash-test\n"
    );

    let source = fixture.join("source");
    let destination = fixture.join("destination");
    assert!(!source.exists() && !destination.exists());
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();

    let session_relative = PathBuf::from("TX_MIC001_20260809_010203");
    let session = source.join(&session_relative);
    fs::create_dir(&session).unwrap();
    let source_files = [
        (
            session_relative.join("TX01_MIC001_20260809_010203.wav"),
            deterministic_bytes(0x31, 12 * 1024),
        ),
        (
            session_relative.join("TX01_MIC001_20260809_010204.wav"),
            deterministic_bytes(0x53, 16 * 1024),
        ),
        (
            session_relative.join("recorder-preview.m4a"),
            b"external recorder m4a fixture".to_vec(),
        ),
        (
            session_relative.join("._TX01_MIC001_20260809_010203.wav"),
            b"appledouble metadata fixture".to_vec(),
        ),
    ];
    for (relative, contents) in &source_files {
        fs::write(source.join(relative), contents).unwrap();
    }
    let initial_scan = scan_once(&source, Transmitter::Tx01, UtcOffset::UTC).unwrap();
    assert_eq!(initial_scan.recordings.len(), 2);
    assert!(initial_scan.additional_files.len() >= 2);
    assert!(initial_scan.issues.is_empty());
    assert!(initial_scan.additional_files.iter().any(|file| {
        file.relative_path == session_relative.join("recorder-preview.m4a")
            && file.classification == AdditionalFileClass::M4a
    }));
    assert!(initial_scan.additional_files.iter().any(|file| {
        file.relative_path == session_relative.join("._TX01_MIC001_20260809_010203.wav")
            && file.classification == AdditionalFileClass::AppleDouble
    }));
    let expected_session_files = initial_scan
        .recordings
        .iter()
        .map(|recording| recording.relative_path.clone())
        .chain(
            initial_scan
                .additional_files
                .iter()
                .map(|file| file.relative_path.clone()),
        )
        .map(|relative| {
            let contents = fs::read(source.join(&relative)).unwrap();
            (relative, contents)
        })
        .collect::<Vec<_>>();

    let state = tempdir().unwrap();
    let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
    let required_bytes = expected_session_files
        .iter()
        .map(|(_, contents)| u64::try_from(contents.len()).unwrap())
        .sum();
    ledger
        .begin_batch_run(
            RUN_ID,
            "2026-08-09T00:00:00Z",
            required_bytes,
            FrozenPreferences {
                automatic_backup: true,
                m4a_conversion: true,
                automatic_trash: false,
            },
        )
        .unwrap();
    ledger
        .advance_batch_phase(RUN_ID, BatchPhase::Copying)
        .unwrap();

    let mut candidates = Vec::new();
    for (index, recording) in initial_scan.recordings.iter().enumerate() {
        let source_relative = &recording.relative_path;
        let source_path = source.join(source_relative);
        let source_digest = hash_file(&source_path).unwrap();
        let destination_relative = Path::new("2026/2026-08-09/TX01")
            .join(source_relative)
            .with_extension("m4a");
        let destination_path = destination.join(&destination_relative);
        fs::create_dir_all(destination_path.parent().unwrap()).unwrap();
        let converted_bytes = deterministic_bytes(0x71 + u8::try_from(index).unwrap(), 4 * 1024);
        fs::write(&destination_path, converted_bytes).unwrap();
        let destination_digest = hash_file(&destination_path).unwrap();
        let metadata = fs::metadata(&source_path).unwrap();
        let recording_id = format!("fat32-acceptance-recording-{index}");
        let candidate = DeletionCandidate {
            recording_id: recording_id.clone(),
            source_relative_path: source_relative.clone(),
            source_size: source_digest.size,
            source_mtime_ns: modified_nanos(&metadata).unwrap(),
            source_sha256: source_digest.sha256.clone(),
            destination_relative_path: destination_relative.clone(),
            destination_size: destination_digest.size,
            destination_sha256: destination_digest.sha256.clone(),
        };
        ledger
            .commit_verified_recording(&VerifiedRecording {
                id: recording_id,
                transmitter: Transmitter::Tx01,
                source_relative_path: source_relative.clone(),
                source_size: source_digest.size,
                source_mtime_ns: candidate.source_mtime_ns,
                source_sha256: source_digest.sha256,
                artifact: VerifiedArtifact {
                    relative_path: destination_relative,
                    format: OutputFormat::M4a,
                    byte_count: destination_digest.size,
                    sha256: destination_digest.sha256,
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
                backup_run_id: RUN_ID.to_owned(),
            })
            .unwrap();
        candidates.push(candidate);
    }

    let mut additional_candidates = Vec::new();
    for (index, additional) in initial_scan.additional_files.iter().enumerate() {
        let source_relative = &additional.relative_path;
        let source_path = source.join(source_relative);
        let source_digest = hash_file(&source_path).unwrap();
        let artifact_relative =
            Path::new("source-extras/2026/2026-08-09/TX01").join(source_relative);
        let artifact_path = destination.join(&artifact_relative);
        fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
        fs::copy(&source_path, &artifact_path).unwrap();
        let artifact_digest = hash_file(&artifact_path).unwrap();
        assert_eq!(source_digest, artifact_digest);
        let additional_id = format!("fat32-acceptance-additional-{index}");
        let metadata = fs::metadata(&source_path).unwrap();
        let candidate = AdditionalDeletionCandidate {
            additional_file_id: additional_id.clone(),
            source_relative_path: source_relative.clone(),
            source_size: source_digest.size,
            source_mtime_ns: modified_nanos(&metadata).unwrap(),
            source_sha256: source_digest.sha256.clone(),
            destination_relative_path: artifact_relative.clone(),
            destination_size: artifact_digest.size,
            destination_sha256: artifact_digest.sha256.clone(),
        };
        ledger
            .commit_verified_additional_file(&VerifiedAdditionalFile {
                id: additional_id,
                transmitter: Transmitter::Tx01,
                source_relative_path: source_relative.clone(),
                source_size: source_digest.size,
                source_mtime_ns: candidate.source_mtime_ns,
                source_sha256: source_digest.sha256,
                artifact_relative_path: artifact_relative,
                artifact_size: artifact_digest.size,
                artifact_sha256: artifact_digest.sha256,
                classification: additional.classification,
                backup_run_id: RUN_ID.to_owned(),
            })
            .unwrap();
        additional_candidates.push(candidate);
    }

    ledger
        .advance_batch_phase(RUN_ID, BatchPhase::CopiesVerified)
        .unwrap();
    let recording_ids = candidates
        .iter()
        .map(|candidate| candidate.recording_id.clone())
        .collect::<Vec<_>>();
    ledger
        .begin_conversion_cohort(RUN_ID, &recording_ids, M4A_PROFILE_ID)
        .unwrap();
    for recording_id in &recording_ids {
        ledger
            .mark_conversion_item_verified(RUN_ID, recording_id)
            .unwrap();
    }
    ledger.commit_m4a_barrier(RUN_ID).unwrap();
    let destination_recordings = candidates
        .iter()
        .map(|candidate| candidate.destination_relative_path.clone())
        .collect::<Vec<_>>();
    let destination_additional = additional_candidates
        .iter()
        .map(|candidate| candidate.destination_relative_path.clone())
        .collect::<Vec<_>>();

    let context = DeletionContext {
        transmitter: Transmitter::Tx01,
        paired_volume_uuid: "isolated-fat32-fixture".to_owned(),
        mount_generation: 1,
        scan_generation: 1,
        destination_generation: 1,
        source_root: source.clone(),
        destination_root: destination.clone(),
    };
    let observed = scan_once(&source, Transmitter::Tx01, UtcOffset::UTC).unwrap();
    assert_eq!(observed.recordings.len(), initial_scan.recordings.len());
    assert_eq!(
        observed.additional_files.len(),
        initial_scan.additional_files.len()
    );
    assert!(observed.issues.is_empty());
    let current_source_paths = observed
        .recordings
        .iter()
        .map(|recording| recording.relative_path.clone())
        .chain(
            observed
                .additional_files
                .iter()
                .map(|file| file.relative_path.clone()),
        )
        .collect::<BTreeSet<_>>();

    let mut store = DeletionProposalStore::default();
    let proposal = store
        .prepare(
            CompleteDeletionSnapshot {
                context: context.clone(),
                candidates,
                additional_files: additional_candidates,
                current_source_paths,
                m4a_barrier_run_id: RUN_ID.to_owned(),
            },
            Duration::from_secs(1),
            true,
            &NoDeletionFaults,
        )
        .unwrap();
    assert_eq!(proposal.session_count, 1);
    assert_eq!(
        proposal.file_count,
        u64::try_from(expected_session_files.len()).unwrap()
    );
    let report = store
        .confirm(
            &proposal.proposal_id,
            DeletionConfirmation {
                current_context: &context,
                now: Duration::from_secs(2),
                started_at: "2026-08-09T00:02:00Z",
                finished_at: "2026-08-09T00:03:00Z",
            },
            &mut ledger,
            &FixtureTrash {
                root: fixture.join("recoverable-trash"),
            },
            &NoDeletionFaults,
        )
        .unwrap();

    assert_eq!(report.outcome, DeletionOutcome::Deleted);
    assert_eq!(
        report.deleted_files,
        u64::try_from(expected_session_files.len()).unwrap()
    );
    assert!(!source.join(&session_relative).exists());
    let trashed_session = fixture.join("recoverable-trash").join(&session_relative);
    assert!(trashed_session.is_dir());
    for (relative, contents) in &expected_session_files {
        assert_eq!(
            fs::read(trashed_session.join(relative.file_name().unwrap())).unwrap(),
            *contents
        );
    }
    for relative in destination_recordings {
        assert!(destination.join(relative).exists());
    }
    for relative in destination_additional {
        assert!(destination.join(relative).exists());
    }
}

fn deterministic_bytes(seed: u8, length: usize) -> Vec<u8> {
    (0..length)
        .map(|index| seed.wrapping_add(u8::try_from(index % 251).unwrap()))
        .collect()
}
