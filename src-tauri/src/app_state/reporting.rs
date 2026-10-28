//! Recording what happened: the daily audit log, failure reports, and the activity feed.

use std::path::PathBuf;

use backup_core::audit_log::{AuditDurability, AuditEvent, AuditSink, AuditValue, FileAuditLog};
use backup_core::error::{CoreError, PublicError};
use backup_core::events::{ActivityEntry, ActivitySeverity};
use backup_core::ledger::MAX_ACTIVITY_ENTRIES;
use backup_core::source::SourceId;
use backup_core::state::Transmitter;
use tauri::AppHandle;

use crate::failure_reporter::{FailureEvent, FailureWriteOutcome};

use super::{AppState, publish_locked};

impl AppState {
    pub fn append_audit(
        &self,
        event: &AuditEvent<'_>,
        durability: AuditDurability,
    ) -> Result<PathBuf, CoreError> {
        let destination = {
            let runtime = self.runtime.lock();
            if !runtime.destination_configured {
                return Err(CoreError::InvalidRequest);
            }
            runtime.destination.clone()
        };
        FileAuditLog::new(destination).append(event, durability)
    }

    pub fn report_failure(
        &self,
        operation: &'static str,
        stage: &'static str,
        error: &CoreError,
        transmitter: Option<Transmitter>,
        item_name: Option<&str>,
    ) -> FailureWriteOutcome {
        self.report_failure_with_context(operation, stage, error, transmitter, item_name, &[])
    }

    /// Reports a failure together with where it happened and the raw errno behind it.
    pub fn report_failure_with_context(
        &self,
        operation: &'static str,
        stage: &'static str,
        error: &CoreError,
        transmitter: Option<Transmitter>,
        item_name: Option<&str>,
        context: &[(&'static str, AuditValue<'_>)],
    ) -> FailureWriteOutcome {
        let primary_destination = {
            let runtime = self.runtime.lock();
            runtime
                .destination_configured
                .then(|| runtime.destination.clone())
        };
        let public = error.public(transmitter);
        let event = FailureEvent {
            operation,
            stage,
            transmitter,
            item_name: sanitize_item_name(item_name),
            error_code: error.diagnostic_code().to_owned(),
            os_kind: error.diagnostic_io_kind_code().map(str::to_owned),
            retryable: public.retryable,
        };
        let mut context = context.to_vec();
        if let Some(code) = error
            .diagnostic_os_code()
            .and_then(|code| u64::try_from(code).ok())
        {
            context.push(("os_code", AuditValue::Unsigned(code)));
        }
        self.failure_reporter.report_with_context(
            primary_destination.as_deref(),
            crate::clock::local_now(),
            &event,
            &context,
        )
    }

    pub fn report_public_failure(
        &self,
        operation: &'static str,
        stage: &'static str,
        error: &PublicError,
        item_name: Option<&str>,
    ) -> FailureWriteOutcome {
        let primary_destination = {
            let runtime = self.runtime.lock();
            runtime
                .destination_configured
                .then(|| runtime.destination.clone())
        };
        let event = FailureEvent {
            operation,
            stage,
            transmitter: error.transmitter,
            item_name: sanitize_item_name(item_name),
            error_code: error.message_code.clone(),
            os_kind: None,
            retryable: error.retryable,
        };
        self.failure_reporter.report(
            primary_destination.as_deref(),
            crate::clock::local_now(),
            &event,
        )
    }

    pub fn record_activity(&self, app: &AppHandle, entry: ActivityEntry) -> Result<(), CoreError> {
        let recent_activity = {
            let mut ledger = self.ledger.lock();
            ledger.append_activity(&entry)?;
            ledger.recent_activity(MAX_ACTIVITY_ENTRIES)?
        };
        let mut runtime = self.runtime.lock();
        runtime.snapshot.recent_activity = runtime
            .snapshot
            .clone()
            .with_activity(recent_activity)
            .recent_activity;
        publish_locked(app, &mut runtime);
        Ok(())
    }

    pub fn log_directory(&self, occurred_at: time::OffsetDateTime) -> Result<PathBuf, CoreError> {
        let runtime = self.runtime.lock();
        if !runtime.destination_configured {
            return Err(CoreError::InvalidRequest);
        }
        FileAuditLog::new(&runtime.destination).ensure_directory(occurred_at)
    }
}

pub(super) fn activity_entry(
    code: &str,
    source_id: SourceId,
    severity: ActivitySeverity,
) -> ActivityEntry {
    ActivityEntry {
        occurred_at: time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned()),
        code: code.to_owned(),
        source_id: Some(source_id),
        source_label: None,
        count_value: None,
        byte_value: None,
        severity,
    }
}

/// A volume label the log accepts; one that looks like an identity is replaced.
pub(crate) fn loggable_label(label: &str) -> &str {
    if !label.is_empty() && backup_core::audit_log::text_is_loggable(label) {
        label
    } else {
        "unnamed"
    }
}

pub(crate) fn sanitize_item_name(item_name: Option<&str>) -> Option<String> {
    item_name
        .and_then(|item_name| std::path::Path::new(item_name).file_name())
        .and_then(|item_name| item_name.to_str())
        .map(|item_name| item_name.chars().take(180).collect())
        .filter(|item_name: &String| !item_name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::loggable_label;

    #[test]
    fn labels_that_look_like_identities_are_not_logged() {
        assert_eq!(loggable_label("DJI-MIC-2"), "DJI-MIC-2");
        assert_eq!(
            loggable_label("071FEE39-3941-33CD-AC05-532A6A258E2C"),
            "unnamed"
        );
        assert_eq!(loggable_label("/Volumes/DJI-MIC-2"), "unnamed");
        assert_eq!(loggable_label(""), "unnamed");
    }
}
