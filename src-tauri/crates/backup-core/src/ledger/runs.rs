//! Backup runs and the per-source batch barriers that gate conversion.

use rusqlite::{OptionalExtension, params};

use crate::batch::BatchPhase;
use crate::batch::BatchRunEvidence;
use crate::batch::FrozenPreferences;
use crate::error::CoreError;
use crate::source::SourceId;

use super::Ledger;
use super::sql::to_i64;

impl Ledger {
    pub fn begin_backup_run(
        &mut self,
        id: &str,
        source_id: &SourceId,
        started_at: &str,
        required_copy_bytes: u64,
    ) -> Result<(), CoreError> {
        self.connection
            .execute(
                r#"INSERT INTO backup_runs(
                     id, source_id, started_at, outcome, required_copy_bytes
                   ) VALUES (?1, ?2, ?3, 'running', ?4)"#,
                params![
                    id,
                    source_id.as_str(),
                    started_at,
                    to_i64(required_copy_bytes)?
                ],
            )
            .map_err(CoreError::Ledger)?;
        Ok(())
    }

    pub fn begin_batch_run(
        &mut self,
        id: &str,
        source_id: &SourceId,
        started_at: &str,
        required_copy_bytes: u64,
        preferences: FrozenPreferences,
    ) -> Result<(), CoreError> {
        self.connection
            .execute(
                r#"INSERT INTO backup_runs(
                     id, source_id, started_at, outcome, required_copy_bytes, batch_phase,
                     frozen_automatic_backup, frozen_m4a_conversion, frozen_automatic_trash
                   ) VALUES (?1, ?2, ?3, 'running', ?4, 'inventory', ?5, ?6, ?7)"#,
                params![
                    id,
                    source_id.as_str(),
                    started_at,
                    to_i64(required_copy_bytes)?,
                    preferences.automatic_backup,
                    preferences.m4a_conversion,
                    preferences.automatic_trash,
                ],
            )
            .map_err(CoreError::Ledger)?;
        Ok(())
    }

    pub fn advance_batch_phase(&mut self, id: &str, next: BatchPhase) -> Result<(), CoreError> {
        let predecessor = next.predecessor().ok_or(CoreError::InvalidRequest)?;
        let changed = self
            .connection
            .execute(
                r#"UPDATE backup_runs SET batch_phase = ?1
                   WHERE id = ?2 AND outcome = 'running' AND batch_phase = ?3"#,
                params![next.storage_name(), id, predecessor.storage_name()],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::InvalidRequest);
        }
        Ok(())
    }

    pub fn batch_run_evidence(&self, id: &str) -> Result<Option<BatchRunEvidence>, CoreError> {
        let row = self
            .connection
            .query_row(
                r#"SELECT source_id, batch_phase, frozen_automatic_backup, frozen_m4a_conversion,
                          frozen_automatic_trash, m4a_profile_id
                   FROM backup_runs WHERE id = ?1"#,
                [id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                        row.get::<_, bool>(3)?,
                        row.get::<_, bool>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(CoreError::Ledger)?;
        row.map(
            |(
                source_id,
                phase,
                automatic_backup,
                m4a_conversion,
                automatic_trash,
                m4a_profile_id,
            )| {
                Ok(BatchRunEvidence {
                    source_id: source_id
                        .map(|source_id| {
                            SourceId::parse(&source_id).map_err(|_| CoreError::LedgerCorrupt)
                        })
                        .transpose()?,
                    phase: BatchPhase::parse(&phase)?,
                    frozen_preferences: FrozenPreferences {
                        automatic_backup,
                        m4a_conversion,
                        automatic_trash,
                    },
                    m4a_profile_id,
                })
            },
        )
        .transpose()
    }

    pub fn begin_conversion_cohort(
        &mut self,
        backup_run_id: &str,
        recording_ids: &[String],
        profile_id: &str,
    ) -> Result<(), CoreError> {
        if recording_ids.is_empty()
            || profile_id != crate::batch::M4A_PROFILE_ID
            || recording_ids
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != recording_ids.len()
        {
            return Err(CoreError::InvalidRequest);
        }
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        let changed = transaction
            .execute(
                r#"UPDATE backup_runs
                   SET batch_phase = 'converting', m4a_profile_id = ?1
                   WHERE id = ?2 AND outcome = 'running' AND batch_phase = 'copies_verified'
                     AND m4a_profile_id IS NULL"#,
                params![profile_id, backup_run_id],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::InvalidRequest);
        }
        for recording_id in recording_ids {
            let inserted = transaction
                .execute(
                    r#"INSERT INTO conversion_cohort_items(
                         backup_run_id, recording_id, source_id, profile_id, status
                       )
                       SELECT ?1, id, source_id, ?3, 'pending'
                       FROM recordings
                       WHERE id = ?2 AND source_id = (
                         SELECT source_id FROM backup_runs WHERE id = ?1
                       )"#,
                    params![backup_run_id, recording_id, profile_id],
                )
                .map_err(CoreError::Ledger)?;
            if inserted != 1 {
                return Err(CoreError::InvalidRequest);
            }
        }
        transaction.commit().map_err(CoreError::Ledger)
    }

    pub fn mark_conversion_item_verified(
        &mut self,
        backup_run_id: &str,
        recording_id: &str,
    ) -> Result<(), CoreError> {
        let changed = self
            .connection
            .execute(
                r#"UPDATE conversion_cohort_items
                   SET status = 'verified_m4a', error_code = NULL
                   WHERE backup_run_id = ?1 AND recording_id = ?2 AND status = 'pending'
                     AND profile_id = (
                       SELECT m4a_profile_id FROM backup_runs
                       WHERE id = ?1 AND batch_phase = 'converting'
                     )
                     AND EXISTS (
                       SELECT 1 FROM recordings
                       WHERE id = ?2 AND artifact_format = 'm4a'
                         AND conversion_status = 'complete'
                         AND artifact_codec = 'aac'
                         AND artifact_sample_rate_hz > 0
                         AND artifact_channel_count > 0
                         AND artifact_valid_frames > 0
                         AND artifact_duration_micros > 0
                     )"#,
                params![backup_run_id, recording_id],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::InvalidRequest);
        }
        Ok(())
    }

    pub fn commit_m4a_barrier(&mut self, backup_run_id: &str) -> Result<(), CoreError> {
        let changed = self
            .connection
            .execute(
                r#"UPDATE backup_runs SET batch_phase = 'm4a_cohort_verified'
                   WHERE id = ?1 AND outcome = 'running' AND batch_phase = 'converting'
                     AND m4a_profile_id IS NOT NULL
                     AND EXISTS (
                       SELECT 1 FROM conversion_cohort_items WHERE backup_run_id = ?1
                     )
                     AND NOT EXISTS (
                       SELECT 1 FROM conversion_cohort_items
                       WHERE backup_run_id = ?1 AND status != 'verified_m4a'
                     )"#,
                [backup_run_id],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::InvalidRequest);
        }
        Ok(())
    }

    pub fn commit_empty_m4a_barrier(&mut self, backup_run_id: &str) -> Result<(), CoreError> {
        let changed = self
            .connection
            .execute(
                r#"UPDATE backup_runs
                   SET batch_phase = 'm4a_cohort_verified', m4a_profile_id = ?1
                   WHERE id = ?2 AND outcome = 'running' AND batch_phase = 'copies_verified'
                     AND source_id IS NOT NULL
                     AND NOT EXISTS (
                       SELECT 1 FROM recordings
                       WHERE backup_run_id = ?2 AND artifact_format = 'wav'
                     )"#,
                params![crate::batch::M4A_PROFILE_ID, backup_run_id],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::InvalidRequest);
        }
        Ok(())
    }

    pub fn mark_interrupted_runs(&mut self, finished_at: &str) -> Result<usize, CoreError> {
        self.connection
            .execute(
                r#"UPDATE backup_runs
                   SET outcome = 'interrupted', finished_at = ?1, error_code = 'interrupted'
                   WHERE outcome = 'running'"#,
                [finished_at],
            )
            .map_err(CoreError::Ledger)
    }

    pub fn backup_run_outcome(&self, id: &str) -> Result<Option<String>, CoreError> {
        self.connection
            .query_row(
                "SELECT outcome FROM backup_runs WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(CoreError::Ledger)
    }

    pub fn finish_backup_run(
        &mut self,
        id: &str,
        finished_at: &str,
        outcome: &str,
        error_code: Option<&str>,
    ) -> Result<(), CoreError> {
        let changed = self
            .connection
            .execute(
                r#"UPDATE backup_runs
                   SET finished_at = ?1, outcome = ?2, error_code = ?3
                   WHERE id = ?4 AND outcome = 'running'"#,
                params![finished_at, outcome, error_code, id],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::InvalidRequest);
        }
        Ok(())
    }
}
