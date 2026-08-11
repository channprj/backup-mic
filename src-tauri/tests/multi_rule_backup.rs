use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use backup_core::{
    artifact::{AudioDescription, OutputFormat},
    backup::{CancellationToken, CopyFaultPoint},
    batch::BatchPhase,
    clock::Clock,
    error::CoreError,
    hash::hash_file,
    ledger::Ledger,
    rule::{BackupRule, BackupRuleDraft, compile_rule},
    rule_scanner::scan_rule_once,
    source::{MountedSourceAuthority, SourceId, SourceRecord},
    state::BackupPhase,
};
use backup_mic_lib::{
    app_state::AppState,
    orchestrator::{
        NoSourceCopyFaults, SourceCopyFaults, overall_backup_phase,
        run_matched_sources_with_adapters,
    },
    platform::{device_registry::MountedVolume, macos::audio::AudioTools},
    rule_runtime::MatchedSource,
};
use tempfile::{TempDir, tempdir};
use time::UtcOffset;

struct InstantClock;

impl Clock for InstantClock {
    fn sleep(&self, _duration: Duration) {}
}

struct FakeAudioTools;

impl AudioTools for FakeAudioTools {
    fn inspect(&self, path: &Path) -> Result<AudioDescription, CoreError> {
        if path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
        {
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

struct FailSource {
    source_id: SourceId,
}

impl SourceCopyFaults for FailSource {
    fn check(&self, source_id: &SourceId, point: CopyFaultPoint) -> Result<(), CoreError> {
        if source_id == &self.source_id && point == CopyFaultPoint::SourceOpen {
            return Err(CoreError::CopyFailed(std::io::Error::other(
                "injected source failure",
            )));
        }
        Ok(())
    }
}

struct EditRuleClock {
    state: AppState,
    draft: BackupRuleDraft,
    edited: AtomicBool,
}

impl Clock for EditRuleClock {
    fn sleep(&self, _duration: Duration) {
        if !self.edited.swap(true, Ordering::SeqCst) {
            self.state
                .save_backup_rule_for_state(self.draft.clone(), "2026-08-10T00:01:00Z")
                .unwrap();
        }
    }
}

struct Fixture {
    _state_root: TempDir,
    _destination: TempDir,
    ledger_path: std::path::PathBuf,
    state: AppState,
}

impl Fixture {
    fn new() -> Self {
        let state_root = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let ledger_path = state_root.path().join("ledger.sqlite3");
        let ledger = Ledger::open(&ledger_path).unwrap();
        let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();
        Self {
            _state_root: state_root,
            _destination: destination,
            ledger_path,
            state,
        }
    }

    fn add_source(
        &self,
        name: &str,
        archive: &str,
        prefix: &str,
        uuid: &str,
    ) -> (TempDir, MatchedSource) {
        let source_root = tempdir().unwrap();
        let directory = source_root.path().join("RECORD/FOLDER01");
        fs::create_dir_all(&directory).unwrap();
        write_pcm_wav(&directory.join("REC0001.WAV"), 17);
        let draft = BackupRuleDraft {
            id: None,
            name: name.to_owned(),
            archive_directory_name: archive.to_owned(),
            enabled: true,
            volume_name_glob: format!("{name}_*"),
            required_path_globs: vec!["RECORD/**".to_owned()],
            backup_file_globs: vec!["RECORD/**/*.WAV".to_owned()],
            session_directory_globs: vec!["RECORD/FOLDER*".to_owned()],
            filename_prefix: prefix.to_owned(),
            filename_suffix: String::new(),
            date_folder_layout: Default::default(),
        };
        let rule = self
            .state
            .save_backup_rule_for_state(draft, "2026-08-10T00:00:00Z")
            .unwrap();
        let source = SourceRecord {
            id: SourceId::new(),
            rule_id: rule.id.clone(),
            volume_uuid: uuid.to_owned(),
            legacy_slot: None,
            display_name: format!("{name}_REC"),
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
            display_name: format!("{name}_REC"),
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

    fn recordings(&self) -> Vec<backup_core::ledger::VerifiedRecording> {
        Ledger::open(&self.ledger_path)
            .unwrap()
            .verified_recordings()
            .unwrap()
    }
}

#[test]
fn two_rules_with_equal_source_names_complete_independent_m4a_barriers() {
    let fixture = Fixture::new();
    let (zoom_root, zoom) = fixture.add_source(
        "ZOOM",
        "Zoom H1n",
        "zoom-",
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    );
    let (sony_root, sony) = fixture.add_source(
        "SONY",
        "Sony PCM",
        "sony-",
        "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
    );

    let outcomes = run_matched_sources_with_adapters(
        &fixture.state,
        &[zoom.clone(), sony.clone()],
        &FakeAudioTools,
        &NoSourceCopyFaults,
        &InstantClock,
        &CancellationToken::default(),
    )
    .unwrap();

    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(|outcome| {
        outcome.phase == BackupPhase::CompletedDeletionPending
            && outcome.deletion_ready
            && outcome.error.is_none()
    }));
    assert_eq!(
        overall_backup_phase(&outcomes),
        BackupPhase::CompletedDeletionPending
    );
    let recordings = fixture.recordings();
    assert_eq!(recordings.len(), 2);
    for (matched, root, archive, prefix) in [
        (&zoom, &zoom_root, "Zoom H1n", "zoom-"),
        (&sony, &sony_root, "Sony PCM", "sony-"),
    ] {
        let recording = recordings
            .iter()
            .find(|recording| recording.source_id == matched.authority.source.id)
            .unwrap();
        assert_eq!(recording.artifact.format, OutputFormat::M4a);
        assert_eq!(recording.artifact.audio.as_ref().unwrap().codec, "aac");
        assert!(recording.artifact.relative_path.starts_with(archive));
        assert!(
            recording
                .artifact
                .relative_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains(prefix)
        );
        let source_digest = hash_file(&root.path().join("RECORD/FOLDER01/REC0001.WAV")).unwrap();
        assert_eq!(recording.source_sha256, source_digest.sha256);
        let ledger = Ledger::open(&fixture.ledger_path).unwrap();
        let superseded = ledger
            .superseded_wav_evidence(&recording.id)
            .unwrap()
            .unwrap();
        assert_eq!(superseded.1, source_digest.size);
        assert_eq!(superseded.2, source_digest.sha256);
        assert_eq!(
            hash_file(
                &fixture
                    ._destination
                    .path()
                    .join(&recording.artifact.relative_path)
            )
            .unwrap()
            .sha256,
            recording.artifact.sha256
        );
        let outcome = outcomes
            .iter()
            .find(|outcome| outcome.source_id == matched.authority.source.id)
            .unwrap();
        assert_eq!(
            ledger
                .batch_run_evidence(&outcome.batch_run_id)
                .unwrap()
                .unwrap()
                .phase,
            BatchPhase::M4aCohortVerified
        );
    }
}

#[test]
fn one_source_copy_failure_does_not_block_the_other_source_cohort() {
    let fixture = Fixture::new();
    let (_zoom_root, zoom) = fixture.add_source(
        "ZOOM",
        "Zoom H1n",
        "zoom-",
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    );
    let (sony_root, sony) = fixture.add_source(
        "SONY",
        "Sony PCM",
        "sony-",
        "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
    );

    let outcomes = run_matched_sources_with_adapters(
        &fixture.state,
        &[zoom.clone(), sony.clone()],
        &FakeAudioTools,
        &FailSource {
            source_id: sony.authority.source.id.clone(),
        },
        &InstantClock,
        &CancellationToken::default(),
    )
    .unwrap();

    let zoom_outcome = outcomes
        .iter()
        .find(|outcome| outcome.source_id == zoom.authority.source.id)
        .unwrap();
    let sony_outcome = outcomes
        .iter()
        .find(|outcome| outcome.source_id == sony.authority.source.id)
        .unwrap();
    assert_eq!(zoom_outcome.phase, BackupPhase::CompletedDeletionPending);
    assert!(zoom_outcome.deletion_ready);
    assert_eq!(sony_outcome.phase, BackupPhase::PartialFailure);
    assert!(!sony_outcome.deletion_ready);
    assert_eq!(overall_backup_phase(&outcomes), BackupPhase::PartialFailure);
    assert!(
        sony_root
            .path()
            .join("RECORD/FOLDER01/REC0001.WAV")
            .is_file()
    );
    let recordings = fixture.recordings();
    assert_eq!(recordings.len(), 1);
    assert_eq!(recordings[0].source_id, zoom.authority.source.id);
    assert_eq!(recordings[0].artifact.format, OutputFormat::M4a);
    assert_eq!(
        Ledger::open(&fixture.ledger_path)
            .unwrap()
            .batch_run_evidence(&sony_outcome.batch_run_id)
            .unwrap()
            .unwrap()
            .phase,
        BatchPhase::Copying
    );
}

#[test]
fn rule_edits_are_frozen_for_the_current_run_and_apply_to_only_new_files_next_run() {
    let fixture = Fixture::new();
    let (zoom_root, zoom) = fixture.add_source(
        "ZOOM",
        "Zoom H1n",
        "zoom-",
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    );
    let mut edited = draft_from_rule(&zoom.rule);
    edited.filename_prefix = "field-".to_owned();
    let _operation = fixture.state.begin_operation().unwrap();
    let edit_clock = EditRuleClock {
        state: fixture.state.clone(),
        draft: edited,
        edited: AtomicBool::new(false),
    };

    run_matched_sources_with_adapters(
        &fixture.state,
        std::slice::from_ref(&zoom),
        &FakeAudioTools,
        &NoSourceCopyFaults,
        &edit_clock,
        &CancellationToken::default(),
    )
    .unwrap();
    assert!(fixture.state.snapshot().setting_applies_next_run);
    let first = fixture.recordings();
    assert_eq!(first.len(), 1);
    assert!(
        first[0]
            .artifact
            .relative_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("zoom-REC0001")
    );

    write_pcm_wav(&zoom_root.path().join("RECORD/FOLDER01/REC0002.WAV"), 29);
    run_matched_sources_with_adapters(
        &fixture.state,
        std::slice::from_ref(&zoom),
        &FakeAudioTools,
        &NoSourceCopyFaults,
        &InstantClock,
        &CancellationToken::default(),
    )
    .unwrap();
    let recordings = fixture.recordings();
    assert_eq!(recordings.len(), 2);
    let names = recordings
        .iter()
        .map(|recording| {
            recording
                .artifact
                .relative_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    assert!(names.iter().any(|name| name.contains("zoom-REC0001")));
    assert!(names.iter().any(|name| name.contains("field-REC0002")));
}

#[test]
fn companion_only_rule_completes_a_source_barrier_without_starting_conversion() {
    let fixture = Fixture::new();
    let (source_root, source) = fixture.add_source(
        "NOTES",
        "Field Notes",
        "notes-",
        "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
    );
    let note_path = source_root.path().join("RECORD/FOLDER01/notes.TXT");
    fs::write(&note_path, b"field notes").unwrap();
    let mut edited = draft_from_rule(&source.rule);
    edited.backup_file_globs = vec!["RECORD/**/*.TXT".to_owned()];
    edited.session_directory_globs.clear();
    fixture
        .state
        .save_backup_rule_for_state(edited, "2026-08-10T00:01:00Z")
        .unwrap();

    let outcomes = run_matched_sources_with_adapters(
        &fixture.state,
        std::slice::from_ref(&source),
        &FakeAudioTools,
        &NoSourceCopyFaults,
        &InstantClock,
        &CancellationToken::default(),
    )
    .unwrap();

    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].deletion_ready);
    let ledger = Ledger::open(&fixture.ledger_path).unwrap();
    assert_eq!(
        ledger
            .batch_run_evidence(&outcomes[0].batch_run_id)
            .unwrap()
            .unwrap()
            .phase,
        BatchPhase::M4aCohortVerified
    );
    assert!(ledger.verified_recordings().unwrap().is_empty());
    let metadata = fs::metadata(&note_path).unwrap();
    let digest = hash_file(&note_path).unwrap();
    assert!(
        ledger
            .verified_additional_file_for_source(
                &source.authority.source.id,
                Path::new("RECORD/FOLDER01/notes.TXT"),
                metadata.len(),
                backup_core::filesystem::modified_nanos(&metadata).unwrap(),
                &digest.sha256,
            )
            .unwrap()
            .is_some()
    );
}

fn write_pcm_wav(path: &Path, sample: i16) {
    let samples = 48_000_u32;
    let data_bytes = samples * 2;
    let mut bytes = Vec::with_capacity((44 + data_bytes) as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&48_000_u32.to_le_bytes());
    bytes.extend_from_slice(&96_000_u32.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for _ in 0..samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, bytes).unwrap();
}

fn draft_from_rule(rule: &BackupRule) -> BackupRuleDraft {
    BackupRuleDraft {
        id: Some(rule.id.clone()),
        name: rule.name.clone(),
        archive_directory_name: rule.archive_directory_name.clone(),
        enabled: rule.enabled,
        volume_name_glob: rule.volume_name_glob.clone(),
        required_path_globs: rule.required_path_globs.clone(),
        backup_file_globs: rule.backup_file_globs.clone(),
        session_directory_globs: rule.session_directory_globs.clone(),
        filename_prefix: rule.filename_prefix.clone(),
        filename_suffix: rule.filename_suffix.clone(),
        date_folder_layout: rule.date_folder_layout,
    }
}
