use std::{
    fs::Metadata,
    path::{Component, Path},
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
