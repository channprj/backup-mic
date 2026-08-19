//! Recording what happened: the daily audit log, failure reports, and the activity feed.

use std::path::PathBuf;

use backup_core::audit_log::{AuditDurability, AuditEvent, AuditSink, FileAuditLog};
use backup_core::error::{CoreError, PublicError};
use backup_core::events::{ActivityEntry, ActivitySeverity};
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
        self.failure_reporter.report(
            primary_destination.as_deref(),
            crate::clock::local_now(),
            &event,
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
            ledger.recent_activity(8)?
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
        FileAuditLog::new(&runtime.destination)
            .path_for(occurred_at)
            .parent()
            .map(PathBuf::from)
            .ok_or(CoreError::InvalidRequest)
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

pub(super) fn sanitize_item_name(item_name: Option<&str>) -> Option<String> {
    item_name
        .and_then(|item_name| std::path::Path::new(item_name).file_name())
        .and_then(|item_name| item_name.to_str())
        .map(|item_name| item_name.chars().take(180).collect())
        .filter(|item_name: &String| !item_name.is_empty())
}
