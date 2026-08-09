use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Wav,
    M4a,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedAudioProperties {
    pub codec: String,
    pub sample_rate_hz: u32,
    pub channel_count: u16,
    pub valid_frames: u64,
    pub duration_micros: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedArtifact {
    pub relative_path: PathBuf,
    pub format: OutputFormat,
    pub byte_count: u64,
    pub sha256: String,
    pub audio: Option<VerifiedAudioProperties>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionStatus {
    NotRequired,
    Pending,
    Complete,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetirementStatus {
    Present,
    TrashPending,
    MovedToTrash,
    LegacyDeleted,
    Failed,
}
