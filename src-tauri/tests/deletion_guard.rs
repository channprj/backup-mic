#[test]
fn production_retirement_policy_has_no_permanent_unlink_or_trash_bypass() {
    let core = include_str!("../crates/backup-core/src/deletion.rs");
    let adapter = include_str!("../src/platform/macos/trash.rs");
    for forbidden in [
        "remove_file",
        "remove_dir",
        ".Trashes",
        "Command::",
        "AppleScript",
        "Finder",
    ] {
        assert!(
            !core.contains(forbidden),
            "core retirement policy contains forbidden disposal token: {forbidden}"
        );
        assert!(
            !adapter.contains(forbidden),
            "macOS Trash adapter contains forbidden disposal token: {forbidden}"
        );
    }
    assert!(adapter.contains("trashItemAtURL_resultingItemURL_error"));
}
