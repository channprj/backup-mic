use std::path::Path;

use backup_core::{deletion::TrashAdapter, error::CoreError};
use objc2_foundation::{NSFileManager, NSURL};

#[derive(Debug, Default, Clone, Copy)]
pub struct MacTrash;

impl TrashAdapter for MacTrash {
    fn move_to_trash(&self, absolute_path: &Path) -> Result<(), CoreError> {
        if !absolute_path.is_absolute() {
            return Err(CoreError::InvalidRequest);
        }
        let metadata =
            std::fs::symlink_metadata(absolute_path).map_err(|_| CoreError::TrashFailed)?;
        if metadata.file_type().is_symlink()
            || (!metadata.file_type().is_file() && !metadata.file_type().is_dir())
        {
            return Err(CoreError::TrashFailed);
        }
        let url = NSURL::from_file_path(absolute_path).ok_or(CoreError::TrashFailed)?;
        let manager = NSFileManager::defaultManager();
        manager
            .trashItemAtURL_resultingItemURL_error(&url, None)
            .map_err(|_| CoreError::TrashFailed)
    }
}
