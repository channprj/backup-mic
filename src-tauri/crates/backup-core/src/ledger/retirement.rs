//! Deletion runs: what was proposed, what actually moved to the Trash, and what refused.

use rusqlite::params;
use std::path::Path;

use crate::error::CoreError;
use crate::source::SourceId;

use super::sql::to_i64;
use super::{Ledger, PendingAdditionalDeletionItem, PendingDeletionItem};

impl Ledger {
    pub fn begin_deletion_run(
        &mut self,
        id: &str,
        source_id: &SourceId,
        started_at: &str,
        items: &[PendingDeletionItem],
    ) -> Result<(), CoreError> {
        self.begin_deletion_run_with_additional(id, source_id, started_at, items, &[])
    }

    pub fn begin_deletion_run_with_additional(
        &mut self,
        id: &str,
        source_id: &SourceId,
        started_at: &str,
        items: &[PendingDeletionItem],
        additional_items: &[PendingAdditionalDeletionItem],
    ) -> Result<(), CoreError> {
        let proposed_bytes = items.iter().try_fold(0_u64, |total, item| {
            total
                .checked_add(item.source_size)
                .ok_or(CoreError::InvalidRequest)
        })?;
        let proposed_bytes = additional_items
            .iter()
            .try_fold(proposed_bytes, |total, item| {
                total
                    .checked_add(item.source_size)
                    .ok_or(CoreError::InvalidRequest)
            })?;
        let proposed_count = items
            .len()
            .checked_add(additional_items.len())
            .ok_or(CoreError::InvalidRequest)?;
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        transaction
            .execute(
                r#"INSERT INTO deletion_runs(
                     id, source_id, started_at, outcome, proposed_file_count, proposed_bytes
                   ) VALUES (?1, ?2, ?3, 'running', ?4, ?5)"#,
                params![
                    id,
                    source_id.as_str(),
                    started_at,
                    to_i64(u64::try_from(proposed_count).map_err(|_| CoreError::InvalidRequest)?)?,
                    to_i64(proposed_bytes)?,
                ],
            )
            .map_err(CoreError::Ledger)?;
        for item in items {
            let inserted = transaction
                .execute(
                    r#"INSERT INTO deletion_items(
                         deletion_run_id, recording_id, source_id, outcome
                       )
                       SELECT ?1, id, source_id, 'pending' FROM recordings
                       WHERE id = ?2 AND source_id = ?3"#,
                    params![id, item.recording_id, source_id.as_str()],
                )
                .map_err(CoreError::Ledger)?;
            if inserted != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
            let changed = transaction
                .execute(
                    r#"UPDATE recordings
                       SET retirement_status = 'trash_pending', deletion_error_code = NULL
                       WHERE id = ?1 AND source_deleted_at IS NULL
                         AND retirement_status IN ('present', 'failed')"#,
                    [&item.recording_id],
                )
                .map_err(CoreError::Ledger)?;
            if changed != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
        }
        for item in additional_items {
            let inserted = transaction
                .execute(
                    r#"INSERT INTO additional_deletion_items(
                         deletion_run_id, additional_file_id, source_id, outcome
                       )
                       SELECT ?1, id, source_id, 'pending' FROM additional_files
                       WHERE id = ?2 AND source_id = ?3"#,
                    params![id, item.additional_file_id, source_id.as_str()],
                )
                .map_err(CoreError::Ledger)?;
            if inserted != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
        }
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn record_deletion_target_success(
        &mut self,
        deletion_run_id: &str,
        recording_ids: &[&str],
        removed_at: &str,
        retired_session_relative_path: Option<&Path>,
    ) -> Result<(), CoreError> {
        self.record_deletion_target_success_with_additional(
            deletion_run_id,
            recording_ids,
            &[],
            removed_at,
            retired_session_relative_path,
        )
    }

    pub fn record_deletion_target_success_with_additional(
        &mut self,
        deletion_run_id: &str,
        recording_ids: &[&str],
        additional_file_ids: &[&str],
        removed_at: &str,
        retired_session_relative_path: Option<&Path>,
    ) -> Result<(), CoreError> {
        if recording_ids.is_empty() && additional_file_ids.is_empty() {
            return Err(CoreError::InvalidRequest);
        }
        let retired_session = retired_session_relative_path
            .map(|path| path.to_str().ok_or(CoreError::InvalidRequest))
            .transpose()?;
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        for recording_id in recording_ids {
            let item_changed = transaction
                .execute(
                    r#"UPDATE deletion_items
                       SET outcome = 'moved_to_trash', removed_at = ?1, error_code = NULL
                       WHERE deletion_run_id = ?2 AND recording_id = ?3 AND outcome = 'pending'"#,
                    params![removed_at, deletion_run_id, recording_id],
                )
                .map_err(CoreError::Ledger)?;
            let recording_changed = transaction
                .execute(
                    r#"UPDATE recordings
                       SET source_deleted_at = ?1, deletion_error_code = NULL,
                           retirement_status = 'moved_to_trash',
                           retired_session_relative_path = ?2
                       WHERE id = ?3 AND source_deleted_at IS NULL
                         AND retirement_status = 'trash_pending'"#,
                    params![removed_at, retired_session, recording_id],
                )
                .map_err(CoreError::Ledger)?;
            if item_changed != 1 || recording_changed != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
        }
        for additional_file_id in additional_file_ids {
            let item_changed = transaction
                .execute(
                    r#"UPDATE additional_deletion_items
                       SET outcome = 'moved_to_trash', removed_at = ?1, error_code = NULL
                       WHERE deletion_run_id = ?2 AND additional_file_id = ?3
                         AND outcome = 'pending'"#,
                    params![removed_at, deletion_run_id, additional_file_id],
                )
                .map_err(CoreError::Ledger)?;
            if item_changed != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
        }
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn record_deletion_target_failure(
        &mut self,
        deletion_run_id: &str,
        recording_ids: &[&str],
        error_code: &str,
    ) -> Result<(), CoreError> {
        self.record_deletion_target_failure_with_additional(
            deletion_run_id,
            recording_ids,
            &[],
            error_code,
        )
    }

    pub fn record_deletion_target_failure_with_additional(
        &mut self,
        deletion_run_id: &str,
        recording_ids: &[&str],
        additional_file_ids: &[&str],
        error_code: &str,
    ) -> Result<(), CoreError> {
        if recording_ids.is_empty() && additional_file_ids.is_empty() {
            return Err(CoreError::InvalidRequest);
        }
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        for recording_id in recording_ids {
            let item_changed = transaction
                .execute(
                    r#"UPDATE deletion_items SET outcome = 'failed', error_code = ?1
                       WHERE deletion_run_id = ?2 AND recording_id = ?3 AND outcome = 'pending'"#,
                    params![error_code, deletion_run_id, recording_id],
                )
                .map_err(CoreError::Ledger)?;
            let recording_changed = transaction
                .execute(
                    r#"UPDATE recordings
                       SET deletion_error_code = ?1, retirement_status = 'failed'
                       WHERE id = ?2 AND retirement_status = 'trash_pending'"#,
                    params![error_code, recording_id],
                )
                .map_err(CoreError::Ledger)?;
            if item_changed != 1 || recording_changed != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
        }
        for additional_file_id in additional_file_ids {
            let item_changed = transaction
                .execute(
                    r#"UPDATE additional_deletion_items
                       SET outcome = 'failed', error_code = ?1
                       WHERE deletion_run_id = ?2 AND additional_file_id = ?3
                         AND outcome = 'pending'"#,
                    params![error_code, deletion_run_id, additional_file_id],
                )
                .map_err(CoreError::Ledger)?;
            if item_changed != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
        }
        transaction
            .execute(
                r#"UPDATE recordings
                   SET retirement_status = 'present'
                   WHERE retirement_status = 'trash_pending'
                     AND id IN (
                       SELECT recording_id FROM deletion_items
                       WHERE deletion_run_id = ?1 AND outcome = 'pending'
                     )"#,
                [deletion_run_id],
            )
            .map_err(CoreError::Ledger)?;
        transaction
            .execute(
                r#"UPDATE deletion_items SET outcome = 'not_attempted'
                   WHERE deletion_run_id = ?1 AND outcome = 'pending'"#,
                [deletion_run_id],
            )
            .map_err(CoreError::Ledger)?;
        transaction
            .execute(
                r#"UPDATE additional_deletion_items SET outcome = 'not_attempted'
                   WHERE deletion_run_id = ?1 AND outcome = 'pending'"#,
                [deletion_run_id],
            )
            .map_err(CoreError::Ledger)?;
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn finish_deletion_run(
        &mut self,
        id: &str,
        finished_at: &str,
        outcome: &str,
        error_code: Option<&str>,
    ) -> Result<(), CoreError> {
        let changed = self
            .connection
            .execute(
                r#"UPDATE deletion_runs SET finished_at = ?1, outcome = ?2, error_code = ?3
                   WHERE id = ?4 AND outcome = 'running'"#,
                params![finished_at, outcome, error_code, id],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::LedgerCorrupt);
        }
        Ok(())
    }

    pub fn additional_deletion_outcomes(
        &self,
        deletion_run_id: &str,
    ) -> Result<Vec<(String, String)>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT additional_file_id, outcome
                   FROM additional_deletion_items
                   WHERE deletion_run_id = ?1
                   ORDER BY additional_file_id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([deletion_run_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(CoreError::Ledger)?;
        rows.map(|row| row.map_err(CoreError::Ledger)).collect()
    }

    pub fn deletion_item_outcomes(&self, id: &str) -> Result<Vec<String>, CoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT outcome FROM deletion_items WHERE deletion_run_id = ?1 ORDER BY rowid")
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([id], |row| row.get::<_, String>(0))
            .map_err(CoreError::Ledger)?;
        rows.map(|row| row.map_err(CoreError::Ledger)).collect()
    }
}
