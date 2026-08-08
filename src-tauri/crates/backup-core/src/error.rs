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
    DeviceRemoved,
    LedgerCorrupt,
    ProposalExpired,
    ProposalInvalidated,
    DeletionPreflightRefused,
    PartialDeletion,
    Busy,
    InvalidRequest,
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
    #[error("device was removed")]
    DeviceRemoved,
    #[error("ledger is corrupt")]
    LedgerCorrupt,
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
}

impl CoreError {
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
            Self::DeviceRemoved => (PublicErrorCode::DeviceRemoved, "device_removed", true),
            Self::LedgerCorrupt => (PublicErrorCode::LedgerCorrupt, "ledger_corrupt", false),
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
        };
        PublicError {
            code,
            message_code: message_code.to_owned(),
            retryable,
            transmitter,
        }
    }
}
