//! The bounded activity feed the popover renders.

use rusqlite::params;

use crate::error::CoreError;
use crate::events::ActivityEntry;
use crate::events::ActivitySeverity;
use crate::source::SourceId;

use super::sql::{optional_i64, optional_u64};
use super::{Ledger, MAX_ACTIVITY_ENTRIES};

impl Ledger {
    pub fn append_activity(&mut self, entry: &ActivityEntry) -> Result<(), CoreError> {
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        transaction
            .execute(
                r#"INSERT INTO activity(
                     occurred_at, code, source_id, count_value, byte_value, severity
                   ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
                params![
                    entry.occurred_at,
                    entry.code,
                    entry.source_id.as_ref().map(SourceId::as_str),
                    optional_i64(entry.count_value)?,
                    optional_i64(entry.byte_value)?,
                    severity_name(entry.severity),
                ],
            )
            .map_err(CoreError::Ledger)?;
        transaction
            .execute(
                r#"DELETE FROM activity WHERE id NOT IN (
                     SELECT id FROM activity ORDER BY id DESC LIMIT ?1
                   )"#,
                [i64::try_from(MAX_ACTIVITY_ENTRIES).map_err(|_| CoreError::LedgerCorrupt)?],
            )
            .map_err(CoreError::Ledger)?;
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn recent_activity(&self, limit: usize) -> Result<Vec<ActivityEntry>, CoreError> {
        let bounded = limit.min(MAX_ACTIVITY_ENTRIES);
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT occurred_at, code, source_id, count_value, byte_value, severity
                   FROM activity ORDER BY id DESC LIMIT ?1"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map(
                [i64::try_from(bounded).map_err(|_| CoreError::LedgerCorrupt)?],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .map_err(CoreError::Ledger)?;
        rows.map(|row| {
            let (occurred_at, code, source_id, count_value, byte_value, severity) =
                row.map_err(CoreError::Ledger)?;
            Ok(ActivityEntry {
                occurred_at,
                code,
                source_id: source_id.map(|value| SourceId::parse(&value)).transpose()?,
                source_label: None,
                count_value: optional_u64(count_value)?,
                byte_value: optional_u64(byte_value)?,
                severity: parse_severity(&severity)?,
            })
        })
        .collect()
    }
}

pub(super) fn severity_name(severity: ActivitySeverity) -> &'static str {
    match severity {
        ActivitySeverity::Info => "info",
        ActivitySeverity::Success => "success",
        ActivitySeverity::Warning => "warning",
        ActivitySeverity::Error => "error",
    }
}

pub(super) fn parse_severity(value: &str) -> Result<ActivitySeverity, CoreError> {
    match value {
        "info" => Ok(ActivitySeverity::Info),
        "success" => Ok(ActivitySeverity::Success),
        "warning" => Ok(ActivitySeverity::Warning),
        "error" => Ok(ActivitySeverity::Error),
        _ => Err(CoreError::LedgerCorrupt),
    }
}
