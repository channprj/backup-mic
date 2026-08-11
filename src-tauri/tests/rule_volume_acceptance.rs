use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use backup_core::{
    artifact::OutputFormat, backup::CancellationToken, batch::BatchPhase, clock::Clock,
    hash::hash_file, ledger::Ledger, rule::BackupRuleDraft, state::BackupPhase,
};
use backup_mic_lib::{
    app_state::AppState,
    orchestrator::{NoSourceCopyFaults, run_matched_sources_with_adapters},
    platform::{
        device_registry::{
            DeviceRegistry, NativeDiskDescription, NativeDiskEvent, VolumeLifecycleEvent,
        },
        macos::audio::AppleAudioTools,
    },
    rule_runtime::{RuleVolumeMatch, match_mounted_volume},
};
use tempfile::tempdir;
use time::UtcOffset;

const MISSING_FIXTURES: &str =
    "BACKUP_MIC_RULE_FIXTURES_REQUIRED: run scripts/accept-rule-fixtures.sh";
const SENTINEL: &str = ".backup-mic-rule-fixture";

struct InstantClock;

impl Clock for InstantClock {
    fn sleep(&self, _duration: Duration) {}
}

#[test]
#[ignore = "requires scripts/accept-rule-fixtures.sh"]
fn backs_up_two_isolated_fat32_rule_volumes_through_the_production_pipeline() {
    let zoom_root = required_fixture("BACKUP_MIC_ZOOM_RULE_FIXTURE", "ZOOM_RULETEST");
    let sony_root = required_fixture("BACKUP_MIC_SONY_RULE_FIXTURE", "SONY_RULETEST");
    assert_ne!(zoom_root, sony_root);

    let state_root = tempdir().unwrap();
    let destination = tempdir().unwrap();
    let ledger_path = state_root.path().join("ledger.sqlite3");
    let mut ledger = Ledger::open(&ledger_path).unwrap();
    for draft in [
        rule(
            "Zoom H1n",
            "Zoom H1n",
            "ZOOM_*",
            "RECORD/**",
            "RECORD/**/*.WAV",
            "RECORD/FOLDER*",
            "zoom-",
            "-field",
        ),
        rule(
            "Sony PCM",
            "Sony PCM",
            "SONY_*",
            "REC_FILE/**",
            "REC_FILE/**/*.WAV",
            "REC_FILE/FOLDER*",
            "sony-",
            "-take",
        ),
    ] {
        ledger
            .save_backup_rule(draft, "2026-08-11T00:00:00Z")
            .unwrap();
    }
    let state = AppState::new(ledger, destination.path().to_path_buf(), true, false).unwrap();

    let mut registry = DeviceRegistry::default();
    let mut matching_ledger = Ledger::open(&ledger_path).unwrap();
    let rules = matching_ledger.backup_rules(false).unwrap();
    let mut matched_sources = Vec::new();
    for (index, (label, root)) in [("ZOOM_RULETEST", &zoom_root), ("SONY_RULETEST", &sony_root)]
        .into_iter()
        .enumerate()
    {
        let events = registry.apply(NativeDiskEvent::Appeared(NativeDiskDescription {
            disk_id: format!("rule-fixture-{index}"),
            volume_uuid: Some(format!("00000000-0000-4000-8000-{index:012}")),
            mount_root: Some(root.clone()),
            protocol: Some("USB".to_owned()),
            is_internal: Some(false),
            is_removable: Some(true),
            is_writable: Some(true),
            media_name: Some("Disposable FAT32 recorder fixture".to_owned()),
            nominal_capacity: Some(64 * 1024 * 1024),
            display_name: Some(label.to_owned()),
        }));
        assert_eq!(events.len(), 1);
        let VolumeLifecycleEvent::Mounted(mounted) = events.into_iter().next().unwrap() else {
            panic!("fixture must produce a mounted lifecycle event");
        };
        let RuleVolumeMatch::Matched(matched) = match_mounted_volume(
            &mut matching_ledger,
            &mounted,
            &rules,
            UtcOffset::UTC,
            "2026-08-11T00:00:01Z",
        )
        .unwrap() else {
            panic!("fixture must match exactly one recorder rule");
        };
        state
            .upsert_source_for_state(&matched.authority.source, "2026-08-11T00:00:01Z")
            .unwrap();
        state.install_matched_source_for_state((*matched).clone());
        matched_sources.push(*matched);
    }
    drop(matching_ledger);

    let outcomes = run_matched_sources_with_adapters(
        &state,
        &matched_sources,
        &AppleAudioTools,
        &NoSourceCopyFaults,
        &InstantClock,
        &CancellationToken::default(),
    )
    .unwrap();

    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.iter().all(|outcome| {
        outcome.phase == BackupPhase::CompletedDeletionPending
            && outcome.deletion_ready
            && outcome.verified_files == 1
            && outcome.error.is_none()
    }));
    assert_ne!(outcomes[0].batch_run_id, outcomes[1].batch_run_id);

    let ledger = Ledger::open(&ledger_path).unwrap();
    let recordings = ledger.verified_recordings().unwrap();
    assert_eq!(recordings.len(), 2);
    let mut hash_prefixes = Vec::new();
    for (label, root, source_relative, archive, expected_name) in [
        (
            "ZOOM_RULETEST",
            &zoom_root,
            Path::new("RECORD/FOLDER01/ZOOM0001.WAV"),
            "Zoom H1n",
            "zoom-ZOOM0001-field.m4a",
        ),
        (
            "SONY_RULETEST",
            &sony_root,
            Path::new("REC_FILE/FOLDER01/SONY0001.WAV"),
            "Sony PCM",
            "sony-SONY0001-take.m4a",
        ),
    ] {
        let recording = recordings
            .iter()
            .find(|recording| recording.source_relative_path == source_relative)
            .unwrap_or_else(|| panic!("{label} recording evidence is missing"));
        assert_eq!(recording.artifact.format, OutputFormat::M4a);
        let audio = recording.artifact.audio.as_ref().unwrap();
        assert_eq!(audio.codec, "aac");
        assert_eq!(audio.sample_rate_hz, 48_000);
        assert_eq!(audio.channel_count, 1);
        assert!(recording.artifact.relative_path.starts_with(archive));
        assert_eq!(
            recording.artifact.relative_path.file_name().unwrap(),
            expected_name
        );

        let source_digest = hash_file(&root.join(source_relative)).unwrap();
        assert_eq!(recording.source_size, source_digest.size);
        assert_eq!(recording.source_sha256, source_digest.sha256);
        let superseded = ledger
            .superseded_wav_evidence(&recording.id)
            .unwrap()
            .unwrap();
        assert_eq!(superseded.1, source_digest.size);
        assert_eq!(superseded.2, source_digest.sha256);
        let artifact_digest =
            hash_file(&destination.path().join(&recording.artifact.relative_path)).unwrap();
        assert_eq!(artifact_digest.size, recording.artifact.byte_count);
        assert_eq!(artifact_digest.sha256, recording.artifact.sha256);

        let outcome = outcomes
            .iter()
            .find(|outcome| outcome.source_id == recording.source_id)
            .unwrap();
        let batch = ledger
            .batch_run_evidence(&outcome.batch_run_id)
            .unwrap()
            .unwrap();
        assert_eq!(batch.source_id.as_ref(), Some(&recording.source_id));
        assert_eq!(batch.phase, BatchPhase::M4aCohortVerified);
        hash_prefixes.push(&recording.source_sha256[..8]);
    }
    assert_ne!(hash_prefixes[0], hash_prefixes[1]);

    println!(
        "Rule fixture acceptance passed: sources=2 recordings=2 m4a=2 source_hash_prefixes={},{}",
        hash_prefixes[0], hash_prefixes[1]
    );
}

fn required_fixture(variable: &str, expected_label: &str) -> PathBuf {
    let requested = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{MISSING_FIXTURES}"));
    let root = fs::canonicalize(requested).unwrap_or_else(|_| panic!("{MISSING_FIXTURES}"));
    assert_eq!(root, Path::new("/Volumes").join(expected_label));
    assert_eq!(
        fs::read_to_string(root.join(SENTINEL)).unwrap_or_else(|_| panic!("{MISSING_FIXTURES}")),
        format!("backup-mic-rule-fixture:{expected_label}\n")
    );
    root
}

#[allow(clippy::too_many_arguments)]
fn rule(
    name: &str,
    archive: &str,
    volume_glob: &str,
    required_glob: &str,
    backup_glob: &str,
    session_glob: &str,
    prefix: &str,
    suffix: &str,
) -> BackupRuleDraft {
    BackupRuleDraft {
        id: None,
        name: name.to_owned(),
        archive_directory_name: archive.to_owned(),
        enabled: true,
        volume_name_glob: volume_glob.to_owned(),
        required_path_globs: vec![required_glob.to_owned()],
        backup_file_globs: vec![backup_glob.to_owned()],
        session_directory_globs: vec![session_glob.to_owned()],
        filename_prefix: prefix.to_owned(),
        filename_suffix: suffix.to_owned(),
    }
}
