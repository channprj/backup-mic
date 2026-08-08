use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use time::{OffsetDateTime, UtcOffset};

use crate::{
    clock::Clock,
    error::CoreError,
    filesystem::{is_hidden, is_safe_relative_path, modified_nanos},
    recording::{RecordingObservation, parse_recording_name},
    state::Transmitter,
};

pub const STABILITY_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanIssue {
    TransmitterPrefixMismatch,
    MalformedName,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScanResult {
    pub recordings: Vec<RecordingObservation>,
    pub issues: Vec<ScanIssue>,
}

pub fn scan_once(
    root: &Path,
    transmitter: Transmitter,
    local_offset: UtcOffset,
) -> Result<ScanResult, CoreError> {
    let canonical_root = fs::canonicalize(root).map_err(CoreError::CopyFailed)?;
    let mut result = ScanResult::default();
    scan_directory(
        &canonical_root,
        &canonical_root,
        transmitter,
        local_offset,
        &mut result,
    )?;
    result
        .recordings
        .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(result)
}

fn scan_directory(
    canonical_root: &Path,
    directory: &Path,
    transmitter: Transmitter,
    local_offset: UtcOffset,
    result: &mut ScanResult,
) -> Result<(), CoreError> {
    for entry in fs::read_dir(directory).map_err(CoreError::CopyFailed)? {
        let entry = entry.map_err(CoreError::CopyFailed)?;
        let path = entry.path();
        if is_hidden(&path) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(CoreError::CopyFailed)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            scan_directory(canonical_root, &path, transmitter, local_offset, result)?;
            continue;
        }
        if !metadata.is_file() || !has_wav_extension(&path) {
            continue;
        }
        let canonical_path = fs::canonicalize(&path).map_err(CoreError::CopyFailed)?;
        if !canonical_path.starts_with(canonical_root) {
            continue;
        }
        let relative_path = canonical_path
            .strip_prefix(canonical_root)
            .map_err(|_| invalid_path_error())?
            .to_path_buf();
        if !is_safe_relative_path(&relative_path) {
            continue;
        }
        let file_name = canonical_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(invalid_path_error)?
            .to_owned();
        let modified_nanos = modified_nanos(&metadata)?;
        let fallback_date = OffsetDateTime::from_unix_timestamp_nanos(modified_nanos)
            .map_err(|_| invalid_path_error())?
            .to_offset(local_offset)
            .date();
        let parsed_name = parse_recording_name(&file_name, fallback_date);
        if parsed_name.used_fallback_date {
            result.issues.push(ScanIssue::MalformedName);
        }
        if parsed_name
            .transmitter_hint
            .is_some_and(|hint| hint != transmitter)
        {
            result.issues.push(ScanIssue::TransmitterPrefixMismatch);
        }
        result.recordings.push(RecordingObservation {
            relative_path,
            file_name,
            size: metadata.len(),
            modified_nanos,
            parsed_name,
        });
    }
    Ok(())
}

fn has_wav_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
}

fn invalid_path_error() -> CoreError {
    CoreError::CopyFailed(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "invalid source path metadata",
    ))
}

pub fn scan_stable(
    root: &Path,
    transmitter: Transmitter,
    local_offset: UtcOffset,
    clock: &dyn Clock,
) -> Result<ScanResult, CoreError> {
    let first = scan_once(root, transmitter, local_offset)?;
    clock.sleep(STABILITY_INTERVAL);
    let mut second = scan_once(root, transmitter, local_offset)?;
    let first_by_path: HashMap<PathBuf, (u64, i128)> = first
        .recordings
        .into_iter()
        .map(|recording| {
            (
                recording.relative_path,
                (recording.size, recording.modified_nanos),
            )
        })
        .collect();
    second.recordings.retain(|recording| {
        first_by_path.get(&recording.relative_path)
            == Some(&(recording.size, recording.modified_nanos))
    });
    Ok(second)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use tempfile::tempdir;

    use super::*;

    #[derive(Default)]
    struct FakeClock(AtomicUsize);

    impl Clock for FakeClock {
        fn sleep(&self, duration: Duration) {
            assert_eq!(duration, STABILITY_INTERVAL);
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct MutatingClock {
        path: PathBuf,
    }

    impl Clock for MutatingClock {
        fn sleep(&self, duration: Duration) {
            assert_eq!(duration, STABILITY_INTERVAL);
            fs::write(&self.path, b"audio changed during observation").unwrap();
        }
    }

    #[test]
    fn discovers_only_visible_regular_wav_files() {
        let source = tempdir().unwrap();
        fs::create_dir(source.path().join("session")).unwrap();
        fs::write(
            source
                .path()
                .join("session/TX01_MIC001_20260809_010203_edit.wav"),
            b"audio",
        )
        .unwrap();
        fs::write(source.path().join("session/notes.txt"), b"ignore").unwrap();
        fs::create_dir(source.path().join(".Trashes")).unwrap();
        fs::write(
            source
                .path()
                .join(".Trashes/TX01_MIC002_20260809_010204.wav"),
            b"trash",
        )
        .unwrap();

        let result = scan_once(source.path(), Transmitter::Tx01, UtcOffset::UTC).unwrap();
        assert_eq!(result.recordings.len(), 1);
        assert_eq!(
            result.recordings[0].relative_path,
            Path::new("session/TX01_MIC001_20260809_010203_edit.wav")
        );
    }

    #[test]
    fn stable_scan_waits_for_two_observations() {
        let source = tempdir().unwrap();
        fs::write(
            source.path().join("TX02_MIC001_20260809_010203.wav"),
            b"audio",
        )
        .unwrap();
        let clock = FakeClock::default();
        let result = scan_stable(source.path(), Transmitter::Tx02, UtcOffset::UTC, &clock).unwrap();
        assert_eq!(clock.0.load(Ordering::SeqCst), 1);
        assert_eq!(result.recordings.len(), 1);
    }

    #[test]
    fn changing_recordings_are_not_stable() {
        let source = tempdir().unwrap();
        let path = source.path().join("TX01_MIC001_20260809_010203.wav");
        fs::write(&path, b"audio").unwrap();
        let result = scan_stable(
            source.path(),
            Transmitter::Tx01,
            UtcOffset::UTC,
            &MutatingClock { path },
        )
        .unwrap();
        assert!(result.recordings.is_empty());
    }

    #[test]
    fn conflicting_transmitter_prefix_blocks_a_clean_snapshot() {
        let source = tempdir().unwrap();
        fs::write(
            source.path().join("TX02_MIC001_20260809_010203.wav"),
            b"audio",
        )
        .unwrap();
        let result = scan_once(source.path(), Transmitter::Tx01, UtcOffset::UTC).unwrap();
        assert_eq!(result.recordings.len(), 1);
        assert!(
            result
                .issues
                .contains(&ScanIssue::TransmitterPrefixMismatch)
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_never_recording_candidates() {
        use std::os::unix::fs::symlink;

        let source = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let target = outside.path().join("TX01_MIC001_20260809_010203.wav");
        fs::write(&target, b"outside audio").unwrap();
        symlink(&target, source.path().join("linked.wav")).unwrap();
        let result = scan_once(source.path(), Transmitter::Tx01, UtcOffset::UTC).unwrap();
        assert!(result.recordings.is_empty());
    }
}
