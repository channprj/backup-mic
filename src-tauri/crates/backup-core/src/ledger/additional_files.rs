//! Companion-file evidence, plus the historical and legacy rows migrations left behind.

use rusqlite::{OptionalExtension, params};
use std::path::Path;
use std::path::PathBuf;

use crate::additional_file::AdditionalFileClass;
use crate::additional_file::VerifiedAdditionalFile;
use crate::error::CoreError;
use crate::source::SourceId;

use super::recordings::row_to_verified_recording;
use super::sql::{parse_source_id_sql, path_text, to_i64};
use super::{Ledger, LegacyRetiredRecording, VerifiedRecording};

impl Ledger {
    pub fn commit_verified_additional_file(
        &mut self,
        file: &VerifiedAdditionalFile,
    ) -> Result<String, CoreError> {
        validate_verified_additional_file(file)?;
        if let Some(existing) = self.verified_additional_file(&file.id)?
            && !additional_file_evidence_matches(&existing, file)
        {
            return Err(CoreError::LedgerCorrupt);
        }
        if let Some(existing) = self.verified_additional_file_for_source(
            &file.source_id,
            &file.source_relative_path,
            file.source_size,
            file.source_mtime_ns,
            &file.source_sha256,
        )? {
            if !additional_file_evidence_matches(&existing, file) {
                return Err(CoreError::LedgerCorrupt);
            }
            self.connection
                .execute(
                    "UPDATE additional_files SET backup_run_id = ?1 WHERE id = ?2",
                    params![file.backup_run_id, existing.id],
                )
                .map_err(CoreError::Ledger)?;
            return Ok(existing.id);
        }
        self.connection
            .execute(
                r#"INSERT INTO additional_files(
                     id, source_id, source_relative_path, source_size, source_mtime_ns,
                     source_sha256, artifact_relative_path, artifact_size, artifact_sha256,
                     classification, backup_run_id
                   ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
                params![
                    file.id,
                    file.source_id.as_str(),
                    path_text(&file.source_relative_path)?,
                    to_i64(file.source_size)?,
                    file.source_mtime_ns.to_string(),
                    file.source_sha256,
                    path_text(&file.artifact_relative_path)?,
                    to_i64(file.artifact_size)?,
                    file.artifact_sha256,
                    file.classification.storage_name(),
                    file.backup_run_id,
                ],
            )
            .map_err(CoreError::Ledger)?;
        Ok(file.id.clone())
    }

    pub fn verified_additional_file(
        &self,
        id: &str,
    ) -> Result<Option<VerifiedAdditionalFile>, CoreError> {
        let row = self
            .connection
            .query_row(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, artifact_relative_path, artifact_size,
                          artifact_sha256, classification, backup_run_id
                   FROM additional_files WHERE id = ?1"#,
                [id],
                row_to_verified_additional_file,
            )
            .optional()
            .map_err(CoreError::Ledger)?;
        if let Some(file) = &row {
            validate_verified_additional_file(file).map_err(|_| CoreError::LedgerCorrupt)?;
        }
        Ok(row)
    }

    pub fn verified_additional_files(&self) -> Result<Vec<VerifiedAdditionalFile>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, artifact_relative_path, artifact_size,
                          artifact_sha256, classification, backup_run_id
                   FROM additional_files
                   ORDER BY artifact_relative_path, id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([], row_to_verified_additional_file)
            .map_err(CoreError::Ledger)?;
        rows.map(|row| {
            let file = row.map_err(CoreError::Ledger)?;
            validate_verified_additional_file(&file).map_err(|_| CoreError::LedgerCorrupt)?;
            Ok(file)
        })
        .collect()
    }

    pub fn relocate_verified_additional_artifact(
        &mut self,
        file_id: &str,
        from_relative_path: &Path,
        to_relative_path: &Path,
        byte_count: u64,
        sha256: &str,
    ) -> Result<(), CoreError> {
        if file_id.is_empty()
            || from_relative_path == to_relative_path
            || !crate::filesystem::is_safe_additional_relative_path(from_relative_path)
            || !crate::filesystem::is_safe_additional_relative_path(to_relative_path)
            || byte_count == 0
            || sha256.len() != 64
            || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(CoreError::InvalidRequest);
        }
        let changed = self
            .connection
            .execute(
                r#"UPDATE additional_files
                   SET artifact_relative_path = ?1
                   WHERE id = ?2 AND artifact_relative_path = ?3
                     AND artifact_size = ?4 AND artifact_sha256 = ?5"#,
                params![
                    path_text(to_relative_path)?,
                    file_id,
                    path_text(from_relative_path)?,
                    to_i64(byte_count)?,
                    sha256,
                ],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::LedgerCorrupt);
        }
        Ok(())
    }

    pub fn verified_additional_file_for_source(
        &self,
        source_id: &SourceId,
        source_relative_path: &Path,
        source_size: u64,
        source_mtime_ns: i128,
        source_sha256: &str,
    ) -> Result<Option<VerifiedAdditionalFile>, CoreError> {
        if !crate::filesystem::is_safe_additional_relative_path(source_relative_path) {
            return Err(CoreError::InvalidRequest);
        }
        let row = self
            .connection
            .query_row(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, artifact_relative_path, artifact_size,
                          artifact_sha256, classification, backup_run_id
                   FROM additional_files
                   WHERE source_id = ?1 AND source_relative_path = ?2
                     AND source_size = ?3 AND source_mtime_ns = ?4 AND source_sha256 = ?5"#,
                params![
                    source_id.as_str(),
                    path_text(source_relative_path)?,
                    to_i64(source_size)?,
                    source_mtime_ns.to_string(),
                    source_sha256,
                ],
                row_to_verified_additional_file,
            )
            .optional()
            .map_err(CoreError::Ledger)?;
        if let Some(file) = &row {
            validate_verified_additional_file(file).map_err(|_| CoreError::LedgerCorrupt)?;
        }
        Ok(row)
    }

    pub fn verified_additional_file_candidate_for_source(
        &self,
        source_id: &SourceId,
        source_relative_path: &Path,
        source_size: u64,
        source_mtime_ns: i128,
    ) -> Result<Option<VerifiedAdditionalFile>, CoreError> {
        if !crate::filesystem::is_safe_additional_relative_path(source_relative_path) {
            return Err(CoreError::InvalidRequest);
        }
        let row = self
            .connection
            .query_row(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, artifact_relative_path, artifact_size,
                          artifact_sha256, classification, backup_run_id
                   FROM additional_files
                   WHERE source_id = ?1 AND source_relative_path = ?2
                     AND source_size = ?3 AND source_mtime_ns = ?4"#,
                params![
                    source_id.as_str(),
                    path_text(source_relative_path)?,
                    to_i64(source_size)?,
                    source_mtime_ns.to_string(),
                ],
                row_to_verified_additional_file,
            )
            .optional()
            .map_err(CoreError::Ledger)?;
        if let Some(file) = &row {
            validate_verified_additional_file(file).map_err(|_| CoreError::LedgerCorrupt)?;
        }
        Ok(row)
    }

    pub fn verified_additional_files_for_backup_run(
        &self,
        source_id: &SourceId,
        backup_run_id: &str,
    ) -> Result<Vec<VerifiedAdditionalFile>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, artifact_relative_path, artifact_size,
                          artifact_sha256, classification, backup_run_id
                   FROM additional_files
                   WHERE source_id = ?1 AND backup_run_id = ?2
                   ORDER BY source_relative_path, id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map(
                params![source_id.as_str(), backup_run_id],
                row_to_verified_additional_file,
            )
            .map_err(CoreError::Ledger)?;
        rows.map(|row| {
            let file = row.map_err(CoreError::Ledger)?;
            validate_verified_additional_file(&file).map_err(|_| CoreError::LedgerCorrupt)?;
            Ok(file)
        })
        .collect()
    }

    pub fn historical_wav_recordings(&self) -> Result<Vec<VerifiedRecording>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, destination_relative_path, destination_size,
                          destination_sha256, verified_at, backup_run_id, artifact_format,
                          artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
                          artifact_valid_frames, artifact_duration_micros, conversion_status,
                          conversion_error_code, retirement_status,
                          retired_session_relative_path
                   FROM recordings
                   WHERE artifact_format = 'wav'
                   ORDER BY destination_relative_path, id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([], row_to_verified_recording)
            .map_err(CoreError::Ledger)?;
        rows.map(|row| row.map_err(CoreError::Ledger)).collect()
    }

    pub fn historical_wav_recordings_for_source(
        &self,
        source_id: &SourceId,
    ) -> Result<Vec<VerifiedRecording>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, destination_relative_path, destination_size,
                          destination_sha256, verified_at, backup_run_id, artifact_format,
                          artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
                          artifact_valid_frames, artifact_duration_micros, conversion_status,
                          conversion_error_code, retirement_status,
                          retired_session_relative_path
                   FROM recordings
                   WHERE source_id = ?1 AND artifact_format = 'wav'
                   ORDER BY destination_relative_path, id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([source_id.as_str()], row_to_verified_recording)
            .map_err(CoreError::Ledger)?;
        rows.map(|row| row.map_err(CoreError::Ledger)).collect()
    }

    pub fn legacy_retired_recordings(
        &self,
        source_id: &SourceId,
    ) -> Result<Vec<LegacyRetiredRecording>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT id, source_relative_path
                   FROM recordings
                   WHERE source_id = ?1 AND retirement_status = 'legacy_deleted'
                   ORDER BY source_relative_path"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([source_id.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(CoreError::Ledger)?;
        rows.map(|row| {
            let (recording_id, source_relative_path) = row.map_err(CoreError::Ledger)?;
            let source_relative_path = PathBuf::from(source_relative_path);
            if !crate::filesystem::is_safe_relative_path(&source_relative_path) {
                return Err(CoreError::LedgerCorrupt);
            }
            Ok(LegacyRetiredRecording {
                recording_id,
                source_relative_path,
            })
        })
        .collect()
    }

    pub fn record_legacy_session_moved_to_trash(
        &mut self,
        recording_ids: &[String],
        session_relative_path: &Path,
        moved_at: &str,
    ) -> Result<(), CoreError> {
        if recording_ids.is_empty()
            || !crate::filesystem::is_safe_relative_path(session_relative_path)
        {
            return Err(CoreError::InvalidRequest);
        }
        let session = path_text(session_relative_path)?;
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        for recording_id in recording_ids {
            let changed = transaction
                .execute(
                    r#"UPDATE recordings
                       SET source_deleted_at = COALESCE(source_deleted_at, ?1),
                           deletion_error_code = NULL,
                           retirement_status = 'moved_to_trash',
                           retired_session_relative_path = ?2
                       WHERE id = ?3 AND retirement_status = 'legacy_deleted'"#,
                    params![moved_at, session, recording_id],
                )
                .map_err(CoreError::Ledger)?;
            if changed != 1 {
                return Err(CoreError::LedgerCorrupt);
            }
        }
        transaction.commit().map_err(CoreError::Ledger)
    }
}

pub(super) fn validate_verified_additional_file(
    file: &VerifiedAdditionalFile,
) -> Result<(), CoreError> {
    if file.id.is_empty()
        || !crate::filesystem::is_safe_additional_relative_path(&file.source_relative_path)
        || !crate::filesystem::is_safe_additional_relative_path(&file.artifact_relative_path)
        || file.source_size == 0
        || file.source_size != file.artifact_size
        || file.source_sha256.len() != 64
        || file.source_sha256 != file.artifact_sha256
        || file.backup_run_id.is_empty()
    {
        return Err(CoreError::InvalidRequest);
    }
    Ok(())
}

pub(super) fn additional_file_evidence_matches(
    existing: &VerifiedAdditionalFile,
    candidate: &VerifiedAdditionalFile,
) -> bool {
    existing.source_id == candidate.source_id
        && existing.source_relative_path == candidate.source_relative_path
        && existing.source_size == candidate.source_size
        && existing.source_mtime_ns == candidate.source_mtime_ns
        && existing.source_sha256 == candidate.source_sha256
        && existing.artifact_relative_path == candidate.artifact_relative_path
        && existing.artifact_size == candidate.artifact_size
        && existing.artifact_sha256 == candidate.artifact_sha256
        && existing.classification == candidate.classification
}

pub(super) fn row_to_verified_additional_file(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<VerifiedAdditionalFile> {
    let source_size = u64::try_from(row.get::<_, i64>(3)?)
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(3, 0))?;
    let source_mtime_ns = row.get::<_, String>(4)?.parse::<i128>().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid source mtime",
            )),
        )
    })?;
    let artifact_size = u64::try_from(row.get::<_, i64>(7)?)
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(7, 0))?;
    let classification = AdditionalFileClass::parse(&row.get::<_, String>(9)?).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            9,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid additional file classification",
            )),
        )
    })?;
    Ok(VerifiedAdditionalFile {
        id: row.get(0)?,
        source_id: parse_source_id_sql(&row.get::<_, String>(1)?, 1)?,
        source_relative_path: PathBuf::from(row.get::<_, String>(2)?),
        source_size,
        source_mtime_ns,
        source_sha256: row.get(5)?,
        artifact_relative_path: PathBuf::from(row.get::<_, String>(6)?),
        artifact_size,
        artifact_sha256: row.get(8)?,
        classification,
        backup_run_id: row.get(10)?,
    })
}
