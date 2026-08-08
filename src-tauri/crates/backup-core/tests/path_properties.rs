use std::path::{Path, PathBuf};

use backup_core::filesystem::is_safe_relative_path;
use proptest::prelude::*;

proptest! {
    #[test]
    fn parent_traversal_is_never_a_safe_relative_path(component in "[^/]{1,24}") {
        let path = PathBuf::from("..").join(component);
        prop_assert!(!is_safe_relative_path(&path));
    }

    #[test]
    fn absolute_paths_are_never_safe(component in "[a-zA-Z0-9_-]{1,24}") {
        let path = PathBuf::from("/").join(component);
        prop_assert!(!is_safe_relative_path(&path));
    }
}

#[test]
fn visible_normal_components_are_safe() {
    assert!(is_safe_relative_path(Path::new("session/recording.wav")));
}

#[test]
fn hidden_and_empty_paths_are_not_safe() {
    assert!(!is_safe_relative_path(Path::new(".Trashes/recording.wav")));
    assert!(!is_safe_relative_path(Path::new("")));
}
