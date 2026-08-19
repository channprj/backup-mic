use std::{
    fs,
    time::{Duration, Instant},
};

use backup_core::{
    rule::{BackupRule, DeviceConstraintProfile, FilenameProfile, RuleId, compile_rule},
    rule_scanner::scan_rule_once,
    source::SourceId,
};
use backup_mic_lib::rescan::{RescanDecision, RescanScheduler};
use tempfile::tempdir;
use time::UtcOffset;

fn rule() -> backup_core::rule::CompiledBackupRule {
    compile_rule(BackupRule {
        id: RuleId::new(),
        name: "Recorder".to_owned(),
        archive_directory_name: "Recorder".to_owned(),
        enabled: true,
        volume_name_glob: "*".to_owned(),
        required_path_globs: Vec::new(),
        backup_file_globs: vec!["*.wav".to_owned()],
        session_directory_globs: Vec::new(),
        filename_prefix: String::new(),
        filename_suffix: String::new(),
        date_folder_layout: Default::default(),
        filename_profile: FilenameProfile::Preserve,
        device_constraint_profile: DeviceConstraintProfile::GenericExternal,
        preset_kind: None,
        preset_revision: None,
        archive_directory_locked: false,
        archived_at: None,
        created_at: "2026-08-10T00:00:00Z".to_owned(),
        updated_at: "2026-08-10T00:00:00Z".to_owned(),
    })
    .unwrap()
}

fn fingerprint(root: &std::path::Path) -> backup_core::rule_scanner::RuleScanFingerprint {
    scan_rule_once(root, &rule(), UtcOffset::UTC)
        .unwrap()
        .fingerprint
}

#[test]
fn unchanged_deadlines_schedule_no_work_but_new_metadata_coalesces_one_pending_run() {
    let source = tempdir().unwrap();
    fs::write(
        source.path().join("TX01_MIC001_20260809_010203.wav"),
        b"first",
    )
    .unwrap();
    let initial = fingerprint(source.path());
    let source_id = SourceId::new();
    let started = Instant::now();
    let mut scheduler = RescanScheduler::new(Duration::from_secs(15));
    scheduler.mount(source_id.clone(), initial.clone(), started);

    assert!(
        scheduler
            .due_sources(started + Duration::from_secs(14))
            .is_empty()
    );
    assert_eq!(
        scheduler.due_sources(started + Duration::from_secs(15)),
        vec![source_id.clone()]
    );
    assert_eq!(
        scheduler.observe(
            source_id.clone(),
            initial,
            started + Duration::from_secs(15),
            false,
        ),
        RescanDecision::Unchanged
    );

    fs::write(
        source.path().join("TX01_MIC002_20260809_010204.wav"),
        b"second",
    )
    .unwrap();
    let added = fingerprint(source.path());
    assert_eq!(
        scheduler.observe(
            source_id.clone(),
            added,
            started + Duration::from_secs(30),
            false,
        ),
        RescanDecision::RequestBackup
    );
    assert!(scheduler.has_pending_backup());

    fs::write(
        source.path().join("TX01_MIC003_20260809_010205.wav"),
        b"third",
    )
    .unwrap();
    let changed_again = fingerprint(source.path());
    assert_eq!(
        scheduler.observe(
            source_id,
            changed_again,
            started + Duration::from_secs(45),
            true,
        ),
        RescanDecision::KeepPending
    );
    scheduler.mark_backup_started();
    assert!(!scheduler.has_pending_backup());
}

#[test]
fn unmount_clears_deadline_fingerprint_and_pending_authority() {
    let source = tempdir().unwrap();
    let initial = fingerprint(source.path());
    let first = SourceId::new();
    let second = SourceId::new();
    let started = Instant::now();
    let mut scheduler = RescanScheduler::new(Duration::from_secs(15));
    scheduler.mount(first.clone(), initial.clone(), started);
    scheduler.mount(second.clone(), initial, started);
    fs::write(
        source.path().join("TX02_MIC001_20260809_010203.wav"),
        b"new",
    )
    .unwrap();
    assert_eq!(
        scheduler.observe(
            first.clone(),
            fingerprint(source.path()),
            started + Duration::from_secs(15),
            false,
        ),
        RescanDecision::RequestBackup
    );

    scheduler.unmount(&first);

    assert!(!scheduler.pending_sources().contains(&first));
    assert_eq!(
        scheduler.due_sources(started + Duration::from_secs(60)),
        vec![second]
    );
}

#[test]
fn a_new_interval_rebases_pending_deadlines_in_both_directions() {
    let source = tempdir().unwrap();
    fs::write(
        source.path().join("TX01_MIC001_20260809_010203.wav"),
        b"first",
    )
    .unwrap();
    let source_id = SourceId::new();
    let started = Instant::now();
    let mut scheduler = RescanScheduler::new(Duration::from_secs(300));
    scheduler.mount(source_id.clone(), fingerprint(source.path()), started);

    // Shortening must not wait out the interval the source was mounted under.
    scheduler.set_interval(Duration::from_secs(15), started);
    assert!(
        scheduler
            .due_sources(started + Duration::from_secs(14))
            .is_empty()
    );
    assert_eq!(
        scheduler.due_sources(started + Duration::from_secs(15)),
        vec![source_id.clone()]
    );

    // Lengthening from a point where the source is already due must push it out, not fire again.
    let due_at = started + Duration::from_secs(15);
    scheduler.set_interval(Duration::from_secs(600), due_at);
    assert!(
        scheduler
            .due_sources(due_at + Duration::from_secs(599))
            .is_empty()
    );
    assert_eq!(
        scheduler.due_sources(due_at + Duration::from_secs(600)),
        vec![source_id]
    );
}

#[test]
fn an_unchanged_interval_leaves_the_existing_deadline_alone() {
    let source = tempdir().unwrap();
    fs::write(
        source.path().join("TX01_MIC001_20260809_010203.wav"),
        b"first",
    )
    .unwrap();
    let source_id = SourceId::new();
    let started = Instant::now();
    let mut scheduler = RescanScheduler::new(Duration::from_secs(15));
    scheduler.mount(source_id.clone(), fingerprint(source.path()), started);

    // Called on every monitor tick, so re-applying the same value must not keep a source from
    // ever becoming due.
    for tick in 0..14 {
        scheduler.set_interval(Duration::from_secs(15), started + Duration::from_secs(tick));
    }
    assert_eq!(
        scheduler.due_sources(started + Duration::from_secs(15)),
        vec![source_id]
    );
}
