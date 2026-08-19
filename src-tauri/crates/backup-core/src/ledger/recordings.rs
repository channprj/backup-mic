//! Verified recording evidence and the destination WAVs a conversion supersedes.

use rusqlite::{OptionalExtension, params};
use std::path::Path;
use std::path::PathBuf;

use crate::artifact::ConversionStatus;
use crate::artifact::OutputFormat;
use crate::artifact::RetirementStatus;
use crate::artifact::VerifiedArtifact;
use crate::artifact::VerifiedAudioProperties;
use crate::error::CoreError;
use crate::source::SourceId;

use super::sql::{
    conversion_status_name, optional_i64, optional_u16, optional_u16_sql, optional_u32,
    optional_u32_sql, optional_u64, optional_u64_sql, output_format_name, parse_conversion_status,
    parse_output_format, parse_retirement_status, parse_source_id_sql, path_text,
    retirement_status_name, to_i64,
};
use super::{Ledger, SupersededWavEvidence, VerifiedRecording};

impl Ledger {
    pub fn commit_verified_recording(
        &mut self,
        recording: &VerifiedRecording,
    ) -> Result<String, CoreError> {
        validate_verified_recording(recording)?;
        let source_relative_path = path_text(&recording.source_relative_path)?;
        let destination_relative_path = path_text(&recording.artifact.relative_path)?;
        let retired_session_relative_path = recording
            .retired_session_relative_path
            .as_deref()
            .map(path_text)
            .transpose()?;
        let audio = recording.artifact.audio.as_ref();
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        let existing = transaction
            .query_row(
                r#"SELECT id, destination_relative_path, destination_size, destination_sha256,
                          artifact_format, artifact_codec, artifact_sample_rate_hz,
                          artifact_channel_count, artifact_valid_frames,
                          artifact_duration_micros, conversion_status,
                          conversion_error_code, retirement_status,
                          retired_session_relative_path
                   FROM recordings
                   WHERE source_id = ?1 AND source_relative_path = ?2
                     AND source_size = ?3 AND source_mtime_ns = ?4 AND source_sha256 = ?5"#,
                params![
                    recording.source_id.as_str(),
                    source_relative_path,
                    to_i64(recording.source_size)?,
                    recording.source_mtime_ns.to_string(),
                    recording.source_sha256,
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                        row.get::<_, Option<i64>>(8)?,
                        row.get::<_, Option<i64>>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, Option<String>>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, Option<String>>(13)?,
                    ))
                },
            )
            .optional()
            .map_err(CoreError::Ledger)?;
        let id = if let Some((
            id,
            existing_path,
            existing_size,
            existing_hash,
            existing_format,
            existing_codec,
            existing_sample_rate,
            existing_channels,
            existing_valid_frames,
            existing_duration,
            existing_conversion,
            existing_conversion_error,
            existing_retirement,
            existing_retired_session,
        )) = existing
        {
            if existing_path != destination_relative_path
                || optional_u64(Some(existing_size))? != Some(recording.artifact.byte_count)
                || existing_hash != recording.artifact.sha256
                || parse_output_format(&existing_format)? != recording.artifact.format
                || existing_codec.as_deref() != audio.map(|value| value.codec.as_str())
                || optional_u32(existing_sample_rate)? != audio.map(|value| value.sample_rate_hz)
                || optional_u16(existing_channels)? != audio.map(|value| value.channel_count)
                || optional_u64(existing_valid_frames)? != audio.map(|value| value.valid_frames)
                || optional_u64(existing_duration)? != audio.map(|value| value.duration_micros)
                || parse_conversion_status(&existing_conversion)? != recording.conversion_status
                || existing_conversion_error != recording.conversion_error_code
                || parse_retirement_status(&existing_retirement)? != recording.retirement_status
                || existing_retired_session.as_deref() != retired_session_relative_path
            {
                return Err(CoreError::LedgerCorrupt);
            }
            transaction
                .execute(
                    r#"UPDATE recordings
                       SET verified_at = ?1, backup_run_id = ?2
                       WHERE id = ?3"#,
                    params![recording.verified_at, recording.backup_run_id, id],
                )
                .map_err(CoreError::Ledger)?;
            id
        } else {
            transaction
                .execute(
                    r#"INSERT INTO recordings(
                         id, source_id, source_relative_path, source_size, source_mtime_ns,
                         source_sha256, destination_relative_path, destination_size,
                         destination_sha256, verified_at, backup_run_id, artifact_format,
                         artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
                         artifact_valid_frames, artifact_duration_micros, conversion_status,
                         conversion_error_code, retirement_status, retired_session_relative_path
                       ) VALUES (
                         ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                         ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
                       )"#,
                    params![
                        recording.id,
                        recording.source_id.as_str(),
                        source_relative_path,
                        to_i64(recording.source_size)?,
                        recording.source_mtime_ns.to_string(),
                        recording.source_sha256,
                        destination_relative_path,
                        to_i64(recording.artifact.byte_count)?,
                        recording.artifact.sha256,
                        recording.verified_at,
                        recording.backup_run_id,
                        output_format_name(recording.artifact.format),
                        audio.map(|value| value.codec.as_str()),
                        audio.map(|value| i64::from(value.sample_rate_hz)),
                        audio.map(|value| i64::from(value.channel_count)),
                        optional_i64(audio.map(|value| value.valid_frames))?,
                        optional_i64(audio.map(|value| value.duration_micros))?,
                        conversion_status_name(recording.conversion_status),
                        recording.conversion_error_code,
                        retirement_status_name(recording.retirement_status),
                        retired_session_relative_path,
                    ],
                )
                .map_err(CoreError::Ledger)?;
            recording.id.clone()
        };
        transaction.commit().map_err(CoreError::Ledger)?;
        Ok(id)
    }

    pub fn verified_recording(&self, id: &str) -> Result<Option<VerifiedRecording>, CoreError> {
        self.connection
            .query_row(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, destination_relative_path, destination_size,
                          destination_sha256, verified_at, backup_run_id, artifact_format,
                          artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
                          artifact_valid_frames, artifact_duration_micros, conversion_status,
                          conversion_error_code, retirement_status,
                          retired_session_relative_path
                   FROM recordings WHERE id = ?1"#,
                [id],
                row_to_verified_recording,
            )
            .optional()
            .map_err(CoreError::Ledger)
    }

    pub fn verified_recordings(&self) -> Result<Vec<VerifiedRecording>, CoreError> {
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
                   ORDER BY destination_relative_path, id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([], row_to_verified_recording)
            .map_err(CoreError::Ledger)?;
        rows.map(|row| {
            let recording = row.map_err(CoreError::Ledger)?;
            validate_verified_recording(&recording).map_err(|_| CoreError::LedgerCorrupt)?;
            Ok(recording)
        })
        .collect()
    }

    pub fn verified_recordings_for_backup_run(
        &self,
        source_id: &SourceId,
        backup_run_id: &str,
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
                   WHERE source_id = ?1 AND backup_run_id = ?2
                   ORDER BY source_relative_path, id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map(
                params![source_id.as_str(), backup_run_id],
                row_to_verified_recording,
            )
            .map_err(CoreError::Ledger)?;
        rows.map(|row| {
            let recording = row.map_err(CoreError::Ledger)?;
            validate_verified_recording(&recording).map_err(|_| CoreError::LedgerCorrupt)?;
            Ok(recording)
        })
        .collect()
    }

    pub fn relocate_verified_artifact(
        &mut self,
        recording_id: &str,
        from_relative_path: &Path,
        to_relative_path: &Path,
        byte_count: u64,
        sha256: &str,
    ) -> Result<(), CoreError> {
        if recording_id.is_empty()
            || from_relative_path == to_relative_path
            || !crate::filesystem::is_safe_relative_path(from_relative_path)
            || !crate::filesystem::is_safe_relative_path(to_relative_path)
            || byte_count == 0
            || sha256.len() != 64
            || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(CoreError::InvalidRequest);
        }
        let changed = self
            .connection
            .execute(
                r#"UPDATE recordings
                   SET destination_relative_path = ?1
                   WHERE id = ?2 AND destination_relative_path = ?3
                     AND destination_size = ?4 AND destination_sha256 = ?5"#,
                params![
                    path_text(to_relative_path)?,
                    recording_id,
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

    pub fn verified_recording_for_source(
        &self,
        source_id: &SourceId,
        source_relative_path: &Path,
        source_size: u64,
        source_mtime_ns: i128,
        source_sha256: &str,
    ) -> Result<Option<VerifiedRecording>, CoreError> {
        if !crate::filesystem::is_safe_relative_path(source_relative_path) {
            return Err(CoreError::InvalidRequest);
        }
        self.connection
            .query_row(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, destination_relative_path, destination_size,
                          destination_sha256, verified_at, backup_run_id, artifact_format,
                          artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
                          artifact_valid_frames, artifact_duration_micros, conversion_status,
                          conversion_error_code, retirement_status,
                          retired_session_relative_path
                   FROM recordings
                   WHERE source_id = ?1 AND source_relative_path = ?2
                     AND source_size = ?3 AND source_mtime_ns = ?4
                     AND source_sha256 = ?5"#,
                params![
                    source_id.as_str(),
                    path_text(source_relative_path)?,
                    to_i64(source_size)?,
                    source_mtime_ns.to_string(),
                    source_sha256,
                ],
                row_to_verified_recording,
            )
            .optional()
            .map_err(CoreError::Ledger)
    }

    pub fn verified_recording_candidate_for_source(
        &self,
        source_id: &SourceId,
        source_relative_path: &Path,
        source_size: u64,
        source_mtime_ns: i128,
    ) -> Result<Option<VerifiedRecording>, CoreError> {
        if !crate::filesystem::is_safe_relative_path(source_relative_path) {
            return Err(CoreError::InvalidRequest);
        }
        let row = self
            .connection
            .query_row(
                r#"SELECT id, source_id, source_relative_path, source_size, source_mtime_ns,
                          source_sha256, destination_relative_path, destination_size,
                          destination_sha256, verified_at, backup_run_id, artifact_format,
                          artifact_codec, artifact_sample_rate_hz, artifact_channel_count,
                          artifact_valid_frames, artifact_duration_micros, conversion_status,
                          conversion_error_code, retirement_status,
                          retired_session_relative_path
                   FROM recordings
                   WHERE source_id = ?1 AND source_relative_path = ?2
                     AND source_size = ?3 AND source_mtime_ns = ?4"#,
                params![
                    source_id.as_str(),
                    path_text(source_relative_path)?,
                    to_i64(source_size)?,
                    source_mtime_ns.to_string(),
                ],
                row_to_verified_recording,
            )
            .optional()
            .map_err(CoreError::Ledger)?;
        if let Some(recording) = &row {
            validate_verified_recording(recording).map_err(|_| CoreError::LedgerCorrupt)?;
        }
        Ok(row)
    }

    pub fn replace_verified_artifact(
        &mut self,
        recording: &VerifiedRecording,
    ) -> Result<(), CoreError> {
        self.replace_verified_artifact_evidence(recording, None)
    }

    pub fn replace_verified_recording_from_fresh_copy(
        &mut self,
        recording: &VerifiedRecording,
    ) -> Result<String, CoreError> {
        validate_verified_recording(recording)?;
        if recording.artifact.format != OutputFormat::Wav
            || recording.artifact.audio.is_some()
            || recording.conversion_status != ConversionStatus::NotRequired
            || recording.conversion_error_code.is_some()
            || recording.retirement_status != RetirementStatus::Present
            || recording.retired_session_relative_path.is_some()
        {
            return Err(CoreError::InvalidRequest);
        }

        let source_relative_path = path_text(&recording.source_relative_path)?;
        let destination_relative_path = path_text(&recording.artifact.relative_path)?;
        let transaction = self.connection.transaction().map_err(CoreError::Ledger)?;
        let id = transaction
            .query_row(
                r#"SELECT id FROM recordings
                   WHERE source_id = ?1 AND source_relative_path = ?2
                     AND source_size = ?3 AND source_mtime_ns = ?4
                     AND source_sha256 = ?5"#,
                params![
                    recording.source_id.as_str(),
                    source_relative_path,
                    to_i64(recording.source_size)?,
                    recording.source_mtime_ns.to_string(),
                    recording.source_sha256,
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(CoreError::Ledger)?
            .ok_or(CoreError::LedgerCorrupt)?;
        let changed = transaction
            .execute(
                r#"UPDATE recordings
                   SET destination_relative_path = ?1, destination_size = ?2,
                       destination_sha256 = ?3, verified_at = ?4, backup_run_id = ?5,
                       artifact_format = 'wav', artifact_codec = NULL,
                       artifact_sample_rate_hz = NULL, artifact_channel_count = NULL,
                       artifact_valid_frames = NULL, artifact_duration_micros = NULL,
                       conversion_status = 'not_required', conversion_error_code = NULL,
                       retirement_status = 'present', retired_session_relative_path = NULL,
                       superseded_wav_relative_path = NULL, superseded_wav_size = NULL,
                       superseded_wav_sha256 = NULL, superseded_wav_retirement_status = 'none'
                   WHERE id = ?6 AND source_id = ?7 AND source_relative_path = ?8
                     AND source_size = ?9 AND source_mtime_ns = ?10
                     AND source_sha256 = ?11"#,
                params![
                    destination_relative_path,
                    to_i64(recording.artifact.byte_count)?,
                    recording.artifact.sha256,
                    recording.verified_at,
                    recording.backup_run_id,
                    id,
                    recording.source_id.as_str(),
                    source_relative_path,
                    to_i64(recording.source_size)?,
                    recording.source_mtime_ns.to_string(),
                    recording.source_sha256,
                ],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::LedgerCorrupt);
        }
        transaction.commit().map_err(CoreError::Ledger)?;
        Ok(id)
    }

    pub fn replace_verified_recording_for_recovery(
        &mut self,
        expected: &VerifiedRecording,
        recording: &VerifiedRecording,
    ) -> Result<(), CoreError> {
        validate_verified_recording(expected)?;
        validate_verified_recording(recording)?;
        if recording.id != expected.id
            || recording.source_id != expected.source_id
            || recording.source_relative_path != expected.source_relative_path
            || recording.source_size != expected.source_size
            || recording.source_mtime_ns != expected.source_mtime_ns
            || recording.source_sha256 != expected.source_sha256
            || recording.backup_run_id != expected.backup_run_id
            || recording.retirement_status != expected.retirement_status
            || recording.retired_session_relative_path != expected.retired_session_relative_path
            || recording.artifact.format != OutputFormat::Wav
            || recording.artifact.audio.is_some()
            || recording.conversion_status != ConversionStatus::NotRequired
            || recording.conversion_error_code.is_some()
        {
            return Err(CoreError::InvalidRequest);
        }

        let changed = self
            .connection
            .execute(
                r#"UPDATE recordings
                   SET destination_relative_path = ?1, destination_size = ?2,
                       destination_sha256 = ?3, verified_at = ?4,
                       artifact_format = 'wav', artifact_codec = NULL,
                       artifact_sample_rate_hz = NULL, artifact_channel_count = NULL,
                       artifact_valid_frames = NULL, artifact_duration_micros = NULL,
                       conversion_status = 'not_required', conversion_error_code = NULL,
                       superseded_wav_relative_path = NULL, superseded_wav_size = NULL,
                       superseded_wav_sha256 = NULL, superseded_wav_retirement_status = 'none'
                   WHERE id = ?5 AND source_id = ?6 AND source_relative_path = ?7
                     AND source_size = ?8 AND source_mtime_ns = ?9
                     AND source_sha256 = ?10 AND destination_relative_path = ?11
                     AND destination_size = ?12 AND destination_sha256 = ?13
                     AND artifact_format = ?14 AND retirement_status = ?15"#,
                params![
                    path_text(&recording.artifact.relative_path)?,
                    to_i64(recording.artifact.byte_count)?,
                    recording.artifact.sha256,
                    recording.verified_at,
                    expected.id,
                    expected.source_id.as_str(),
                    path_text(&expected.source_relative_path)?,
                    to_i64(expected.source_size)?,
                    expected.source_mtime_ns.to_string(),
                    expected.source_sha256,
                    path_text(&expected.artifact.relative_path)?,
                    to_i64(expected.artifact.byte_count)?,
                    expected.artifact.sha256,
                    output_format_name(expected.artifact.format),
                    retirement_status_name(expected.retirement_status),
                ],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::LedgerCorrupt);
        }
        Ok(())
    }

    pub fn replace_verified_artifact_with_superseded_wav(
        &mut self,
        recording: &VerifiedRecording,
        superseded_relative_path: &Path,
        superseded_size: u64,
        superseded_sha256: &str,
    ) -> Result<(), CoreError> {
        if !crate::filesystem::is_safe_relative_path(superseded_relative_path)
            || superseded_size == 0
            || superseded_sha256.len() != 64
            || !superseded_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(CoreError::InvalidRequest);
        }
        self.replace_verified_artifact_evidence(
            recording,
            Some((superseded_relative_path, superseded_size, superseded_sha256)),
        )
    }

    fn replace_verified_artifact_evidence(
        &mut self,
        recording: &VerifiedRecording,
        superseded: Option<(&Path, u64, &str)>,
    ) -> Result<(), CoreError> {
        validate_verified_recording(recording)?;
        let destination_relative_path = path_text(&recording.artifact.relative_path)?;
        let audio = recording
            .artifact
            .audio
            .as_ref()
            .ok_or(CoreError::InvalidRequest)?;
        let superseded_path = superseded.map(|value| path_text(value.0)).transpose()?;
        let superseded_size = superseded.map(|value| to_i64(value.1)).transpose()?;
        let superseded_sha256 = superseded.map(|value| value.2);
        let changed = self
            .connection
            .execute(
                r#"UPDATE recordings
                   SET destination_relative_path = ?1, destination_size = ?2,
                       destination_sha256 = ?3, artifact_format = ?4,
                       artifact_codec = ?5, artifact_sample_rate_hz = ?6,
                       artifact_channel_count = ?7, artifact_valid_frames = ?8,
                       artifact_duration_micros = ?9, conversion_status = ?10,
                       conversion_error_code = ?11, verified_at = ?12,
                       backup_run_id = ?13,
                       superseded_wav_relative_path = COALESCE(?14, superseded_wav_relative_path),
                       superseded_wav_size = COALESCE(?15, superseded_wav_size),
                       superseded_wav_sha256 = COALESCE(?16, superseded_wav_sha256),
                       superseded_wav_retirement_status = CASE
                           WHEN ?14 IS NOT NULL THEN 'pending'
                           ELSE superseded_wav_retirement_status
                       END
                   WHERE id = ?17 AND source_id = ?18 AND source_relative_path = ?19
                     AND source_size = ?20 AND source_mtime_ns = ?21
                     AND source_sha256 = ?22"#,
                params![
                    destination_relative_path,
                    to_i64(recording.artifact.byte_count)?,
                    recording.artifact.sha256,
                    output_format_name(recording.artifact.format),
                    audio.codec,
                    i64::from(audio.sample_rate_hz),
                    i64::from(audio.channel_count),
                    to_i64(audio.valid_frames)?,
                    to_i64(audio.duration_micros)?,
                    conversion_status_name(recording.conversion_status),
                    recording.conversion_error_code,
                    recording.verified_at,
                    recording.backup_run_id,
                    superseded_path,
                    superseded_size,
                    superseded_sha256,
                    recording.id,
                    recording.source_id.as_str(),
                    path_text(&recording.source_relative_path)?,
                    to_i64(recording.source_size)?,
                    recording.source_mtime_ns.to_string(),
                    recording.source_sha256,
                ],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::LedgerCorrupt);
        }
        Ok(())
    }

    pub fn superseded_wav_evidence(
        &self,
        recording_id: &str,
    ) -> Result<Option<(PathBuf, u64, String)>, CoreError> {
        let row = self
            .connection
            .query_row(
                r#"SELECT superseded_wav_relative_path, superseded_wav_size,
                          superseded_wav_sha256
                   FROM recordings WHERE id = ?1"#,
                [recording_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(CoreError::Ledger)?;
        let Some((path, size, sha256)) = row else {
            return Ok(None);
        };
        match (path, size, sha256) {
            (Some(path), Some(size), Some(sha256)) => Ok(Some((
                PathBuf::from(path),
                u64::try_from(size).map_err(|_| CoreError::LedgerCorrupt)?,
                sha256,
            ))),
            (None, None, None) => Ok(None),
            _ => Err(CoreError::LedgerCorrupt),
        }
    }

    pub fn pending_superseded_wavs(&self) -> Result<Vec<SupersededWavEvidence>, CoreError> {
        let mut statement = self
            .connection
            .prepare(
                r#"SELECT id, source_id, superseded_wav_relative_path,
                          superseded_wav_size, superseded_wav_sha256
                   FROM recordings
                   WHERE superseded_wav_retirement_status = 'pending'
                   ORDER BY superseded_wav_relative_path, id"#,
            )
            .map_err(CoreError::Ledger)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(CoreError::Ledger)?;
        rows.map(|row| {
            let (recording_id, source_id, path, size, sha256) = row.map_err(CoreError::Ledger)?;
            let (Some(path), Some(size), Some(sha256)) = (path, size, sha256) else {
                return Err(CoreError::LedgerCorrupt);
            };
            let relative_path = PathBuf::from(path);
            let byte_count = u64::try_from(size).map_err(|_| CoreError::LedgerCorrupt)?;
            if !crate::filesystem::is_safe_relative_path(&relative_path)
                || byte_count == 0
                || sha256.len() != 64
            {
                return Err(CoreError::LedgerCorrupt);
            }
            Ok(SupersededWavEvidence {
                recording_id,
                source_id: SourceId::parse(&source_id).map_err(|_| CoreError::LedgerCorrupt)?,
                relative_path,
                byte_count,
                sha256,
            })
        })
        .collect()
    }

    pub fn mark_superseded_wav_moved_to_trash(
        &mut self,
        recording_id: &str,
    ) -> Result<(), CoreError> {
        self.mark_superseded_wav_retired(recording_id, "moved_to_trash")
    }

    pub fn mark_superseded_wav_absent_after_conversion(
        &mut self,
        recording_id: &str,
    ) -> Result<(), CoreError> {
        self.mark_superseded_wav_retired(recording_id, "absent_after_conversion")
    }

    fn mark_superseded_wav_retired(
        &mut self,
        recording_id: &str,
        status: &str,
    ) -> Result<(), CoreError> {
        let changed = self
            .connection
            .execute(
                r#"UPDATE recordings
                   SET superseded_wav_retirement_status = ?2
                   WHERE id = ?1 AND superseded_wav_retirement_status = 'pending'
                     AND superseded_wav_relative_path IS NOT NULL
                     AND superseded_wav_size IS NOT NULL
                     AND superseded_wav_sha256 IS NOT NULL"#,
                params![recording_id, status],
            )
            .map_err(CoreError::Ledger)?;
        if changed != 1 {
            return Err(CoreError::InvalidRequest);
        }
        Ok(())
    }

    pub fn verified_recording_count(&self) -> Result<u64, CoreError> {
        let count = self
            .connection
            .query_row("SELECT COUNT(*) FROM recordings", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(CoreError::Ledger)?;
        u64::try_from(count).map_err(|_| CoreError::LedgerCorrupt)
    }
}

pub(super) fn validate_verified_recording(recording: &VerifiedRecording) -> Result<(), CoreError> {
    let invalid_audio = recording.artifact.audio.as_ref().is_some_and(|audio| {
        audio.codec.is_empty()
            || audio.sample_rate_hz == 0
            || audio.channel_count == 0
            || audio.valid_frames == 0
            || audio.duration_micros == 0
    });
    if !crate::filesystem::is_safe_relative_path(&recording.source_relative_path)
        || !crate::filesystem::is_safe_relative_path(&recording.artifact.relative_path)
        || recording.source_sha256.len() != 64
        || recording.artifact.sha256.len() != 64
        || recording.artifact.byte_count == 0
        || invalid_audio
        || recording
            .retired_session_relative_path
            .as_deref()
            .is_some_and(|path| !crate::filesystem::is_safe_relative_path(path))
    {
        return Err(CoreError::InvalidRequest);
    }
    match recording.artifact.format {
        OutputFormat::Wav
            if recording.source_size == recording.artifact.byte_count
                && recording.source_sha256 == recording.artifact.sha256
                && recording.artifact.audio.is_none()
                && recording.conversion_status == ConversionStatus::NotRequired =>
        {
            Ok(())
        }
        OutputFormat::M4a
            if recording.artifact.audio.is_some()
                && recording.conversion_status == ConversionStatus::Complete =>
        {
            Ok(())
        }
        OutputFormat::Wav | OutputFormat::M4a => Err(CoreError::InvalidRequest),
    }
}

pub(super) fn row_to_verified_recording(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<VerifiedRecording> {
    let source_size = u64::try_from(row.get::<_, i64>(3)?)
        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(3, 0))?;
    let source_mtime_text = row.get::<_, String>(4)?;
    let source_mtime_ns = source_mtime_text.parse::<i128>().map_err(|_| {
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
    let format_text = row.get::<_, String>(11)?;
    let format = parse_output_format(&format_text).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            11,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid artifact format",
            )),
        )
    })?;
    let codec = row.get::<_, Option<String>>(12)?;
    let sample_rate = optional_u32_sql(row.get::<_, Option<i64>>(13)?, 13)?;
    let channels = optional_u16_sql(row.get::<_, Option<i64>>(14)?, 14)?;
    let valid_frames = optional_u64_sql(row.get::<_, Option<i64>>(15)?, 15)?;
    let duration_micros = optional_u64_sql(row.get::<_, Option<i64>>(16)?, 16)?;
    let audio = match (codec, sample_rate, channels, valid_frames, duration_micros) {
        (None, None, None, None, None) => None,
        (
            Some(codec),
            Some(sample_rate_hz),
            Some(channel_count),
            Some(valid_frames),
            Some(duration_micros),
        ) => Some(VerifiedAudioProperties {
            codec,
            sample_rate_hz,
            channel_count,
            valid_frames,
            duration_micros,
        }),
        _ => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                12,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "incomplete artifact audio properties",
                )),
            ));
        }
    };
    let conversion_text = row.get::<_, String>(17)?;
    let conversion_status = parse_conversion_status(&conversion_text).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            17,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid conversion status",
            )),
        )
    })?;
    let retirement_text = row.get::<_, String>(19)?;
    let retirement_status = parse_retirement_status(&retirement_text).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            19,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid retirement status",
            )),
        )
    })?;
    Ok(VerifiedRecording {
        id: row.get(0)?,
        source_id: parse_source_id_sql(&row.get::<_, String>(1)?, 1)?,
        source_relative_path: PathBuf::from(row.get::<_, String>(2)?),
        source_size,
        source_mtime_ns,
        source_sha256: row.get(5)?,
        artifact: VerifiedArtifact {
            relative_path: PathBuf::from(row.get::<_, String>(6)?),
            format,
            byte_count: artifact_size,
            sha256: row.get(8)?,
            audio,
        },
        verified_at: row.get(9)?,
        backup_run_id: row.get(10)?,
        conversion_status,
        conversion_error_code: row.get(18)?,
        retirement_status,
        retired_session_relative_path: row.get::<_, Option<String>>(20)?.map(PathBuf::from),
    })
}
