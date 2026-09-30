use std::fs;

use backup_core::deletion::TrashAdapter;
use backup_mic_lib::platform::macos::trash::MacTrash;
use tempfile::tempdir;

#[test]
fn foundation_adapter_moves_a_directory_as_one_recoverable_trash_item() {
    let parent = tempdir().unwrap();
    let session = parent.path().join("TX_MIC001_20260809_021747");
    fs::create_dir(&session).unwrap();
    fs::write(session.join("recording.wav"), b"recoverable fixture").unwrap();

    MacTrash
        .move_to_trash(&fs::canonicalize(&session).unwrap())
        .unwrap();

    assert!(!session.exists());
}

#[test]
fn foundation_adapter_refuses_a_symlinked_parent() {
    use std::os::unix::fs::symlink;

    let fixture = tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    let outside = root.join("outside");
    let session = outside.join("session");
    fs::create_dir_all(&session).unwrap();
    fs::write(session.join("keep.wav"), b"must remain in place").unwrap();
    let redirected = root.join("redirected");
    symlink(&outside, &redirected).unwrap();

    assert!(MacTrash.move_to_trash(&redirected.join("session")).is_err());
    assert_eq!(
        fs::read(session.join("keep.wav")).unwrap(),
        b"must remain in place"
    );
}
