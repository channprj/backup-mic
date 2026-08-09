use std::fs;

use backup_core::{scanner::metadata_fingerprint, state::Transmitter};
use tempfile::tempdir;
use time::UtcOffset;

#[test]
fn metadata_fingerprint_changes_for_added_removed_or_resized_wavs() {
    let source = tempdir().unwrap();
    let first_path = source.path().join("TX02_MIC001_20260809_010203.wav");
    fs::write(&first_path, b"first").unwrap();
    let baseline = metadata_fingerprint(source.path(), Transmitter::Tx02, UtcOffset::UTC).unwrap();
    assert_eq!(
        baseline,
        metadata_fingerprint(source.path(), Transmitter::Tx02, UtcOffset::UTC).unwrap()
    );

    let second_path = source.path().join("TX02_MIC002_20260809_010204.wav");
    fs::write(&second_path, b"second").unwrap();
    let added = metadata_fingerprint(source.path(), Transmitter::Tx02, UtcOffset::UTC).unwrap();
    assert_ne!(added, baseline);

    fs::write(&first_path, b"first recording grew").unwrap();
    let resized = metadata_fingerprint(source.path(), Transmitter::Tx02, UtcOffset::UTC).unwrap();
    assert_ne!(resized, added);

    fs::remove_file(second_path).unwrap();
    let removed = metadata_fingerprint(source.path(), Transmitter::Tx02, UtcOffset::UTC).unwrap();
    assert_ne!(removed, resized);
}

#[cfg(unix)]
#[test]
fn metadata_fingerprint_ignores_hidden_and_symlink_entries() {
    use std::os::unix::fs::symlink;

    let source = tempdir().unwrap();
    let outside = tempdir().unwrap();
    fs::create_dir(source.path().join(".Trashes")).unwrap();
    fs::write(
        source
            .path()
            .join(".Trashes/TX01_MIC001_20260809_010203.wav"),
        b"trash",
    )
    .unwrap();
    let outside_path = outside.path().join("TX01_MIC002_20260809_010204.wav");
    fs::write(&outside_path, b"outside").unwrap();
    symlink(&outside_path, source.path().join("linked.wav")).unwrap();

    let fingerprint =
        metadata_fingerprint(source.path(), Transmitter::Tx01, UtcOffset::UTC).unwrap();

    assert!(fingerprint.is_empty());
}
