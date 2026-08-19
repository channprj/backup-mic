//! Shared SQLite column encoding: enum names, integer widths, and row-level validation.

use std::path::Path;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use crate::artifact::ConversionStatus;
use crate::artifact::OutputFormat;
use crate::artifact::RetirementStatus;
use crate::error::CoreError;
use crate::source::SourceId;
use crate::state::Transmitter;

pub(super) fn invalid_text_column(column: usize, message: &'static str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        )),
    )
}

pub(super) fn now_timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
}

pub(super) fn transmitter_name(transmitter: Transmitter) -> &'static str {
    match transmitter {
        Transmitter::Tx01 => "TX01",
        Transmitter::Tx02 => "TX02",
    }
}

pub(super) fn parse_transmitter(value: &str) -> Result<Transmitter, CoreError> {
    match value {
        "TX01" => Ok(Transmitter::Tx01),
        "TX02" => Ok(Transmitter::Tx02),
        _ => Err(CoreError::LedgerCorrupt),
    }
}

pub(super) fn output_format_name(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Wav => "wav",
        OutputFormat::M4a => "m4a",
    }
}

pub(super) fn parse_output_format(value: &str) -> Result<OutputFormat, CoreError> {
    match value {
        "wav" => Ok(OutputFormat::Wav),
        "m4a" => Ok(OutputFormat::M4a),
        _ => Err(CoreError::LedgerCorrupt),
    }
}

pub(super) fn conversion_status_name(status: ConversionStatus) -> &'static str {
    match status {
        ConversionStatus::NotRequired => "not_required",
        ConversionStatus::Pending => "pending",
        ConversionStatus::Complete => "complete",
        ConversionStatus::Failed => "failed",
    }
}

pub(super) fn parse_conversion_status(value: &str) -> Result<ConversionStatus, CoreError> {
    match value {
        "not_required" => Ok(ConversionStatus::NotRequired),
        "pending" => Ok(ConversionStatus::Pending),
        "complete" => Ok(ConversionStatus::Complete),
        "failed" => Ok(ConversionStatus::Failed),
        _ => Err(CoreError::LedgerCorrupt),
    }
}

pub(super) fn retirement_status_name(status: RetirementStatus) -> &'static str {
    match status {
        RetirementStatus::Present => "present",
        RetirementStatus::TrashPending => "trash_pending",
        RetirementStatus::MovedToTrash => "moved_to_trash",
        RetirementStatus::LegacyDeleted => "legacy_deleted",
        RetirementStatus::Failed => "failed",
    }
}

pub(super) fn parse_retirement_status(value: &str) -> Result<RetirementStatus, CoreError> {
    match value {
        "present" => Ok(RetirementStatus::Present),
        "trash_pending" => Ok(RetirementStatus::TrashPending),
        "moved_to_trash" => Ok(RetirementStatus::MovedToTrash),
        "legacy_deleted" => Ok(RetirementStatus::LegacyDeleted),
        "failed" => Ok(RetirementStatus::Failed),
        _ => Err(CoreError::LedgerCorrupt),
    }
}

pub(super) fn parse_source_id_sql(value: &str, column: usize) -> rusqlite::Result<SourceId> {
    SourceId::parse(value).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid source id",
            )),
        )
    })
}

pub(super) fn to_i64(value: u64) -> Result<i64, CoreError> {
    i64::try_from(value).map_err(|_| CoreError::LedgerCorrupt)
}

pub(super) fn optional_i64(value: Option<u64>) -> Result<Option<i64>, CoreError> {
    value.map(to_i64).transpose()
}

pub(super) fn optional_u64(value: Option<i64>) -> Result<Option<u64>, CoreError> {
    value
        .map(|value| u64::try_from(value).map_err(|_| CoreError::LedgerCorrupt))
        .transpose()
}

pub(super) fn optional_u32(value: Option<i64>) -> Result<Option<u32>, CoreError> {
    value
        .map(|value| u32::try_from(value).map_err(|_| CoreError::LedgerCorrupt))
        .transpose()
}

pub(super) fn optional_u16(value: Option<i64>) -> Result<Option<u16>, CoreError> {
    value
        .map(|value| u16::try_from(value).map_err(|_| CoreError::LedgerCorrupt))
        .transpose()
}

pub(super) fn optional_u64_sql(value: Option<i64>, column: usize) -> rusqlite::Result<Option<u64>> {
    value
        .map(|value| {
            u64::try_from(value)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
        })
        .transpose()
}

pub(super) fn optional_u32_sql(value: Option<i64>, column: usize) -> rusqlite::Result<Option<u32>> {
    value
        .map(|value| {
            u32::try_from(value)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
        })
        .transpose()
}

pub(super) fn optional_u16_sql(value: Option<i64>, column: usize) -> rusqlite::Result<Option<u16>> {
    value
        .map(|value| {
            u16::try_from(value)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(column, value))
        })
        .transpose()
}

pub(super) fn path_text(path: &Path) -> Result<&str, CoreError> {
    path.to_str().ok_or(CoreError::InvalidRequest)
}
