use std::{fs, path::PathBuf};

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
    filesystem::modified_nanos,
    hash::hash_file,
    ledger::{Ledger, VerifiedRecording},
    rule::BackupRuleDraft,
    source::{MountedSourceAuthority, SourceId, SourceRecord},
};
use backup_mic_lib::platform::macos::{
    audio::{AppleAudioTools, AudioTools},
    trash::MacTrash,
};
use tempfile::tempdir;
use time::{Duration, OffsetDateTime};

const FIXTURE_ROOT: &str = "/Volumes/DJI-DELTEST";
const MARKER: &str = ".dji-mic-backup-delete-fixture";

#[test]
#[ignore = "requires scripts/accept-deletion-fixture.sh"]
fn foundation_moves_a_whole_session_on_the_isolated_fat32_fixture() {
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

    let session = fixture.join("TX_MIC001_20260809_021747");
    assert!(!session.exists());
    fs::create_dir(&session).unwrap();
    let files = [
        ("TX01_MIC001_20260809_021747.wav", write_pcm_wav(17)),
        ("TX01_MIC001_20260809_021748.wav", write_pcm_wav(29)),
        ("recorder-preview.m4a", b"external M4A fixture".to_vec()),
        (
            "._TX01_MIC001_20260809_021747.wav",
            b"AppleDouble fixture".to_vec(),
        ),
    ];
    for (name, contents) in &files {
        fs::write(session.join(name), contents).unwrap();
    }

    let destination = fixture.join("platform-destination");
    fs::create_dir(&destination).unwrap();
    let tools = AppleAudioTools;
    for (name, _) in &files[..2] {
        let source_wav = session.join(name);
        let destination_m4a = destination.join(PathBuf::from(name).with_extension("m4a"));
        tools
            .convert_aac_lc_128k(&source_wav, &destination_m4a)
            .unwrap();
        let inspected = tools.inspect(&destination_m4a).unwrap();
        assert_eq!(inspected.container, "m4af");
        assert_eq!(inspected.codec, "aac");
        assert_eq!(inspected.sample_rate_hz, 48_000);
        assert_eq!(inspected.channels, 1);
        assert_eq!(inspected.valid_frames, 48_000);
    }
    for (name, _) in &files[2..] {
        let source_extra = session.join(name);
        let destination_extra = destination.join("source-extras").join(name);
        fs::create_dir_all(destination_extra.parent().unwrap()).unwrap();
        fs::copy(&source_extra, &destination_extra).unwrap();
        assert_eq!(
            hash_file(&source_extra).unwrap(),
            hash_file(&destination_extra).unwrap()
        );
    }

    let empty_session = fixture.join("TX_MIC001_20260809_021749");
    assert!(!empty_session.exists());
    fs::create_dir(&empty_session).unwrap();

    MacTrash.move_to_trash(&session).unwrap();
    MacTrash.move_to_trash(&empty_session).unwrap();

    assert!(!session.exists());
    assert!(!empty_session.exists());

    // Direct Trash inspection is intentionally confined to this disposable
    // acceptance image. Production code only calls Foundation's Trash API.
    let trash = fixture
        .join(".Trashes")
        .join(unsafe { libc::geteuid() }.to_string());
    let trashed_session = trash.join("TX_MIC001_20260809_021747");
    assert!(trashed_session.is_dir());
    for (name, contents) in files {
        assert_eq!(fs::read(trashed_session.join(name)).unwrap(), contents);
    }
    assert!(trash.join("TX_MIC001_20260809_021749").is_dir());
}

#[test]
#[ignore = "requires scripts/accept-deletion-fixture.sh"]
fn foundation_moves_only_the_requested_generic_session_on_the_isolated_fat32_fixture() {
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

    let dji_session = fixture.join("TX_MIC001_20260809_031000");
    fs::create_dir(&dji_session).unwrap();
    fs::write(
        dji_session.join("TX01_MIC001_20260809_031000.wav"),
        write_pcm_wav(11),
    )
    .unwrap();

    let zoom_root = fixture.join("ZOOM_TEST");
    let zoom_session = zoom_root.join("RECORD/FOLDER01");
    fs::create_dir_all(&zoom_session).unwrap();
    let source_relative = PathBuf::from("RECORD/FOLDER01/ZOOM0001.wav");
    let source_path = zoom_root.join(&source_relative);
    fs::write(&source_path, write_pcm_wav(31)).unwrap();

    let destination = fixture.join("generic-platform-destination");
    fs::create_dir(&destination).unwrap();
    let destination_relative = PathBuf::from("Zoom Archive/ZOOM0001.m4a");
    let destination_path = destination.join(&destination_relative);
    fs::create_dir_all(destination_path.parent().unwrap()).unwrap();
    let tools = AppleAudioTools;
    tools
        .convert_aac_lc_128k(&source_path, &destination_path)
        .unwrap();
    let inspected = tools.inspect(&destination_path).unwrap();
    assert_eq!(inspected.container, "m4af");
    assert_eq!(inspected.codec, "aac");

    let state = tempdir().unwrap();
    let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
    let rule = ledger
        .save_backup_rule(
            BackupRuleDraft {
                id: None,
                name: "Zoom FAT32".to_owned(),
                archive_directory_name: "Zoom Archive".to_owned(),
                enabled: true,
                volume_name_glob: "ZOOM_TEST".to_owned(),
                required_path_globs: vec!["RECORD/**".to_owned()],
                backup_file_globs: vec!["RECORD/**/*.wav".to_owned()],
                session_directory_globs: vec!["RECORD/FOLDER*".to_owned()],
                filename_prefix: "zoom-".to_owned(),
                filename_suffix: String::new(),
            },
            "2026-08-10T00:00:00Z",
        )
        .unwrap();
    let source = SourceRecord {
        id: SourceId::new(),
        rule_id: rule.id.clone(),
        volume_uuid: "fat32-zoom-fixture".to_owned(),
        legacy_slot: None,
        display_name: "ZOOM_TEST".to_owned(),
    };
    ledger
        .upsert_source(&source, "2026-08-10T00:00:00Z")
        .unwrap();
    ledger
        .begin_batch_run(
            "fat32-zoom-run",
            &source.id,
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
        .advance_batch_phase("fat32-zoom-run", BatchPhase::Copying)
        .unwrap();
    let source_digest = hash_file(&source_path).unwrap();
    let destination_digest = hash_file(&destination_path).unwrap();
    let source_mtime_ns = modified_nanos(&fs::metadata(&source_path).unwrap()).unwrap();
    let recording_id = "fat32-zoom-recording".to_owned();
    ledger
        .commit_verified_recording(&VerifiedRecording {
            id: recording_id.clone(),
            source_id: source.id.clone(),
            source_relative_path: source_relative.clone(),
            source_size: source_digest.size,
            source_mtime_ns,
            source_sha256: source_digest.sha256.clone(),
            artifact: VerifiedArtifact {
                relative_path: destination_relative.clone(),
                format: OutputFormat::M4a,
                byte_count: destination_digest.size,
                sha256: destination_digest.sha256.clone(),
                audio: Some(VerifiedAudioProperties {
                    codec: inspected.codec,
                    sample_rate_hz: inspected.sample_rate_hz,
                    channel_count: inspected.channels,
                    valid_frames: inspected.valid_frames,
                    duration_micros: inspected.duration_micros,
                }),
            },
            conversion_status: ConversionStatus::Complete,
            conversion_error_code: None,
            retirement_status: RetirementStatus::Present,
            retired_session_relative_path: None,
            verified_at: "2026-08-10T00:01:00Z".to_owned(),
            backup_run_id: "fat32-zoom-run".to_owned(),
        })
        .unwrap();
    ledger
        .advance_batch_phase("fat32-zoom-run", BatchPhase::CopiesVerified)
        .unwrap();
    ledger
        .begin_conversion_cohort(
            "fat32-zoom-run",
            std::slice::from_ref(&recording_id),
            M4A_PROFILE_ID,
        )
        .unwrap();
    ledger
        .mark_conversion_item_verified("fat32-zoom-run", &recording_id)
        .unwrap();
    ledger.commit_m4a_barrier("fat32-zoom-run").unwrap();

    let candidate = DeletionCandidate {
        recording_id,
        source_relative_path: source_relative,
        source_size: source_digest.size,
        source_mtime_ns,
        source_sha256: source_digest.sha256,
        destination_relative_path: destination_relative,
        destination_size: destination_digest.size,
        destination_sha256: destination_digest.sha256.clone(),
    };
    let authority = MountedSourceAuthority {
        source: source.clone(),
        descriptor: VolumeDescriptor {
            volume_uuid: source.volume_uuid.clone(),
            mount_root: zoom_root.clone(),
            protocol: "USB".to_owned(),
            is_internal: false,
            is_removable: true,
            is_writable: true,
            media_name: "Recorder".to_owned(),
            nominal_capacity: 64 * 1024 * 1024,
            mount_generation: 1,
        },
    };
    let context = RuleDeletionContext {
        source_id: &source.id,
        rule_id: &rule.id,
        rule_updated_at: &rule.updated_at,
        authority: &authority,
        destination_generation: 1,
        scan_generation: 1,
    };
    let now = OffsetDateTime::from_unix_timestamp(1_786_320_000).unwrap();
    let mut proposals = DeletionProposalStore::default();
    let proposal = proposals
        .prepare_rule(
            context,
            &rule,
            CompleteRuleDeletionSnapshot {
                files: Vec::new(),
                additional_files: Vec::new(),
                sessions: vec![RuleSessionDeletionCandidate {
                    relative_directory: PathBuf::from("RECORD/FOLDER01"),
                    files: vec![candidate],
                    additional_files: Vec::new(),
                }],
                destination_root: destination.clone(),
                m4a_barrier_run_id: "fat32-zoom-run".to_owned(),
            },
            now,
            true,
            &ledger,
            &NoDeletionFaults,
        )
        .unwrap();
    let report = proposals
        .confirm_rule(
            &proposal.proposal_id,
            RuleDeletionConfirmation {
                current_context: context,
                current_rule: &rule,
                now: now + Duration::seconds(1),
                started_at: "2026-08-10T00:02:00Z",
                finished_at: "2026-08-10T00:03:00Z",
            },
            &mut ledger,
            &MacTrash,
            &NoDeletionFaults,
        )
        .unwrap();
    assert_eq!(report.outcome, DeletionOutcome::Deleted);
    assert!(!zoom_session.exists());
    assert!(zoom_root.join("RECORD").is_dir());
    assert!(dji_session.is_dir());
    assert_eq!(
        hash_file(&destination_path).unwrap().sha256,
        destination_digest.sha256
    );

    let trash = fixture
        .join(".Trashes")
        .join(unsafe { libc::geteuid() }.to_string());
    assert!(trash.join("FOLDER01").is_dir());
}

fn write_pcm_wav(pattern: i16) -> Vec<u8> {
    let sample_rate = 48_000_u32;
    let channels = 1_u16;
    let frames = 48_000_u32;
    let bits_per_sample = 16_u16;
    let block_align = channels * (bits_per_sample / 8);
    let byte_rate = sample_rate * u32::from(block_align);
    let data_size = frames * u32::from(block_align);
    let mut wav = Vec::with_capacity(44 + usize::try_from(data_size).unwrap());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    for frame in 0..frames {
        let sample = (((frame % 200) as i16) - 100) * pattern;
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}
