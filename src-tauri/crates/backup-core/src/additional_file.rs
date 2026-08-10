use std::path::PathBuf;

use crate::source::SourceId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdditionalFileClass {
    M4a,
    AppleDouble,
    Other,
}

impl AdditionalFileClass {
    pub(crate) const fn storage_name(self) -> &'static str {
        match self {
            Self::M4a => "m4a",
            Self::AppleDouble => "apple_double",
            Self::Other => "other",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, crate::error::CoreError> {
        match value {
            "m4a" => Ok(Self::M4a),
            "apple_double" => Ok(Self::AppleDouble),
            "other" => Ok(Self::Other),
            _ => Err(crate::error::CoreError::LedgerCorrupt),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedAdditionalFile {
    pub id: String,
    pub source_id: SourceId,
    pub source_relative_path: PathBuf,
    pub source_size: u64,
    pub source_mtime_ns: i128,
    pub source_sha256: String,
    pub artifact_relative_path: PathBuf,
    pub artifact_size: u64,
    pub artifact_sha256: String,
    pub classification: AdditionalFileClass,
    pub backup_run_id: String,
}
