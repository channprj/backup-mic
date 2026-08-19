//! Small helpers every stage needs: resolving a live source and shaping safe errors.

use backup_core::error::{CoreError, PublicError, PublicErrorCode};
use backup_core::source::SourceId;
use backup_core::state::Transmitter;

use crate::app_state::AppState;
use crate::rule_runtime::MatchedSource;

use super::pipeline::SourceRunOutcome;

pub(super) fn resolve_live_source(
    root: &std::path::Path,
    relative: &std::path::Path,
) -> Result<std::path::PathBuf, CoreError> {
    let canonical_root = std::fs::canonicalize(root).map_err(CoreError::CopyFailed)?;
    let path = canonical_root.join(relative);
    let metadata = std::fs::symlink_metadata(&path).map_err(CoreError::CopyFailed)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(CoreError::SourceChanged);
    }
    let canonical = std::fs::canonicalize(path).map_err(CoreError::CopyFailed)?;
    if !canonical.starts_with(canonical_root) {
        return Err(CoreError::SourceChanged);
    }
    Ok(canonical)
}

pub(super) fn matched_legacy_transmitter(matched: &MatchedSource) -> Option<Transmitter> {
    match matched.authority.source.legacy_slot.as_deref() {
        Some("TX01") => Some(Transmitter::Tx01),
        Some("TX02") => Some(Transmitter::Tx02),
        _ => None,
    }
}

pub(super) fn adapter_public_error(message_code: &str, retryable: bool) -> PublicError {
    PublicError {
        code: PublicErrorCode::Internal,
        message_code: message_code.to_owned(),
        retryable,
        transmitter: None,
        source_id: None,
        source_label: None,
    }
}

pub(super) fn source_label(state: &AppState, source_id: &SourceId) -> Option<String> {
    state
        .runtime
        .lock()
        .matched
        .get(source_id)
        .map(|matched| matched.authority.source.display_name.clone())
}

pub(super) fn source_failure_stage(outcome: &SourceRunOutcome) -> &'static str {
    if outcome.batch_run_id.is_empty() {
        "source_preparation"
    } else {
        "source_pipeline"
    }
}
