//! The SQLite ledger: the app's durable record of what was backed up and verified.
//!
//! One `Ledger` owns one connection. Its inherent methods live in submodules named for the tables
//! they speak for, so finding rule storage does not mean scrolling past deletion evidence.

mod activity;
mod additional_files;
mod devices;
mod recordings;
mod retirement;
mod rules;
mod runs;
mod schema;
mod settings;
mod sql;

use std::path::PathBuf;

use rusqlite::Connection;

use crate::{
    artifact::{ConversionStatus, RetirementStatus, VerifiedArtifact},
    source::SourceId,
};

pub const MAX_ACTIVITY_ENTRIES: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRecording {
    pub id: String,
    pub source_id: SourceId,
    pub source_relative_path: PathBuf,
    pub source_size: u64,
    pub source_mtime_ns: i128,
    pub source_sha256: String,
    pub artifact: VerifiedArtifact,
    pub conversion_status: ConversionStatus,
    pub conversion_error_code: Option<String>,
    pub retirement_status: RetirementStatus,
    pub retired_session_relative_path: Option<PathBuf>,
    pub verified_at: String,
    pub backup_run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupersededWavEvidence {
    pub recording_id: String,
    pub source_id: SourceId,
    pub relative_path: PathBuf,
    pub byte_count: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDeletionItem {
    pub recording_id: String,
    pub source_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAdditionalDeletionItem {
    pub additional_file_id: String,
    pub source_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyRetiredRecording {
    pub recording_id: String,
    pub source_relative_path: PathBuf,
}

pub struct Ledger {
    connection: Connection,
    path: PathBuf,
    deletion_disabled: bool,
}
