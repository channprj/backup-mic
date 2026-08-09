use std::{
    fs,
    time::{Duration, Instant},
};

use backup_core::{scanner::metadata_fingerprint, state::Transmitter};
use dji_mic_backup_lib::rescan::{RescanDecision, RescanScheduler};
use tempfile::tempdir;
use time::UtcOffset;

#[test]
fn unchanged_deadlines_schedule_no_work_but_new_metadata_coalesces_one_pending_run() {
    let source = tempdir().unwrap();
    fs::write(
        source.path().join("TX01_MIC001_20260809_010203.wav"),
        b"first",
    )
    .unwrap();
    let initial = metadata_fingerprint(source.path(), Transmitter::Tx01, UtcOffset::UTC).unwrap();
    let started = Instant::now();
    let mut scheduler = RescanScheduler::new(Duration::from_secs(15));
    scheduler.mount(Transmitter::Tx01, initial.clone(), started);

    assert!(
        scheduler
            .due_transmitters(started + Duration::from_secs(14))
            .is_empty()
    );
    assert_eq!(
        scheduler.due_transmitters(started + Duration::from_secs(15)),
        vec![Transmitter::Tx01]
    );
    assert_eq!(
        scheduler.observe(
            Transmitter::Tx01,
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
    let added = metadata_fingerprint(source.path(), Transmitter::Tx01, UtcOffset::UTC).unwrap();
    assert_eq!(
        scheduler.observe(
            Transmitter::Tx01,
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
    let changed_again =
        metadata_fingerprint(source.path(), Transmitter::Tx01, UtcOffset::UTC).unwrap();
    assert_eq!(
        scheduler.observe(
            Transmitter::Tx01,
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
    let fingerprint =
        metadata_fingerprint(source.path(), Transmitter::Tx02, UtcOffset::UTC).unwrap();
    let started = Instant::now();
    let mut scheduler = RescanScheduler::new(Duration::from_secs(15));
    scheduler.mount(Transmitter::Tx02, fingerprint.clone(), started);
    fs::write(
        source.path().join("TX02_MIC001_20260809_010203.wav"),
        b"new",
    )
    .unwrap();
    assert_eq!(
        scheduler.observe(
            Transmitter::Tx02,
            metadata_fingerprint(source.path(), Transmitter::Tx02, UtcOffset::UTC).unwrap(),
            started + Duration::from_secs(15),
            false,
        ),
        RescanDecision::RequestBackup
    );

    scheduler.unmount(Transmitter::Tx02);

    assert!(!scheduler.has_pending_backup());
    assert!(
        scheduler
            .due_transmitters(started + Duration::from_secs(60))
            .is_empty()
    );
}
