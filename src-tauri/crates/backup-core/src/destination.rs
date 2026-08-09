use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    error::CoreError,
    filesystem::{
        is_recognized_session_name, is_safe_additional_relative_path, is_safe_relative_path,
    },
    hash::{FileDigest, hash_file},
    recording::{AdditionalFileObservation, RecordingObservation},
    state::Transmitter,
};

pub const DEFAULT_CAPACITY_RESERVE_BYTES: u64 = 10 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationDisposition {
    Copy,
    Reuse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationPlan {
    pub source: RecordingObservation,
    pub source_sha256: String,
    pub relative_destination: PathBuf,
    pub disposition: DestinationDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdditionalFilePlan {
    pub source: AdditionalFileObservation,
    pub source_sha256: String,
    pub relative_destination: PathBuf,
    pub disposition: DestinationDisposition,
}

pub fn plan_additional_file(
    source_root: &Path,
    destination_root: &Path,
    transmitter: Transmitter,
    source: AdditionalFileObservation,
) -> Result<AdditionalFilePlan, CoreError> {
    if !is_safe_additional_relative_path(&source.relative_path)
        || source
            .relative_path
            .file_name()
            .and_then(|name| name.to_str())
            != Some(source.file_name.as_str())
    {
        return Err(CoreError::InvalidRequest);
    }
    let mut components = source.relative_path.components();
    let session = components
        .next()
        .map(|component| PathBuf::from(component.as_os_str()))
        .ok_or(CoreError::InvalidRequest)?;
    if !is_recognized_session_name(&session)
        || components.next().is_none()
        || components.next().is_some()
    {
        return Err(CoreError::InvalidRequest);
    }
    let date = session_date(&session)?;
    let source_digest = hash_file(&source_root.join(&source.relative_path))?;
    if source_digest.size != source.size {
        return Err(CoreError::SourceChanged);
    }
    let default_relative = PathBuf::from("source-extras")
        .join(date.year().to_string())
        .join(date.to_string())
        .join(transmitter_name(transmitter))
        .join(&source.relative_path);
    if !is_safe_additional_relative_path(&default_relative) {
        return Err(CoreError::InvalidRequest);
    }
    let (relative_destination, disposition) =
        choose_available_name(destination_root, &default_relative, &source_digest)?;
    Ok(AdditionalFilePlan {
        source,
        source_sha256: source_digest.sha256,
        relative_destination,
        disposition,
    })
}

pub fn plan_recording(
    source_root: &Path,
    destination_root: &Path,
    transmitter: Transmitter,
    source: RecordingObservation,
) -> Result<DestinationPlan, CoreError> {
    if !is_safe_relative_path(&source.relative_path)
        || source
            .relative_path
            .file_name()
            .and_then(|name| name.to_str())
            != Some(source.file_name.as_str())
        || source.file_name.starts_with('.')
    {
        return Err(CoreError::InvalidRequest);
    }
    let source_path = source_root.join(&source.relative_path);
    let source_digest = hash_file(&source_path)?;
    if source_digest.size != source.size {
        return Err(CoreError::SourceChanged);
    }
    let directory = PathBuf::from(source.parsed_name.destination_date.year().to_string())
        .join(source.parsed_name.destination_date.to_string())
        .join(transmitter_name(transmitter));
    let default_relative = directory.join(&source.file_name);
    if !is_safe_relative_path(&default_relative) {
        return Err(CoreError::InvalidRequest);
    }
    let (relative_destination, disposition) =
        choose_available_name(destination_root, &default_relative, &source_digest)?;
    Ok(DestinationPlan {
        source,
        source_sha256: source_digest.sha256,
        relative_destination,
        disposition,
    })
}

pub fn required_copy_bytes(plans: &[DestinationPlan]) -> Result<u64, CoreError> {
    plans
        .iter()
        .filter(|plan| plan.disposition == DestinationDisposition::Copy)
        .try_fold(0_u64, |total, plan| {
            total
                .checked_add(plan.source.size)
                .ok_or(CoreError::InvalidRequest)
        })
}

pub fn required_additional_copy_bytes(plans: &[AdditionalFilePlan]) -> Result<u64, CoreError> {
    plans
        .iter()
        .filter(|plan| plan.disposition == DestinationDisposition::Copy)
        .try_fold(0_u64, |total, plan| {
            total
                .checked_add(plan.source.size)
                .ok_or(CoreError::InvalidRequest)
        })
}

pub fn capacity_is_sufficient(available: u64, required: u64, reserve: u64) -> bool {
    required
        .checked_add(reserve)
        .is_some_and(|minimum| available >= minimum)
}

fn choose_available_name(
    destination_root: &Path,
    default_relative: &Path,
    source_digest: &FileDigest,
) -> Result<(PathBuf, DestinationDisposition), CoreError> {
    if let Some(disposition) =
        disposition_for(&destination_root.join(default_relative), source_digest)?
    {
        if disposition == DestinationDisposition::Reuse {
            return Ok((default_relative.to_path_buf(), disposition));
        }
    } else {
        return Ok((default_relative.to_path_buf(), DestinationDisposition::Copy));
    }

    for prefix_length in (8..=source_digest.sha256.len()).step_by(4) {
        let candidate = with_hash_suffix(default_relative, &source_digest.sha256[..prefix_length])?;
        match disposition_for(&destination_root.join(&candidate), source_digest)? {
            None => return Ok((candidate, DestinationDisposition::Copy)),
            Some(DestinationDisposition::Reuse) => {
                return Ok((candidate, DestinationDisposition::Reuse));
            }
            Some(DestinationDisposition::Copy) => {}
        }
    }
    Err(CoreError::DestinationUnavailable)
}

fn disposition_for(
    path: &Path,
    source_digest: &FileDigest,
) -> Result<Option<DestinationDisposition>, CoreError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(CoreError::CopyFailed(error)),
    };
    if !metadata.file_type().is_file() {
        return Err(CoreError::DestinationUnavailable);
    }
    if metadata.len() == source_digest.size {
        let existing_digest = hash_file(path)?;
        if existing_digest == *source_digest {
            return Ok(Some(DestinationDisposition::Reuse));
        }
    }
    Ok(Some(DestinationDisposition::Copy))
}

fn with_hash_suffix(path: &Path, hash_prefix: &str) -> Result<PathBuf, CoreError> {
    let parent = path.parent().ok_or(CoreError::InvalidRequest)?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(CoreError::InvalidRequest)?;
    if let (Some(stem), Some(extension)) = (
        path.file_stem().and_then(|value| value.to_str()),
        path.extension().and_then(|value| value.to_str()),
    ) {
        Ok(parent.join(format!("{stem}-{hash_prefix}.{extension}")))
    } else {
        Ok(parent.join(format!("{file_name}-{hash_prefix}")))
    }
}

fn session_date(session: &Path) -> Result<time::Date, CoreError> {
    let name = session.to_str().ok_or(CoreError::InvalidRequest)?;
    let encoded = name.split('_').nth(2).ok_or(CoreError::InvalidRequest)?;
    let year = encoded
        .get(0..4)
        .and_then(|value| value.parse().ok())
        .ok_or(CoreError::InvalidRequest)?;
    let month = encoded
        .get(4..6)
        .and_then(|value| value.parse::<u8>().ok())
        .and_then(|value| time::Month::try_from(value).ok())
        .ok_or(CoreError::InvalidRequest)?;
    let day = encoded
        .get(6..8)
        .and_then(|value| value.parse().ok())
        .ok_or(CoreError::InvalidRequest)?;
    time::Date::from_calendar_date(year, month, day).map_err(|_| CoreError::InvalidRequest)
}

fn transmitter_name(transmitter: Transmitter) -> &'static str {
    match transmitter {
        Transmitter::Tx01 => "TX01",
        Transmitter::Tx02 => "TX02",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use time::macros::date;

    use crate::recording::ParsedRecordingName;

    use super::*;

    fn observed(file_name: &str, size: u64) -> RecordingObservation {
        RecordingObservation {
            relative_path: PathBuf::from(file_name),
            file_name: file_name.to_owned(),
            size,
            modified_nanos: 42,
            parsed_name: ParsedRecordingName {
                transmitter_hint: Some(Transmitter::Tx01),
                destination_date: date!(2026 - 08 - 09),
                used_fallback_date: false,
            },
        }
    }

    #[test]
    fn uses_the_approved_date_and_transmitter_layout() {
        let source = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let file_name = "TX01_MIC001_20260809_010203.wav";
        fs::write(source.path().join(file_name), b"audio").unwrap();

        let plan = plan_recording(
            source.path(),
            destination.path(),
            Transmitter::Tx01,
            observed(file_name, 5),
        )
        .unwrap();

        assert_eq!(plan.disposition, DestinationDisposition::Copy);
        assert_eq!(
            plan.relative_destination,
            Path::new("2026/2026-08-09/TX01").join(file_name)
        );
    }

    #[test]
    fn reuses_equal_content_and_suffixes_a_real_name_collision() {
        let source = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let file_name = "TX01_MIC001_20260809_010203.wav";
        fs::write(source.path().join(file_name), b"source audio").unwrap();
        let destination_directory = destination.path().join("2026/2026-08-09/TX01");
        fs::create_dir_all(&destination_directory).unwrap();
        fs::write(destination_directory.join(file_name), b"different audio").unwrap();

        let collision = plan_recording(
            source.path(),
            destination.path(),
            Transmitter::Tx01,
            observed(file_name, 12),
        )
        .unwrap();
        assert_eq!(collision.disposition, DestinationDisposition::Copy);
        assert_ne!(
            collision.relative_destination.file_name().unwrap(),
            file_name
        );
        assert!(
            collision
                .relative_destination
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .starts_with("TX01_MIC001_20260809_010203-")
        );

        fs::write(
            destination.path().join(&collision.relative_destination),
            b"source audio",
        )
        .unwrap();
        let reused = plan_recording(
            source.path(),
            destination.path(),
            Transmitter::Tx01,
            observed(file_name, 12),
        )
        .unwrap();
        assert_eq!(reused.disposition, DestinationDisposition::Reuse);
        assert_eq!(reused.relative_destination, collision.relative_destination);
    }

    #[test]
    fn capacity_keeps_the_ten_gibibyte_safety_reserve_and_rejects_overflow() {
        assert!(capacity_is_sufficient(110, 10, 100));
        assert!(!capacity_is_sufficient(109, 10, 100));
        assert!(!capacity_is_sufficient(u64::MAX, u64::MAX, 1));
    }

    #[test]
    fn rejects_a_file_name_that_does_not_match_the_scanned_relative_path() {
        let source = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let file_name = "TX01_MIC001_20260809_010203.wav";
        fs::write(source.path().join(file_name), b"audio").unwrap();
        let mut candidate = observed(file_name, 5);
        candidate.file_name = "../../escape.wav".to_owned();

        assert!(matches!(
            plan_recording(
                source.path(),
                destination.path(),
                Transmitter::Tx01,
                candidate
            ),
            Err(CoreError::InvalidRequest)
        ));
    }
}
