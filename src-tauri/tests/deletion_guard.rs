use std::{fs, path::PathBuf};

/// Every source file of the production retirement policy, read at test time.
///
/// The directory is walked rather than the files being named, because the point of this guard is
/// that nothing in the retirement policy escapes it — including a module added after the guard
/// was written.
fn retirement_policy_sources() -> Vec<(PathBuf, String)> {
    let policy = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("crates/backup-core/src/deletion");
    let mut sources = fs::read_dir(&policy)
        .expect("the retirement policy directory exists")
        .map(|entry| {
            entry
                .expect("the retirement policy directory is readable")
                .path()
        })
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .map(|path| {
            let text = fs::read_to_string(&path).expect("a retirement policy file is readable");
            (path, text)
        })
        .collect::<Vec<_>>();
    sources.sort();
    assert!(
        sources.len() >= 6,
        "the retirement policy lost files; the scan may no longer cover it: {sources:?}",
    );
    sources
}

#[test]
fn production_retirement_policy_has_no_permanent_unlink_or_trash_bypass() {
    let adapter = include_str!("../src/platform/macos/trash.rs");
    let policy = retirement_policy_sources();
    for forbidden in [
        "remove_file",
        "remove_dir",
        ".Trashes",
        "Command::",
        "AppleScript",
        "Finder",
    ] {
        for (path, source) in &policy {
            assert!(
                !source.contains(forbidden),
                "{} contains forbidden disposal token: {forbidden}",
                path.display(),
            );
        }
        assert!(
            !adapter.contains(forbidden),
            "macOS Trash adapter contains forbidden disposal token: {forbidden}"
        );
    }
    assert!(adapter.contains("trashItemAtURL_resultingItemURL_error"));
}
