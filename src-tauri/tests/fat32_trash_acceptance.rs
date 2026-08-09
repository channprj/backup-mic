use std::{fs, path::PathBuf};

use backup_core::deletion::TrashAdapter;
use dji_mic_backup_lib::platform::macos::trash::MacTrash;

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
    fs::write(
        session.join("TX01_MIC001_20260809_021747.wav"),
        b"recoverable FAT32 Trash fixture",
    )
    .unwrap();

    MacTrash.move_to_trash(&session).unwrap();

    assert!(!session.exists());
}
