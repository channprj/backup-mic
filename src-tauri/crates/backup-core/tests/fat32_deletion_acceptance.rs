use std::{collections::BTreeSet, fs, path::PathBuf, time::Duration};

use backup_core::{
    artifact::{ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact},
    deletion::{
        CompleteDeletionSnapshot, DeletionConfirmation, DeletionContext, DeletionOutcome,
        DeletionProposalStore, NoDeletionFaults,
    },
    filesystem::modified_nanos,
    hash::hash_file,
    ledger::{Ledger, VerifiedRecording},
    state::Transmitter,
};
use tempfile::tempdir;

const FIXTURE_ROOT: &str = "/Volumes/DJI-DELTEST";
const MARKER: &str = ".dji-mic-backup-delete-fixture";

#[test]
#[ignore = "requires scripts/accept-deletion-fixture.sh"]
fn deletes_only_reverified_sources_on_an_isolated_fat32_volume() {
    let requested = std::env::var_os("DJI_MIC_DELETION_FIXTURE")
        .map(PathBuf::from)
        .expect("fixture path is required");
    assert_eq!(requested, PathBuf::from(FIXTURE_ROOT));
    let fixture = fs::canonicalize(&requested).expect("fixture must be mounted");
    assert_eq!(fixture, PathBuf::from(FIXTURE_ROOT));
    assert_eq!(
        fs::read_to_string(fixture.join(MARKER)).expect("fixture marker is required"),
        "isolated-fat32-deletion-test\n"
    );

    let source = fixture.join("source");
    let destination = fixture.join("destination");
    assert!(!source.exists() && !destination.exists());
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();

    let source_relative = PathBuf::from("TX01_MIC001_20260809_010203.wav");
    let destination_relative = PathBuf::from("2026/2026-08-09/TX01").join(&source_relative);
    let source_path = source.join(&source_relative);
    let destination_path = destination.join(&destination_relative);
    fs::create_dir_all(destination_path.parent().unwrap()).unwrap();
    let audio = vec![0x5a; 8 * 1024];
    fs::write(&source_path, &audio).unwrap();
    fs::write(&destination_path, &audio).unwrap();

    let source_metadata = fs::metadata(&source_path).unwrap();
    let digest = hash_file(&source_path).unwrap();
    let candidate = backup_core::deletion::DeletionCandidate {
        recording_id: "fat32-acceptance-recording".to_owned(),
        source_relative_path: source_relative.clone(),
        source_size: digest.size,
        source_mtime_ns: modified_nanos(&source_metadata).unwrap(),
        source_sha256: digest.sha256.clone(),
        destination_relative_path: destination_relative.clone(),
        destination_size: digest.size,
        destination_sha256: digest.sha256.clone(),
    };

    let state = tempdir().unwrap();
    let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
    ledger
        .begin_backup_run(
            "fat32-acceptance-backup",
            "2026-08-09T00:00:00Z",
            digest.size,
        )
        .unwrap();
    ledger
        .commit_verified_recording(&VerifiedRecording {
            id: candidate.recording_id.clone(),
            transmitter: Transmitter::Tx01,
            source_relative_path: source_relative.clone(),
            source_size: digest.size,
            source_mtime_ns: candidate.source_mtime_ns,
            source_sha256: digest.sha256.clone(),
            artifact: VerifiedArtifact {
                relative_path: destination_relative.clone(),
                format: OutputFormat::Wav,
                byte_count: digest.size,
                sha256: digest.sha256,
                audio: None,
            },
            conversion_status: ConversionStatus::NotRequired,
            conversion_error_code: None,
            retirement_status: RetirementStatus::Present,
            retired_session_relative_path: None,
            verified_at: "2026-08-09T00:01:00Z".to_owned(),
            backup_run_id: "fat32-acceptance-backup".to_owned(),
        })
        .unwrap();

    let context = DeletionContext {
        transmitter: Transmitter::Tx01,
        paired_volume_uuid: "isolated-fat32-fixture".to_owned(),
        mount_generation: 1,
        scan_generation: 1,
        destination_generation: 1,
        source_root: source.clone(),
        destination_root: destination,
    };
    let mut store = DeletionProposalStore::default();
    let proposal = store
        .prepare(
            CompleteDeletionSnapshot {
                context: context.clone(),
                candidates: vec![candidate],
                current_source_paths: BTreeSet::from([source_relative]),
            },
            Duration::from_secs(1),
            true,
            &NoDeletionFaults,
        )
        .unwrap();
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
            &NoDeletionFaults,
        )
        .unwrap();

    assert_eq!(report.outcome, DeletionOutcome::Deleted);
    assert_eq!(report.deleted_files, 1);
    assert!(!source_path.exists());
    assert!(destination_path.exists());
    assert_eq!(fs::read(&destination_path).unwrap(), audio);
}
