//! Coordinating one backup: device events, planning, copying, and source retirement.
//!
//! The stages are separate modules because they fail differently. Planning refuses before any
//! write, copying preserves the source on every error, and retirement needs a second live
//! verification. Keeping them apart makes it visible which guarantee a change touches.

mod copying;
mod monitor;
mod pipeline;
mod planning;
mod run;
mod shared;
mod trash;

pub use monitor::DeviceOrchestrator;
pub use pipeline::{
    CompletedRuleDeletionEvidence, NoSourceCopyFaults, SourceCopyFaults, SourceDeletionOutcome,
    SourceRunOutcome, overall_backup_phase, run_matched_sources_with_adapters,
};
pub use run::request_backup;
pub(crate) use run::{BackupTrigger, start_backup};
pub use trash::{
    confirm_rule_trash_with_adapter, confirm_trash, prepare_rule_trash, prepare_trash_for_source,
    retire_ready_rule_sources_with_adapter,
};
