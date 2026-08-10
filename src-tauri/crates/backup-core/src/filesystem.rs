use std::{
    fs::{self, Metadata},
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

use crate::error::CoreError;

pub fn modified_nanos(metadata: &Metadata) -> Result<i128, CoreError> {
    let modified = metadata.modified().map_err(CoreError::CopyFailed)?;
    let duration = modified.duration_since(UNIX_EPOCH).map_err(|_| {
        CoreError::CopyFailed(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "modification time predates unix epoch",
        ))
    })?;
    Ok(i128::from(duration.as_secs()) * 1_000_000_000 + i128::from(duration.subsec_nanos()))
}

pub fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

pub fn is_safe_relative_path(path: &Path) -> bool {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return false;
    }
    path.components().all(|component| match component {
        Component::Normal(value) => value
            .to_str()
            .is_some_and(|value| !value.is_empty() && !value.starts_with('.')),
        Component::CurDir | Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
            false
        }
    })
}

pub fn is_safe_additional_relative_path(path: &Path) -> bool {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return false;
    }
    let components = path.components().collect::<Vec<_>>();
    !components.is_empty()
        && components.iter().enumerate().all(|(index, component)| {
            let Component::Normal(value) = component else {
                return false;
            };
            value.to_str().is_some_and(|value| {
                !value.is_empty() && (index + 1 == components.len() || !value.starts_with('.'))
            })
        })
}

pub fn is_recognized_session_name(path: &Path) -> bool {
    let Some(name) = path.to_str() else {
        return false;
    };
    let mut parts = name.split('_');
    let (Some(tx), Some(mic), Some(date), Some(time), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return false;
    };
    tx == "TX"
        && mic.strip_prefix("MIC").is_some_and(|digits| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
        && date.len() == 8
        && date.bytes().all(|byte| byte.is_ascii_digit())
        && time.len() == 6
        && time.bytes().all(|byte| byte.is_ascii_digit())
}

pub fn canonical_regular_file(root: &Path, relative: &Path) -> Result<PathBuf, CoreError> {
    if !is_safe_additional_relative_path(relative) {
        return Err(CoreError::InvalidRequest);
    }
    let root_metadata = fs::symlink_metadata(root).map_err(CoreError::CopyFailed)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(CoreError::InvalidRequest);
    }
    let canonical_root = fs::canonicalize(root).map_err(CoreError::CopyFailed)?;
    let candidate = root.join(relative);
    let metadata = fs::symlink_metadata(&candidate).map_err(CoreError::CopyFailed)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CoreError::InvalidRequest);
    }
    let canonical = fs::canonicalize(candidate).map_err(CoreError::CopyFailed)?;
    if !canonical.starts_with(&canonical_root) {
        return Err(CoreError::InvalidRequest);
    }
    Ok(canonical)
}
