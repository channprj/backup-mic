use std::{collections::BTreeSet, path::PathBuf};

use backup_core::{
    batch::{BatchItemKey, CopyBarrier},
    error::CoreError,
    source::{LEGACY_TX01_SOURCE_ID, LEGACY_TX02_SOURCE_ID, SourceId},
};

fn source_id(slot: &str) -> SourceId {
    SourceId::parse(match slot {
        "TX01" => LEGACY_TX01_SOURCE_ID,
        "TX02" => LEGACY_TX02_SOURCE_ID,
        _ => unreachable!(),
    })
    .unwrap()
}

fn recording(slot: &str, path: &str) -> BatchItemKey {
    BatchItemKey::Recording {
        source_id: source_id(slot),
        relative_path: PathBuf::from(path),
    }
}

fn additional(slot: &str, path: &str) -> BatchItemKey {
    BatchItemKey::Additional {
        source_id: source_id(slot),
        relative_path: PathBuf::from(path),
    }
}

#[test]
fn conversion_requires_every_expected_copy_and_verification() {
    let first = recording("TX01", "session/first.wav");
    let second = additional("TX01", "session/._first.m4a");
    let third = recording("TX02", "second.wav");
    let mut barrier = CopyBarrier::new(BTreeSet::from([
        first.clone(),
        second.clone(),
        third.clone(),
    ]))
    .unwrap();

    assert!(!barrier.conversion_allowed());
    barrier.record_verified(&first).unwrap();
    barrier.record_verified(&second).unwrap();
    assert!(!barrier.conversion_allowed());
    barrier.record_verified(&third).unwrap();
    assert!(barrier.conversion_allowed());
}

#[test]
fn a_failed_unknown_or_duplicate_item_can_never_open_the_barrier() {
    let first = recording("TX01", "first.wav");
    let second = recording("TX02", "second.wav");
    let mut barrier = CopyBarrier::new(BTreeSet::from([first.clone(), second.clone()])).unwrap();

    barrier.record_verified(&first).unwrap();
    assert!(matches!(
        barrier.record_verified(&first),
        Err(CoreError::InvalidRequest)
    ));
    assert!(matches!(
        barrier.record_failed(&additional("TX01", "unknown.m4a")),
        Err(CoreError::InvalidRequest)
    ));
    barrier.record_failed(&second).unwrap();
    assert!(!barrier.conversion_allowed());
    assert!(matches!(
        barrier.record_verified(&second),
        Err(CoreError::InvalidRequest)
    ));
}

#[test]
fn an_empty_batch_is_valid_and_needs_no_conversion_work() {
    let barrier = CopyBarrier::new(BTreeSet::new()).unwrap();
    assert!(barrier.conversion_allowed());
}
