use std::fs;

use backup_core::deletion::TrashAdapter;
use dji_mic_backup_lib::platform::macos::trash::MacTrash;
use tempfile::tempdir;

#[test]
fn foundation_adapter_moves_a_directory_as_one_recoverable_trash_item() {
    let parent = tempdir().unwrap();
    let session = parent.path().join("TX_MIC001_20260809_021747");
    fs::create_dir(&session).unwrap();
    fs::write(session.join("recording.wav"), b"recoverable fixture").unwrap();

    MacTrash.move_to_trash(&session).unwrap();

    assert!(!session.exists());
}
