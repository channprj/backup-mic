use std::{
    fs::{self, File, OpenOptions},
    io::{BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use sha2::{Digest, Sha256};
use tempfile::{NamedTempFile, TempPath};
use uuid::Uuid;

use crate::{
    artifact::{ConversionStatus, OutputFormat, RetirementStatus, VerifiedArtifact},
    destination::{DestinationDisposition, DestinationPlan, required_copy_bytes},
    error::CoreError,
    filesystem::{is_safe_relative_path, modified_nanos},
    hash::{FileDigest, HASH_BUFFER_BYTES, hash_reader_checked},
    ledger::{Ledger, VerifiedRecording},
    state::{Progress, Transmitter},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyFaultPoint {
    SourceMetadata,
    SourceOpen,
    SourceRead,
    PartialWrite,
    Flush,
    FileSync,
    SourceReread,
    DestinationHash,
    Persist,
    DirectorySync,
    LedgerCommit,
}

pub trait CopyFaults: Send + Sync {
    fn check(&self, point: CopyFaultPoint) -> Result<(), CoreError>;
}

#[derive(Debug, Default)]
pub struct NoCopyFaults;

impl CopyFaults for NoCopyFaults {
    fn check(&self, _point: CopyFaultPoint) -> Result<(), CoreError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn check(&self) -> Result<(), CoreError> {
        if self.0.load(Ordering::SeqCst) {
            return Err(CoreError::Cancelled);
        }
        Ok(())
    }
}

pub struct BackupItemContext<'a> {
    pub source_root: &'a Path,
    pub destination_root: &'a Path,
    pub transmitter: Transmitter,
    pub backup_run_id: &'a str,
    pub verified_at: &'a str,
}

pub fn progress_for_plans(plans: &[DestinationPlan]) -> Result<Progress, CoreError> {
    let bytes_requiring_copy = required_copy_bytes(plans)?;
    let total_work_units = plans.iter().try_fold(0_u64, |total, plan| {
        let item_work = match plan.disposition {
            DestinationDisposition::Copy => plan
                .source
                .size
                .checked_mul(2)
                .ok_or(CoreError::InvalidRequest)?,
            DestinationDisposition::Reuse => plan.source.size,
        };
        total
            .checked_add(item_work)
            .ok_or(CoreError::InvalidRequest)
    })?;
    Ok(Progress {
        total_work_units,
        bytes_requiring_copy,
        total_files: u64::try_from(plans.len()).map_err(|_| CoreError::InvalidRequest)?,
        ..Progress::default()
    })
}

pub fn execute_backup_item(
    context: &BackupItemContext<'_>,
    plan: &DestinationPlan,
    ledger: &mut Ledger,
    progress: &mut Progress,
    faults: &dyn CopyFaults,
    cancellation: &CancellationToken,
) -> Result<VerifiedRecording, CoreError> {
    execute_backup_item_observed(
        context,
        plan,
        ledger,
        progress,
        faults,
        cancellation,
        &mut |_, _| {},
    )
}

pub fn execute_backup_item_observed(
    context: &BackupItemContext<'_>,
    plan: &DestinationPlan,
    ledger: &mut Ledger,
    progress: &mut Progress,
    faults: &dyn CopyFaults,
    cancellation: &CancellationToken,
    observer: &mut dyn FnMut(&Progress, crate::state::CurrentStage),
) -> Result<VerifiedRecording, CoreError> {
    cancellation.check()?;
    if !is_safe_relative_path(&plan.source.relative_path)
        || !is_safe_relative_path(&plan.relative_destination)
    {
        return Err(CoreError::InvalidRequest);
    }
    let source_path = resolve_existing_regular(
        context.source_root,
        &plan.source.relative_path,
        CoreError::SourceChanged,
    )?;
    let destination_root = fs::canonicalize(context.destination_root)
        .map_err(|_| CoreError::DestinationUnavailable)?;
    let destination_path = match plan.disposition {
        DestinationDisposition::Copy => {
            prepare_destination_path(&destination_root, &plan.relative_destination)?
        }
        DestinationDisposition::Reuse => resolve_existing_regular(
            &destination_root,
            &plan.relative_destination,
            CoreError::DestinationUnavailable,
        )?,
    };
    let source_digest = match plan.disposition {
        DestinationDisposition::Copy => copy_and_verify(
            &source_path,
            &destination_path,
            plan,
            progress,
            faults,
            cancellation,
            observer,
        )?,
        DestinationDisposition::Reuse => verify_reused(
            &source_path,
            &destination_path,
            plan,
            progress,
            faults,
            cancellation,
            observer,
        )?,
    };
    let mut verified = VerifiedRecording {
        id: Uuid::new_v4().to_string(),
        transmitter: context.transmitter,
        source_relative_path: plan.source.relative_path.clone(),
        source_size: source_digest.size,
        source_mtime_ns: plan.source.modified_nanos,
        source_sha256: source_digest.sha256.clone(),
        artifact: VerifiedArtifact {
            relative_path: plan.relative_destination.clone(),
            format: OutputFormat::Wav,
            byte_count: source_digest.size,
            sha256: source_digest.sha256,
            audio: None,
        },
        conversion_status: ConversionStatus::NotRequired,
        conversion_error_code: None,
        retirement_status: RetirementStatus::Present,
        retired_session_relative_path: None,
        verified_at: context.verified_at.to_owned(),
        backup_run_id: context.backup_run_id.to_owned(),
    };
    faults.check(CopyFaultPoint::LedgerCommit)?;
    verified.id = ledger.commit_verified_recording(&verified)?;
    progress.record_verified_file();
    observer(progress, crate::state::CurrentStage::Sha256Verification);
    Ok(verified)
}

pub fn cleanup_owned_partials(destination_root: &Path) -> Result<usize, CoreError> {
    let mut removed = 0_usize;
    cleanup_directory(destination_root, &mut removed)?;
    Ok(removed)
}

pub fn ensure_capacity(
    destination_root: &Path,
    required_copy_bytes: u64,
    reserve_bytes: u64,
) -> Result<(), CoreError> {
    let available =
        fs2::available_space(destination_root).map_err(|_| CoreError::DestinationUnavailable)?;
    if !crate::destination::capacity_is_sufficient(available, required_copy_bytes, reserve_bytes) {
        return Err(CoreError::InsufficientCapacity);
    }
    Ok(())
}

fn copy_and_verify(
    source_path: &Path,
    destination_path: &Path,
    plan: &DestinationPlan,
    progress: &mut Progress,
    faults: &dyn CopyFaults,
    cancellation: &CancellationToken,
    observer: &mut dyn FnMut(&Progress, crate::state::CurrentStage),
) -> Result<FileDigest, CoreError> {
    let initial_metadata = checked_source_metadata(source_path, plan, faults)?;
    faults.check(CopyFaultPoint::SourceOpen)?;
    let mut source = BufReader::with_capacity(
        HASH_BUFFER_BYTES,
        File::open(source_path).map_err(CoreError::CopyFailed)?,
    );
    let parent = destination_path
        .parent()
        .ok_or(CoreError::DestinationUnavailable)?;
    fs::create_dir_all(parent).map_err(CoreError::CopyFailed)?;
    let mut partial = create_owned_partial(parent)?;
    let mut hasher = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    loop {
        cancellation.check()?;
        faults.check(CopyFaultPoint::SourceRead)?;
        let read = source.read(&mut buffer).map_err(CoreError::CopyFailed)?;
        if read == 0 {
            break;
        }
        faults.check(CopyFaultPoint::PartialWrite)?;
        partial
            .as_file_mut()
            .write_all(&buffer[..read])
            .map_err(CoreError::CopyFailed)?;
        hasher.update(&buffer[..read]);
        let read = u64::try_from(read).map_err(|_| CoreError::InvalidRequest)?;
        copied = copied.checked_add(read).ok_or(CoreError::InvalidRequest)?;
        progress.record_copy(read);
        observer(progress, crate::state::CurrentStage::Copy);
    }
    faults.check(CopyFaultPoint::Flush)?;
    partial
        .as_file_mut()
        .flush()
        .map_err(CoreError::CopyFailed)?;
    faults.check(CopyFaultPoint::FileSync)?;
    partial
        .as_file()
        .sync_all()
        .map_err(CoreError::SyncFailed)?;
    faults.check(CopyFaultPoint::SourceReread)?;
    let final_metadata = fs::symlink_metadata(source_path).map_err(CoreError::CopyFailed)?;
    if !final_metadata.file_type().is_file()
        || initial_metadata.len() != final_metadata.len()
        || modified_nanos(&initial_metadata)? != modified_nanos(&final_metadata)?
        || copied != plan.source.size
    {
        return Err(CoreError::SourceChanged);
    }
    let source_digest = FileDigest {
        size: copied,
        sha256: digest_hex(hasher.finalize()),
    };
    if source_digest.sha256 != plan.source_sha256 {
        return Err(CoreError::SourceChanged);
    }
    faults.check(CopyFaultPoint::DestinationHash)?;
    let destination_digest = hash_reader_checked(
        BufReader::with_capacity(
            HASH_BUFFER_BYTES,
            File::open(partial.path()).map_err(CoreError::CopyFailed)?,
        ),
        |bytes| {
            cancellation.check()?;
            progress.record_verification(bytes);
            observer(progress, crate::state::CurrentStage::Sha256Verification);
            Ok(())
        },
    )?;
    if destination_digest != source_digest {
        return Err(CoreError::HashMismatch);
    }
    faults.check(CopyFaultPoint::Persist)?;
    match partial.persist_noclobber(destination_path) {
        Ok(file) => drop(file),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = hash_reader_checked(
                BufReader::with_capacity(
                    HASH_BUFFER_BYTES,
                    File::open(destination_path).map_err(CoreError::CopyFailed)?,
                ),
                |_| cancellation.check(),
            )?;
            if existing != source_digest {
                return Err(CoreError::DestinationUnavailable);
            }
        }
        Err(error) => return Err(CoreError::CopyFailed(error.error)),
    }
    faults.check(CopyFaultPoint::DirectorySync)?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(CoreError::SyncFailed)?;
    Ok(source_digest)
}

fn verify_reused(
    source_path: &Path,
    destination_path: &Path,
    plan: &DestinationPlan,
    progress: &mut Progress,
    faults: &dyn CopyFaults,
    cancellation: &CancellationToken,
    observer: &mut dyn FnMut(&Progress, crate::state::CurrentStage),
) -> Result<FileDigest, CoreError> {
    checked_source_metadata(source_path, plan, faults)?;
    faults.check(CopyFaultPoint::SourceOpen)?;
    let source_digest = hash_reader_checked(
        BufReader::with_capacity(
            HASH_BUFFER_BYTES,
            File::open(source_path).map_err(CoreError::CopyFailed)?,
        ),
        |_bytes| {
            cancellation.check()?;
            faults.check(CopyFaultPoint::SourceRead)?;
            Ok(())
        },
    )?;
    faults.check(CopyFaultPoint::SourceReread)?;
    let final_metadata = fs::symlink_metadata(source_path).map_err(CoreError::CopyFailed)?;
    if !final_metadata.file_type().is_file()
        || final_metadata.len() != plan.source.size
        || modified_nanos(&final_metadata)? != plan.source.modified_nanos
        || source_digest.sha256 != plan.source_sha256
    {
        return Err(CoreError::SourceChanged);
    }
    let destination_metadata =
        fs::symlink_metadata(destination_path).map_err(|_| CoreError::DestinationUnavailable)?;
    if !destination_metadata.file_type().is_file() || destination_metadata.len() != plan.source.size
    {
        return Err(CoreError::DestinationUnavailable);
    }
    faults.check(CopyFaultPoint::DestinationHash)?;
    let destination_digest = hash_reader_checked(
        BufReader::with_capacity(
            HASH_BUFFER_BYTES,
            File::open(destination_path).map_err(CoreError::CopyFailed)?,
        ),
        |bytes| {
            cancellation.check()?;
            progress.record_verification(bytes);
            observer(progress, crate::state::CurrentStage::Sha256Verification);
            Ok(())
        },
    )?;
    if destination_digest != source_digest {
        return Err(CoreError::HashMismatch);
    }
    Ok(source_digest)
}

fn checked_source_metadata(
    source_path: &Path,
    plan: &DestinationPlan,
    faults: &dyn CopyFaults,
) -> Result<fs::Metadata, CoreError> {
    faults.check(CopyFaultPoint::SourceMetadata)?;
    let metadata = fs::symlink_metadata(source_path).map_err(CoreError::CopyFailed)?;
    if !metadata.file_type().is_file()
        || metadata.len() != plan.source.size
        || modified_nanos(&metadata)? != plan.source.modified_nanos
    {
        return Err(CoreError::SourceChanged);
    }
    Ok(metadata)
}

fn create_owned_partial(directory: &Path) -> Result<NamedTempFile, CoreError> {
    for _ in 0..8 {
        let path = directory.join(format!(".{}.partial", Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                let temp_path = TempPath::try_from_path(path).map_err(CoreError::CopyFailed)?;
                return Ok(NamedTempFile::from_parts(file, temp_path));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(CoreError::CopyFailed(error)),
        }
    }
    Err(CoreError::DestinationUnavailable)
}

fn resolve_existing_regular(
    root: &Path,
    relative_path: &Path,
    failure: CoreError,
) -> Result<PathBuf, CoreError> {
    let canonical_root = fs::canonicalize(root).map_err(|_| same_error(&failure))?;
    let mut current = canonical_root.clone();
    for component in relative_path.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(same_error(&failure));
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|_| same_error(&failure))?;
        if metadata.file_type().is_symlink() {
            return Err(same_error(&failure));
        }
    }
    let metadata = fs::symlink_metadata(&current).map_err(|_| same_error(&failure))?;
    if !metadata.file_type().is_file() {
        return Err(same_error(&failure));
    }
    let canonical_path = fs::canonicalize(&current).map_err(|_| same_error(&failure))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(same_error(&failure));
    }
    Ok(canonical_path)
}

fn prepare_destination_path(
    canonical_root: &Path,
    relative_path: &Path,
) -> Result<PathBuf, CoreError> {
    let parent_relative = relative_path
        .parent()
        .ok_or(CoreError::DestinationUnavailable)?;
    let mut current = canonical_root.to_path_buf();
    for component in parent_relative.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(CoreError::DestinationUnavailable);
        };
        current.push(component);
        match fs::create_dir(&current) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(CoreError::CopyFailed(error)),
        }
        let metadata = fs::symlink_metadata(&current).map_err(CoreError::CopyFailed)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(CoreError::DestinationUnavailable);
        }
        let canonical_current =
            fs::canonicalize(&current).map_err(|_| CoreError::DestinationUnavailable)?;
        if !canonical_current.starts_with(canonical_root) {
            return Err(CoreError::DestinationUnavailable);
        }
        current = canonical_current;
    }
    let file_name = relative_path
        .file_name()
        .ok_or(CoreError::DestinationUnavailable)?;
    Ok(current.join(file_name))
}

fn same_error(error: &CoreError) -> CoreError {
    match error {
        CoreError::SourceChanged => CoreError::SourceChanged,
        CoreError::DestinationUnavailable => CoreError::DestinationUnavailable,
        _ => CoreError::InvalidRequest,
    }
}

fn cleanup_directory(directory: &Path, removed: &mut usize) -> Result<(), CoreError> {
    for entry in fs::read_dir(directory).map_err(CoreError::CopyFailed)? {
        let entry = entry.map_err(CoreError::CopyFailed)?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(CoreError::CopyFailed)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            cleanup_directory(&path, removed)?;
        } else if metadata.is_file() && is_owned_partial(&path) {
            fs::remove_file(&path).map_err(CoreError::CopyFailed)?;
            *removed = removed.checked_add(1).ok_or(CoreError::InvalidRequest)?;
        }
    }
    Ok(())
}

fn is_owned_partial(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(hidden_name) = name.strip_prefix('.') else {
        return false;
    };
    if hidden_name
        .strip_suffix(".partial")
        .is_some_and(|id| Uuid::parse_str(id).is_ok())
    {
        return true;
    }
    hidden_name
        .rsplit_once(".m4a.part-")
        .is_some_and(|(stem, id)| !stem.is_empty() && Uuid::parse_str(id).is_ok())
}

fn digest_hex(digest: impl AsRef<[u8]>) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(64);
    for byte in digest.as_ref() {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, sync::Mutex};

    use tempfile::tempdir;
    use time::macros::date;

    use crate::{
        destination::{DestinationDisposition, plan_recording},
        filesystem::modified_nanos,
        recording::{ParsedRecordingName, RecordingObservation},
    };

    use super::*;

    struct FailOnce(Mutex<Option<CopyFaultPoint>>);

    impl CopyFaults for FailOnce {
        fn check(&self, point: CopyFaultPoint) -> Result<(), CoreError> {
            let mut target = self.0.lock().unwrap();
            if *target == Some(point) {
                *target = None;
                return Err(CoreError::CopyFailed(std::io::Error::other(
                    "injected copy failure",
                )));
            }
            Ok(())
        }
    }

    struct MutateOnce {
        point: CopyFaultPoint,
        path: PathBuf,
        mutated: Mutex<bool>,
    }

    impl CopyFaults for MutateOnce {
        fn check(&self, point: CopyFaultPoint) -> Result<(), CoreError> {
            let mut mutated = self.mutated.lock().unwrap();
            if point == self.point && !*mutated {
                fs::write(&self.path, b"source changed while backup was running").unwrap();
                *mutated = true;
            }
            Ok(())
        }
    }

    fn observation(source_root: &Path, file_name: &str) -> RecordingObservation {
        let metadata = fs::metadata(source_root.join(file_name)).unwrap();
        RecordingObservation {
            relative_path: PathBuf::from(file_name),
            file_name: file_name.to_owned(),
            size: metadata.len(),
            modified_nanos: modified_nanos(&metadata).unwrap(),
            parsed_name: ParsedRecordingName {
                transmitter_hint: Some(Transmitter::Tx01),
                destination_date: date!(2026 - 08 - 09),
                used_fallback_date: false,
            },
        }
    }

    fn setup() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        tempfile::TempDir,
        DestinationPlan,
    ) {
        let source = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let state = tempdir().unwrap();
        let file_name = "TX01_MIC001_20260809_010203.wav";
        fs::write(
            source.path().join(file_name),
            vec![0x5a; 2 * 1024 * 1024 + 17],
        )
        .unwrap();
        let plan = plan_recording(
            source.path(),
            destination.path(),
            Transmitter::Tx01,
            observation(source.path(), file_name),
        )
        .unwrap();
        (source, destination, state, plan)
    }

    fn execute(
        source: &Path,
        destination: &Path,
        state: &Path,
        plan: &DestinationPlan,
        faults: &dyn CopyFaults,
        cancellation: &CancellationToken,
    ) -> Result<(VerifiedRecording, Ledger, Progress), CoreError> {
        let mut ledger = Ledger::open(state.join("ledger.sqlite3"))?;
        let run_id = Uuid::new_v4().to_string();
        ledger.begin_backup_run(&run_id, "2026-08-09T00:00:00Z", plan.source.size)?;
        let mut progress = progress_for_plans(std::slice::from_ref(plan))?;
        let verified = execute_backup_item(
            &BackupItemContext {
                source_root: source,
                destination_root: destination,
                transmitter: Transmitter::Tx01,
                backup_run_id: &run_id,
                verified_at: "2026-08-09T00:01:00Z",
            },
            plan,
            &mut ledger,
            &mut progress,
            faults,
            cancellation,
        )?;
        Ok((verified, ledger, progress))
    }

    #[test]
    fn streams_verifies_persists_and_commits_without_removing_the_source() {
        let (source, destination, state, plan) = setup();
        let (verified, ledger, progress) = execute(
            source.path(),
            destination.path(),
            state.path(),
            &plan,
            &NoCopyFaults,
            &CancellationToken::default(),
        )
        .unwrap();

        assert!(source.path().join(&plan.source.relative_path).exists());
        assert_eq!(
            fs::read(destination.path().join(&verified.artifact.relative_path)).unwrap(),
            fs::read(source.path().join(&plan.source.relative_path)).unwrap()
        );
        assert_eq!(verified.source_sha256, verified.artifact.sha256);
        assert_eq!(ledger.verified_recording_count().unwrap(), 1);
        assert_eq!(progress.percent(), 100);
        assert_eq!(progress.verified_files, 1);
    }

    #[test]
    fn every_injected_copy_boundary_preserves_the_source() {
        for point in [
            CopyFaultPoint::SourceMetadata,
            CopyFaultPoint::SourceOpen,
            CopyFaultPoint::SourceRead,
            CopyFaultPoint::PartialWrite,
            CopyFaultPoint::Flush,
            CopyFaultPoint::FileSync,
            CopyFaultPoint::SourceReread,
            CopyFaultPoint::DestinationHash,
            CopyFaultPoint::Persist,
            CopyFaultPoint::DirectorySync,
            CopyFaultPoint::LedgerCommit,
        ] {
            let (source, destination, state, plan) = setup();
            let result = execute(
                source.path(),
                destination.path(),
                state.path(),
                &plan,
                &FailOnce(Mutex::new(Some(point))),
                &CancellationToken::default(),
            );
            assert!(result.is_err(), "{point:?} must fail");
            assert!(
                source.path().join(&plan.source.relative_path).exists(),
                "{point:?} removed the source"
            );
            let ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
            assert_eq!(ledger.verified_recording_count().unwrap(), 0);
        }
    }

    #[test]
    fn a_crash_after_persist_recovers_by_reusing_and_committing_the_final_file() {
        let (source, destination, state, plan) = setup();
        let failed = execute(
            source.path(),
            destination.path(),
            state.path(),
            &plan,
            &FailOnce(Mutex::new(Some(CopyFaultPoint::LedgerCommit))),
            &CancellationToken::default(),
        );
        assert!(failed.is_err());
        assert!(destination.path().join(&plan.relative_destination).exists());

        let recovered_plan = plan_recording(
            source.path(),
            destination.path(),
            Transmitter::Tx01,
            observation(source.path(), &plan.source.file_name),
        )
        .unwrap();
        assert_eq!(recovered_plan.disposition, DestinationDisposition::Reuse);
        let (_, ledger, _) = execute(
            source.path(),
            destination.path(),
            state.path(),
            &recovered_plan,
            &NoCopyFaults,
            &CancellationToken::default(),
        )
        .unwrap();
        assert_eq!(ledger.verified_recording_count().unwrap(), 1);
    }

    #[test]
    fn cleanup_removes_only_owned_partial_names() {
        let destination = tempdir().unwrap();
        fs::create_dir(destination.path().join("nested")).unwrap();
        fs::write(
            destination
                .path()
                .join("nested/.550e8400-e29b-41d4-a716-446655440000.partial"),
            b"partial",
        )
        .unwrap();
        fs::write(
            destination.path().join("nested/.someone-else.partial"),
            b"keep",
        )
        .unwrap();
        fs::write(
            destination
                .path()
                .join("nested/.recording.m4a.part-550e8400-e29b-41d4-a716-446655440000"),
            b"conversion partial",
        )
        .unwrap();

        assert_eq!(cleanup_owned_partials(destination.path()).unwrap(), 2);
        assert!(
            destination
                .path()
                .join("nested/.someone-else.partial")
                .exists()
        );
    }

    #[test]
    fn cancellation_and_source_mutation_fail_closed() {
        let (source, destination, state, plan) = setup();
        let cancellation = CancellationToken::default();
        cancellation.cancel();
        let cancelled = execute(
            source.path(),
            destination.path(),
            state.path(),
            &plan,
            &NoCopyFaults,
            &cancellation,
        );
        assert!(matches!(cancelled, Err(CoreError::Cancelled)));
        assert!(source.path().join(&plan.source.relative_path).exists());

        let (source, destination, state, plan) = setup();
        let source_path = source.path().join(&plan.source.relative_path);
        let changed = execute(
            source.path(),
            destination.path(),
            state.path(),
            &plan,
            &MutateOnce {
                point: CopyFaultPoint::SourceReread,
                path: source_path.clone(),
                mutated: Mutex::new(false),
            },
            &CancellationToken::default(),
        );
        assert!(matches!(changed, Err(CoreError::SourceChanged)));
        assert!(source_path.exists());
        assert!(!destination.path().join(&plan.relative_destination).exists());
    }

    #[test]
    fn a_racing_destination_is_never_overwritten() {
        let (source, destination, state, plan) = setup();
        let final_path = destination.path().join(&plan.relative_destination);
        fs::create_dir_all(final_path.parent().unwrap()).unwrap();
        fs::write(&final_path, b"racing writer wins this name").unwrap();

        let result = execute(
            source.path(),
            destination.path(),
            state.path(),
            &plan,
            &NoCopyFaults,
            &CancellationToken::default(),
        );
        assert!(matches!(result, Err(CoreError::DestinationUnavailable)));
        assert_eq!(
            fs::read(&final_path).unwrap(),
            b"racing writer wins this name"
        );
        assert!(source.path().join(&plan.source.relative_path).exists());
    }

    #[test]
    fn progress_counts_copy_plus_verify_but_only_verify_for_reuse() {
        let (source, destination, _state, copy) = setup();
        let copy_progress = progress_for_plans(std::slice::from_ref(&copy)).unwrap();
        assert_eq!(copy_progress.total_work_units, copy.source.size * 2);
        assert_eq!(copy_progress.bytes_requiring_copy, copy.source.size);

        let final_path = destination.path().join(&copy.relative_destination);
        fs::create_dir_all(final_path.parent().unwrap()).unwrap();
        fs::copy(source.path().join(&copy.source.relative_path), &final_path).unwrap();
        let reused = plan_recording(
            source.path(),
            destination.path(),
            Transmitter::Tx01,
            observation(source.path(), &copy.source.file_name),
        )
        .unwrap();
        assert_eq!(reused.disposition, DestinationDisposition::Reuse);
        let reused_progress = progress_for_plans(&[reused]).unwrap();
        assert_eq!(reused_progress.total_work_units, copy.source.size);
        assert_eq!(reused_progress.bytes_requiring_copy, 0);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_swaps_after_planning_never_escape_source_or_destination_roots() {
        use std::os::unix::fs::symlink;

        let (source, destination, state, plan) = setup();
        let outside = tempdir().unwrap();
        let source_path = source.path().join(&plan.source.relative_path);
        fs::remove_file(&source_path).unwrap();
        let outside_source = outside.path().join("outside.wav");
        fs::write(&outside_source, b"must remain untouched").unwrap();
        symlink(&outside_source, &source_path).unwrap();
        let result = execute(
            source.path(),
            destination.path(),
            state.path(),
            &plan,
            &NoCopyFaults,
            &CancellationToken::default(),
        );
        assert!(matches!(result, Err(CoreError::SourceChanged)));
        assert_eq!(fs::read(&outside_source).unwrap(), b"must remain untouched");

        let (source, destination, state, plan) = setup();
        let outside_destination = tempdir().unwrap();
        symlink(outside_destination.path(), destination.path().join("2026")).unwrap();
        let result = execute(
            source.path(),
            destination.path(),
            state.path(),
            &plan,
            &NoCopyFaults,
            &CancellationToken::default(),
        );
        assert!(matches!(result, Err(CoreError::DestinationUnavailable)));
        assert_eq!(fs::read_dir(outside_destination.path()).unwrap().count(), 0);
        assert!(source.path().join(&plan.source.relative_path).exists());
    }
}
