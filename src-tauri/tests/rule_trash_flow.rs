use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use backup_core::{
    artifact::AudioDescription,
    backup::CancellationToken,
    clock::Clock,
    deletion::{DeletionOutcome, TrashAdapter},
    error::CoreError,
    ledger::Ledger,
    preferences::PreferenceKey,
    rule::{BackupRuleDraft, compile_rule},
    rule_scanner::scan_rule_once,
    source::{MountedSourceAuthority, SourceId, SourceRecord},
};
use backup_mic_lib::{
    app_state::AppState,
    orchestrator::{
        NoSourceCopyFaults, confirm_rule_trash_with_adapter, prepare_rule_trash,
        retire_ready_rule_sources_with_adapter, run_matched_sources_with_adapters,
    },
    platform::{device_registry::MountedVolume, macos::audio::AudioTools},
    rule_runtime::MatchedSource,
};
use tempfile::{TempDir, tempdir};
use time::{OffsetDateTime, UtcOffset};

struct InstantClock;

struct BackupTrash;

impl TrashAdapter for BackupTrash {
    fn move_to_trash(&self, absolute_path: &Path) -> Result<(), CoreError> {
        fs::remove_file(absolute_path).map_err(CoreError::CopyFailed)
    }
}

struct RefuseBackupTrash;

impl TrashAdapter for RefuseBackupTrash {
    fn move_to_trash(&self, _absolute_path: &Path) -> Result<(), CoreError> {
        Err(CoreError::TrashFailed)
    }
}

impl Clock for InstantClock {
    fn sleep(&self, _duration: Duration) {}
}

struct FakeAudioTools;

impl AudioTools for FakeAudioTools {
    fn inspect(&self, path: &Path) -> Result<AudioDescription, CoreError> {
        if path.extension().is_some_and(|extension| extension == "wav") {
            Ok(AudioDescription {
                container: "WAVE".to_owned(),
                codec: "lpcm".to_owned(),
                sample_rate_hz: 48_000,
                channels: 1,
                audio_bytes: 96_000,
                packets: 48_000,
                frames_per_packet: 1,
                valid_frames: 48_000,
                total_frames: 48_000,
                priming_frames: 0,
                remainder_frames: 0,
                duration_micros: 1_000_000,
            })
        } else {
            Ok(AudioDescription {
                container: "m4af".to_owned(),
                codec: "aac".to_owned(),
                sample_rate_hz: 48_000,
                channels: 1,
                audio_bytes: 4,
                packets: 47,
                frames_per_packet: 1_024,
                valid_frames: 48_000,
                total_frames: 50_176,
                priming_frames: 2_112,
                remainder_frames: 64,
                duration_micros: 1_000_000,
            })
        }
    }

    fn convert_aac_lc_128k(&self, _input: &Path, output: &Path) -> Result<(), CoreError> {
        fs::write(output, b"m4a!").map_err(CoreError::CopyFailed)
    }
}

struct FakeTrash {
    root: TempDir,
    refused_root: Option<PathBuf>,
    calls: AtomicUsize,
    moved: Mutex<Vec<PathBuf>>,
}

impl FakeTrash {
    fn new(refused_root: Option<PathBuf>) -> Self {
        Self {
            root: tempdir().unwrap(),
            refused_root,
            calls: AtomicUsize::new(0),
            moved: Mutex::new(Vec::new()),
        }
    }
}

impl TrashAdapter for FakeTrash {
    fn move_to_trash(&self, absolute_path: &Path) -> Result<(), CoreError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self
            .refused_root
            .as_deref()
            .is_some_and(|root| absolute_path.starts_with(root))
        {
            return Err(CoreError::TrashFailed);
        }
        let target = self
            .root
            .path()
            .join(format!("moved-{}", self.calls.load(Ordering::SeqCst)));
        fs::rename(absolute_path, target).map_err(CoreError::CopyFailed)?;
        self.moved.lock().unwrap().push(absolute_path.to_path_buf());
        Ok(())
    }
}

struct Fixture {
    _state_root: TempDir,
    destination: TempDir,
    state: AppState,
}

impl Fixture {
    fn new() -> Self {
        let state_root = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let ledger = Ledger::open(state_root.path().join("ledger.sqlite3")).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();
        Self {
            _state_root: state_root,
            destination,
            state,
        }
    }

    fn add_source(&self, name: &str, uuid: &str) -> (TempDir, MatchedSource) {
        let source_root = tempdir().unwrap();
        let session = source_root.path().join("RECORD/FOLDER01");
        fs::create_dir_all(&session).unwrap();
        write_pcm_wav(&session.join("REC0001.wav"));
        let rule = self
            .state
            .save_backup_rule_for_state(
                BackupRuleDraft {
                    id: None,
                    name: name.to_owned(),
                    archive_directory_name: format!("{name} Archive"),
                    enabled: true,
                    volume_name_glob: format!("{name}*"),
                    required_path_globs: vec!["RECORD/**".to_owned()],
                    backup_file_globs: vec!["RECORD/**/*.wav".to_owned()],
                    session_directory_globs: vec!["RECORD/FOLDER*".to_owned()],
                    filename_prefix: format!("{}-", name.to_ascii_lowercase()),
                    filename_suffix: String::new(),
                    date_folder_layout: Default::default(),
                },
                "2026-08-10T00:00:00Z",
            )
            .unwrap();
        let source = SourceRecord {
            id: SourceId::new(),
            rule_id: rule.id.clone(),
            volume_uuid: uuid.to_owned(),
            legacy_slot: None,
            display_name: format!("{name} RECORDER"),
        };
        self.state
            .upsert_source_for_state(&source, "2026-08-10T00:00:00Z")
            .unwrap();
        let mounted = MountedVolume {
            descriptor: backup_core::device::VolumeDescriptor {
                volume_uuid: uuid.to_owned(),
                mount_root: source_root.path().to_path_buf(),
                protocol: "USB".to_owned(),
                is_internal: false,
                is_removable: true,
                is_writable: true,
                media_name: "Generic Recorder".to_owned(),
                nominal_capacity: 32_000_000_000,
                mount_generation: 1,
            },
            display_name: format!("{name} RECORDER"),
        };
        let fingerprint = scan_rule_once(
            source_root.path(),
            &compile_rule(rule.clone()).unwrap(),
            UtcOffset::UTC,
        )
        .unwrap()
        .fingerprint;
        let matched = MatchedSource {
            authority: MountedSourceAuthority {
                source,
                descriptor: mounted.descriptor.clone(),
            },
            rule,
            mounted,
            initial_fingerprint: fingerprint,
        };
        self.state.install_matched_source_for_state(matched.clone());
        (source_root, matched)
    }
}

#[test]
fn generic_manual_trash_uses_the_frozen_source_id_and_moves_one_complete_session() {
    let fixture = Fixture::new();
    let (source_root, source) = fixture.add_source("ZOOM", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
    let outcomes = run_matched_sources_with_adapters(
        &fixture.state,
        std::slice::from_ref(&source),
        &FakeAudioTools,
        &NoSourceCopyFaults,
        &BackupTrash,
        &InstantClock,
        &CancellationToken::default(),
    )
    .unwrap();
    assert!(outcomes[0].deletion_ready);

    let now = OffsetDateTime::from_unix_timestamp(1_786_320_000).unwrap();
    let proposal = prepare_rule_trash(&fixture.state, &source.authority.source.id, now).unwrap();
    assert_eq!(proposal.source_id, source.authority.source.id);
    assert_eq!(proposal.session_count, 1);
    let trash = FakeTrash::new(None);
    let report = confirm_rule_trash_with_adapter(
        &fixture.state,
        &source.authority.source.id,
        &proposal.proposal_id,
        now + time::Duration::seconds(1),
        &trash,
        &mut || {},
    )
    .unwrap();
    assert_eq!(report.outcome, DeletionOutcome::Deleted);
    assert_eq!(trash.calls.load(Ordering::SeqCst), 1);
    assert!(!source_root.path().join("RECORD/FOLDER01").exists());
    assert!(fixture.destination.path().join("ZOOM Archive").exists());
}

#[test]
fn backup_wav_trash_failure_never_authorizes_source_retirement() {
    let fixture = Fixture::new();
    let (source_root, source) = fixture.add_source("ZOOM", "dddddddd-dddd-4ddd-8ddd-dddddddddddd");

    let outcomes = run_matched_sources_with_adapters(
        &fixture.state,
        std::slice::from_ref(&source),
        &FakeAudioTools,
        &NoSourceCopyFaults,
        &RefuseBackupTrash,
        &InstantClock,
        &CancellationToken::default(),
    )
    .unwrap();

    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].error.is_some());
    assert!(!outcomes[0].deletion_ready);
    assert!(outcomes[0].deletion_evidence.is_none());
    assert!(
        source_root
            .path()
            .join("RECORD/FOLDER01/REC0001.wav")
            .is_file()
    );
    assert!(
        fixture
            .destination
            .path()
            .join("ZOOM Archive")
            .read_dir()
            .unwrap()
            .any(|entry| entry.unwrap().path().is_dir())
    );
    assert_eq!(
        Ledger::open(fixture._state_root.path().join("ledger.sqlite3"))
            .unwrap()
            .pending_superseded_wavs()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn automatic_trash_refusal_for_one_source_does_not_block_another_source() {
    let fixture = Fixture::new();
    fixture
        .state
        .set_preference(PreferenceKey::AutomaticTrash, true, "2026-08-10T00:00:00Z")
        .unwrap();
    let (zoom_root, zoom) = fixture.add_source("ZOOM", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
    let (sony_root, sony) = fixture.add_source("SONY", "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
    let outcomes = run_matched_sources_with_adapters(
        &fixture.state,
        &[zoom.clone(), sony.clone()],
        &FakeAudioTools,
        &NoSourceCopyFaults,
        &BackupTrash,
        &InstantClock,
        &CancellationToken::default(),
    )
    .unwrap();
    let trash = FakeTrash::new(Some(fs::canonicalize(sony_root.path()).unwrap()));
    let retirements = retire_ready_rule_sources_with_adapter(
        &fixture.state,
        &outcomes,
        OffsetDateTime::from_unix_timestamp(1_786_320_000).unwrap(),
        &trash,
        &CancellationToken::default(),
    )
    .unwrap();

    assert_eq!(retirements.len(), 2);
    assert_eq!(
        retirements
            .iter()
            .find(|outcome| outcome.source_id == zoom.authority.source.id)
            .unwrap()
            .report
            .as_ref()
            .unwrap()
            .outcome,
        DeletionOutcome::Deleted
    );
    assert_eq!(
        retirements
            .iter()
            .find(|outcome| outcome.source_id == sony.authority.source.id)
            .unwrap()
            .report
            .as_ref()
            .unwrap()
            .outcome,
        DeletionOutcome::Refused
    );
    assert!(!zoom_root.path().join("RECORD/FOLDER01").exists());
    assert!(
        sony_root
            .path()
            .join("RECORD/FOLDER01/REC0001.wav")
            .is_file()
    );
    assert!(fixture.destination.path().join("ZOOM Archive").exists());
    assert!(fixture.destination.path().join("SONY Archive").exists());
}

#[test]
fn cancellation_before_automatic_retirement_keeps_source_files_in_place() {
    let fixture = Fixture::new();
    fixture
        .state
        .set_preference(PreferenceKey::AutomaticTrash, true, "2026-08-10T00:00:00Z")
        .unwrap();
    let (source_root, source) = fixture.add_source("ZOOM", "cccccccc-cccc-4ccc-8ccc-cccccccccccc");
    let outcomes = run_matched_sources_with_adapters(
        &fixture.state,
        std::slice::from_ref(&source),
        &FakeAudioTools,
        &NoSourceCopyFaults,
        &BackupTrash,
        &InstantClock,
        &CancellationToken::default(),
    )
    .unwrap();
    let cancellation = CancellationToken::default();
    cancellation.cancel();

    let result = retire_ready_rule_sources_with_adapter(
        &fixture.state,
        &outcomes,
        OffsetDateTime::from_unix_timestamp(1_786_320_000).unwrap(),
        &FakeTrash::new(None),
        &cancellation,
    );

    assert!(matches!(result, Err(CoreError::Cancelled)));
    assert!(source_root.path().join("RECORD/FOLDER01").is_dir());
    assert!(
        source_root
            .path()
            .join("RECORD/FOLDER01/REC0001.wav")
            .is_file()
    );
}

#[test]
fn wav_only_backup_never_creates_source_retirement_authority() {
    let fixture = Fixture::new();
    fixture
        .state
        .set_preference(PreferenceKey::M4aConversion, false, "2026-08-10T00:00:00Z")
        .unwrap();
    let (source_root, source) = fixture.add_source("ZOOM", "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
    let outcomes = run_matched_sources_with_adapters(
        &fixture.state,
        std::slice::from_ref(&source),
        &FakeAudioTools,
        &NoSourceCopyFaults,
        &BackupTrash,
        &InstantClock,
        &CancellationToken::default(),
    )
    .unwrap();
    assert!(!outcomes[0].deletion_ready);
    assert!(matches!(
        prepare_rule_trash(
            &fixture.state,
            &source.authority.source.id,
            OffsetDateTime::UNIX_EPOCH,
        ),
        Err(CoreError::DeletionPreflightRefused)
    ));
    assert!(
        source_root
            .path()
            .join("RECORD/FOLDER01/REC0001.wav")
            .is_file()
    );
}

fn write_pcm_wav(path: &Path) {
    let frames = 48_000_u32;
    let data_size = frames * 2;
    let mut bytes = Vec::with_capacity((44 + data_size) as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&48_000_u32.to_le_bytes());
    bytes.extend_from_slice(&96_000_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_size.to_le_bytes());
    for _ in 0..frames {
        bytes.extend_from_slice(&17_i16.to_le_bytes());
    }
    fs::write(path, bytes).unwrap();
}
