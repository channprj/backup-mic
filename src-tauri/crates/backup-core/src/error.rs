use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::state::Transmitter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicErrorCode {
    IdentityMismatch,
    SourceChanged,
    DestinationUnavailable,
    InsufficientCapacity,
    CopyFailed,
    SyncFailed,
    HashMismatch,
    ArtifactInvalid,
    AudioToolFailed,
    TrashFailed,
    DeviceRemoved,
    LedgerCorrupt,
    AuditLogUnavailable,
    ProposalExpired,
    ProposalInvalidated,
    DeletionPreflightRefused,
    #[serde(rename = "partial_trash")]
    PartialDeletion,
    Busy,
    InvalidRequest,
    Cancelled,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicError {
    pub code: PublicErrorCode,
    pub message_code: String,
    pub retryable: bool,
    pub transmitter: Option<Transmitter>,
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("device identity mismatch")]
    IdentityMismatch,
    #[error("source changed during backup")]
    SourceChanged,
    #[error("destination unavailable")]
    DestinationUnavailable,
    #[error("insufficient destination capacity")]
    InsufficientCapacity,
    #[error("copy operation failed")]
    CopyFailed(#[source] std::io::Error),
    #[error("durability synchronization failed")]
    SyncFailed(#[source] std::io::Error),
    #[error("source and destination hashes differ")]
    HashMismatch,
    #[error("converted audio artifact failed validation")]
    ArtifactInvalid,
    #[error("Apple audio tool failed")]
    AudioToolFailed,
    #[error("macOS Trash movement failed")]
    TrashFailed,
    #[error("device was removed")]
    DeviceRemoved,
    #[error("ledger is corrupt")]
    LedgerCorrupt,
    #[error("ledger operation failed")]
    Ledger(#[source] rusqlite::Error),
    #[error("ledger filesystem operation failed")]
    LedgerIo(#[source] std::io::Error),
    #[error("daily audit log is unavailable")]
    AuditLogUnavailable(#[source] std::io::Error),
    #[error("daily audit event contains unsafe data")]
    InvalidAuditEvent,
    #[error("deletion proposal expired")]
    ProposalExpired,
    #[error("deletion proposal was invalidated")]
    ProposalInvalidated,
    #[error("deletion preflight was refused")]
    DeletionPreflightRefused,
    #[error("another operation is active")]
    Busy,
    #[error("invalid request")]
    InvalidRequest,
    #[error("operation was cancelled")]
    Cancelled,
}

impl CoreError {
    pub fn diagnostic_code(&self) -> &'static str {
        match self {
            Self::IdentityMismatch => "identity_mismatch",
            Self::SourceChanged => "source_changed",
            Self::DestinationUnavailable => "destination_unavailable",
            Self::InsufficientCapacity => "insufficient_capacity",
            Self::CopyFailed(_) => "copy_failed",
            Self::SyncFailed(_) => "sync_failed",
            Self::HashMismatch => "hash_mismatch",
            Self::ArtifactInvalid => "artifact_validation_failed",
            Self::AudioToolFailed => "audio_conversion_failed",
            Self::TrashFailed => "trash_move_failed",
            Self::DeviceRemoved => "device_removed",
            Self::LedgerCorrupt => "ledger_corrupt",
            Self::Ledger(_) | Self::LedgerIo(_) => "ledger_operation_failed",
            Self::AuditLogUnavailable(_) => "audit_log_unavailable",
            Self::InvalidAuditEvent => "audit_event_invalid",
            Self::ProposalExpired => "proposal_expired",
            Self::ProposalInvalidated => "proposal_invalidated",
            Self::DeletionPreflightRefused => "deletion_preflight_refused",
            Self::Busy => "operation_busy",
            Self::InvalidRequest => "invalid_request",
            Self::Cancelled => "operation_cancelled",
        }
    }

    pub fn diagnostic_io_kind(&self) -> Option<std::io::ErrorKind> {
        match self {
            Self::CopyFailed(error)
            | Self::SyncFailed(error)
            | Self::LedgerIo(error)
            | Self::AuditLogUnavailable(error) => Some(error.kind()),
            _ => None,
        }
    }

    pub fn diagnostic_io_kind_code(&self) -> Option<&'static str> {
        self.diagnostic_io_kind().map(io_kind_code)
    }

    pub fn public(&self, transmitter: Option<Transmitter>) -> PublicError {
        let (code, message_code, retryable) = match self {
            Self::IdentityMismatch => (
                PublicErrorCode::IdentityMismatch,
                "identity_mismatch",
                false,
            ),
            Self::SourceChanged => (PublicErrorCode::SourceChanged, "source_changed", true),
            Self::DestinationUnavailable => (
                PublicErrorCode::DestinationUnavailable,
                "destination_unavailable",
                true,
            ),
            Self::InsufficientCapacity => (
                PublicErrorCode::InsufficientCapacity,
                "insufficient_capacity",
                true,
            ),
            Self::CopyFailed(_) => (PublicErrorCode::CopyFailed, "copy_failed", true),
            Self::SyncFailed(_) => (PublicErrorCode::SyncFailed, "sync_failed", true),
            Self::HashMismatch => (PublicErrorCode::HashMismatch, "hash_mismatch", true),
            Self::ArtifactInvalid => (
                PublicErrorCode::ArtifactInvalid,
                "artifact_validation_failed",
                true,
            ),
            Self::AudioToolFailed => (
                PublicErrorCode::AudioToolFailed,
                "audio_conversion_failed",
                true,
            ),
            Self::TrashFailed => (PublicErrorCode::TrashFailed, "trash_move_failed", true),
            Self::DeviceRemoved => (PublicErrorCode::DeviceRemoved, "device_removed", true),
            Self::LedgerCorrupt => (PublicErrorCode::LedgerCorrupt, "ledger_corrupt", false),
            Self::Ledger(_) | Self::LedgerIo(_) => {
                (PublicErrorCode::Internal, "ledger_operation_failed", true)
            }
            Self::AuditLogUnavailable(_) => (
                PublicErrorCode::AuditLogUnavailable,
                "audit_log_unavailable",
                true,
            ),
            Self::InvalidAuditEvent => (PublicErrorCode::Internal, "audit_event_invalid", false),
            Self::ProposalExpired => (PublicErrorCode::ProposalExpired, "proposal_expired", true),
            Self::ProposalInvalidated => (
                PublicErrorCode::ProposalInvalidated,
                "proposal_invalidated",
                true,
            ),
            Self::DeletionPreflightRefused => (
                PublicErrorCode::DeletionPreflightRefused,
                "deletion_preflight_refused",
                true,
            ),
            Self::Busy => (PublicErrorCode::Busy, "operation_busy", true),
            Self::InvalidRequest => (PublicErrorCode::InvalidRequest, "invalid_request", false),
            Self::Cancelled => (PublicErrorCode::Cancelled, "operation_cancelled", true),
        };
        PublicError {
            code,
            message_code: message_code.to_owned(),
            retryable,
            transmitter,
        }
    }
}

fn io_kind_code(kind: std::io::ErrorKind) -> &'static str {
    match kind {
        std::io::ErrorKind::NotFound => "not_found",
        std::io::ErrorKind::PermissionDenied => "permission_denied",
        std::io::ErrorKind::AlreadyExists => "already_exists",
        std::io::ErrorKind::WouldBlock => "would_block",
        std::io::ErrorKind::InvalidInput => "invalid_input",
        std::io::ErrorKind::InvalidData => "invalid_data",
        std::io::ErrorKind::TimedOut => "timed_out",
        std::io::ErrorKind::Interrupted => "interrupted",
        std::io::ErrorKind::UnexpectedEof => "unexpected_eof",
        std::io::ErrorKind::WriteZero => "write_zero",
        std::io::ErrorKind::StorageFull => "storage_full",
        std::io::ErrorKind::ReadOnlyFilesystem => "read_only_filesystem",
        _ => "other",
    }
}
