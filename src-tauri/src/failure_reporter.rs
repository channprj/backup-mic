use std::path::{Path, PathBuf};

use backup_core::{
    audit_log::{AuditDurability, AuditEvent, AuditLevel, AuditSink, AuditValue, FileAuditLog},
    state::Transmitter,
};
use time::OffsetDateTime;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailureEvent {
    pub operation: &'static str,
    pub stage: &'static str,
    pub transmitter: Option<Transmitter>,
    pub item_name: Option<String>,
    pub error_code: String,
    pub os_kind: Option<String>,
    pub retryable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureWriteOutcome {
    Primary,
    Fallback,
    BothFailed,
}

#[derive(Clone, Debug)]
pub struct FailureReporter {
    fallback_root: PathBuf,
}

impl FailureReporter {
    pub fn new(fallback_root: impl AsRef<Path>) -> Self {
        Self {
            fallback_root: fallback_root.as_ref().to_path_buf(),
        }
    }

    pub fn report(
        &self,
        primary_destination: Option<&Path>,
        occurred_at: OffsetDateTime,
        event: &FailureEvent,
    ) -> FailureWriteOutcome {
        let os_kind = event.os_kind.as_deref().unwrap_or("none");
        let item_name = event.item_name.as_deref().unwrap_or("none");
        let fields = [
            ("operation", AuditValue::Text(event.operation)),
            ("stage", AuditValue::Text(event.stage)),
            ("error_code", AuditValue::Text(&event.error_code)),
            ("os_kind", AuditValue::Text(os_kind)),
            ("retryable", AuditValue::Boolean(event.retryable)),
            ("item", AuditValue::Text(item_name)),
        ];
        let audit_event = AuditEvent {
            occurred_at,
            level: AuditLevel::Error,
            code: "operation.failed",
            transmitter: event.transmitter,
            fields: &fields,
        };

        if let Some(primary_destination) = primary_destination
            && FileAuditLog::new(primary_destination)
                .append(&audit_event, AuditDurability::SyncData)
                .is_ok()
        {
            return FailureWriteOutcome::Primary;
        }

        if FileAuditLog::new_log_root(&self.fallback_root)
            .append(&audit_event, AuditDurability::SyncData)
            .is_ok()
        {
            FailureWriteOutcome::Fallback
        } else {
            FailureWriteOutcome::BothFailed
        }
    }
}
