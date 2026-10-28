//! The transcription view React renders, and the one options shape it may send back.
//!
//! Paths only ever travel as home-relative display strings, the same way the backup folder does,
//! and no path travels from React to Rust: folders and model files are picked in a Rust-owned
//! dialog.

use backup_core::{
    error::PublicError,
    transcription::{TranscriptFormats, WhisperOptions},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptionSnapshotDto {
    pub revision: u64,
    pub enabled: bool,
    pub include_existing_recordings: bool,
    pub output_directory_display: String,
    pub output_directory_is_default: bool,
    pub models_directory_display: String,
    pub models_directory_is_default: bool,
    pub selected_model: SelectedModelDto,
    pub models: Vec<WhisperModelDto>,
    pub download: Option<ModelDownloadDto>,
    pub formats: TranscriptFormats,
    pub whisper: WhisperOptions,
    pub error: Option<PublicError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSourceDto {
    Catalog,
    Custom,
}

/// The model transcription will load, and whether it can be loaded right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectedModelDto {
    pub source: ModelSourceDto,
    /// The catalog id; absent for a picked file.
    pub id: Option<String>,
    pub label: String,
    pub file_display: String,
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WhisperModelDto {
    pub id: String,
    pub label: String,
    pub byte_count: u64,
    pub english_only: bool,
    pub quantized: bool,
    pub installed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelDownloadDto {
    pub model_id: String,
    pub received_bytes: u64,
    pub total_bytes: u64,
}

/// Everything in the transcription settings that is not a path.
///
/// Strict on purpose: the stored row tolerates fields it does not know so a downgrade still
/// reads, but a request from the window that does not match this shape is a bug to refuse.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptionOptionsDto {
    pub include_existing_recordings: bool,
    pub formats: TranscriptFormats,
    pub whisper: WhisperOptions,
}

/// The two folders the transcription settings let the user choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionFolder {
    Output,
    Models,
}
