use std::{
    collections::HashMap,
    fs::{self, Metadata},
    path::{Path, PathBuf},
};

use time::{OffsetDateTime, UtcOffset};

use crate::{
    backup::CancellationToken,
    clock::Clock,
    error::CoreError,
    filesystem::{
        is_hidden, is_safe_additional_relative_path, is_safe_relative_path, modified_nanos,
    },
    recording::archive_date_for_filename,
    rule::CompiledBackupRule,
    scanner::STABILITY_INTERVAL,
};

pub const MAX_SCAN_DEPTH: usize = 32;
pub const MAX_VISITED_ENTRIES: usize = 100_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectedFileKind {
    RecordingWav,
    Companion,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleFileObservation {
    pub relative_path: PathBuf,
    pub file_name: String,
    pub kind: SelectedFileKind,
    pub session_relative_path: Option<PathBuf>,
    pub size: u64,
    pub modified_nanos: i128,
    pub archive_date: time::Date,
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct RuleScanFingerprint(Vec<(PathBuf, u64, i128)>);

impl RuleScanFingerprint {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleScanResult {
    pub files: Vec<RuleFileObservation>,
    pub fingerprint: RuleScanFingerprint,
    pub unsafe_session_count: u64,
}

#[cfg(unix)]
type FileIdentity = (u64, u64);

#[cfg(not(unix))]
type FileIdentity = (u64, i128);

#[derive(Clone, Debug)]
struct StableObservation {
    file: RuleFileObservation,
    identity: FileIdentity,
}

#[derive(Debug)]
struct InternalScan {
    files: Vec<StableObservation>,
    fingerprint: RuleScanFingerprint,
    unsafe_session_count: u64,
}

struct Walker<'a> {
    root: &'a Path,
    rule: &'a CompiledBackupRule,
    cancellation: &'a CancellationToken,
    local_offset: UtcOffset,
    visited: usize,
    max_depth: usize,
    max_entries: usize,
    required_candidates: Vec<String>,
    selected: Vec<StableObservation>,
    fingerprint_entries: Vec<(PathBuf, u64, i128)>,
    unsafe_session_count: u64,
}

pub fn scan_rule_once(
    root: &Path,
    rule: &CompiledBackupRule,
    local_offset: UtcOffset,
) -> Result<RuleScanResult, CoreError> {
    let cancellation = CancellationToken::default();
    let scan = scan_internal(
        root,
        rule,
        local_offset,
        MAX_SCAN_DEPTH,
        MAX_VISITED_ENTRIES,
        &cancellation,
    )?;
    Ok(public_result(scan))
}

pub fn scan_rule_stable(
    root: &Path,
    rule: &CompiledBackupRule,
    local_offset: UtcOffset,
    clock: &dyn Clock,
    cancellation: &CancellationToken,
) -> Result<RuleScanResult, CoreError> {
    cancellation.check()?;
    let first = scan_internal(
        root,
        rule,
        local_offset,
        MAX_SCAN_DEPTH,
        MAX_VISITED_ENTRIES,
        cancellation,
    )?;
    cancellation.check()?;
    clock.sleep(STABILITY_INTERVAL);
    cancellation.check()?;
    let mut second = scan_internal(
        root,
        rule,
        local_offset,
        MAX_SCAN_DEPTH,
        MAX_VISITED_ENTRIES,
        cancellation,
    )?;
    let first_by_path = first
        .files
        .into_iter()
        .map(|observation| {
            (
                observation.file.relative_path.clone(),
                (
                    observation.file.size,
                    observation.file.modified_nanos,
                    observation.identity,
                ),
            )
        })
        .collect::<HashMap<_, _>>();
    second.files.retain(|observation| {
        first_by_path.get(&observation.file.relative_path)
            == Some(&(
                observation.file.size,
                observation.file.modified_nanos,
                observation.identity,
            ))
    });
    cancellation.check()?;
    Ok(public_result(second))
}

fn scan_internal(
    root: &Path,
    rule: &CompiledBackupRule,
    local_offset: UtcOffset,
    max_depth: usize,
    max_entries: usize,
    cancellation: &CancellationToken,
) -> Result<InternalScan, CoreError> {
    cancellation.check()?;
    let root_metadata = fs::symlink_metadata(root).map_err(CoreError::CopyFailed)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(CoreError::InvalidRequest);
    }
    let canonical_root = fs::canonicalize(root).map_err(CoreError::CopyFailed)?;
    let mut walker = Walker {
        root: &canonical_root,
        rule,
        cancellation,
        local_offset,
        visited: 0,
        max_depth,
        max_entries,
        required_candidates: Vec::new(),
        selected: Vec::new(),
        fingerprint_entries: Vec::new(),
        unsafe_session_count: 0,
    };
    walker.walk_directory(&canonical_root, None)?;
    if !rule.required_paths_match(&walker.required_candidates) {
        walker.selected.clear();
    }
    walker
        .selected
        .sort_by(|left, right| left.file.relative_path.cmp(&right.file.relative_path));
    walker.fingerprint_entries.sort();
    Ok(InternalScan {
        files: walker.selected,
        fingerprint: RuleScanFingerprint(walker.fingerprint_entries),
        unsafe_session_count: walker.unsafe_session_count,
    })
}

impl Walker<'_> {
    fn walk_directory(
        &mut self,
        directory: &Path,
        active_session: Option<&Path>,
    ) -> Result<(), CoreError> {
        self.cancellation.check()?;
        for entry in fs::read_dir(directory).map_err(CoreError::CopyFailed)? {
            self.cancellation.check()?;
            let entry = entry.map_err(CoreError::CopyFailed)?;
            self.visited = self
                .visited
                .checked_add(1)
                .ok_or(CoreError::RuleScanLimit)?;
            if self.visited > self.max_entries {
                return Err(CoreError::RuleScanLimit);
            }
            let path = entry.path();
            let relative = path
                .strip_prefix(self.root)
                .map_err(|_| CoreError::InvalidRequest)?
                .to_path_buf();
            if relative.components().count() > self.max_depth {
                return Err(CoreError::RuleScanLimit);
            }
            let metadata = fs::symlink_metadata(&path).map_err(CoreError::CopyFailed)?;
            if active_session.is_none() && is_hidden(&path) {
                continue;
            }
            if metadata.file_type().is_symlink() {
                self.mark_unsafe(active_session);
                continue;
            }
            if metadata.is_dir() {
                self.visit_directory(&path, &relative, &metadata, active_session)?;
                continue;
            }
            if metadata.is_file() {
                self.visit_regular_file(&path, relative, &metadata, active_session)?;
                continue;
            }
            self.mark_unsafe(active_session);
        }
        Ok(())
    }

    fn visit_directory(
        &mut self,
        path: &Path,
        relative: &Path,
        metadata: &Metadata,
        active_session: Option<&Path>,
    ) -> Result<(), CoreError> {
        if active_session.is_some() {
            self.unsafe_session_count = self.unsafe_session_count.saturating_add(1);
            self.push_fingerprint(relative, metadata)?;
            return Ok(());
        }
        if !is_safe_relative_path(relative) {
            return Ok(());
        }
        let relative_text = relative.to_str().ok_or(CoreError::InvalidRequest)?;
        self.required_candidates.push(relative_text.to_owned());
        self.push_fingerprint(relative, metadata)?;
        let session = self
            .rule
            .matches_session_directory(relative_text)
            .then(|| relative.to_path_buf());
        self.walk_directory(path, session.as_deref())
    }

    fn visit_regular_file(
        &mut self,
        path: &Path,
        relative: PathBuf,
        metadata: &Metadata,
        active_session: Option<&Path>,
    ) -> Result<(), CoreError> {
        if !is_safe_additional_relative_path(&relative) {
            self.mark_unsafe(active_session);
            return Ok(());
        }
        let canonical = fs::canonicalize(path).map_err(CoreError::CopyFailed)?;
        if !canonical.starts_with(self.root) {
            self.mark_unsafe(active_session);
            return Ok(());
        }
        let relative_text = relative.to_str().ok_or(CoreError::InvalidRequest)?;
        self.required_candidates.push(relative_text.to_owned());
        self.push_fingerprint(&relative, metadata)?;
        if !self.rule.selects_backup_file(relative_text) {
            self.mark_unsafe(active_session);
            return Ok(());
        }
        let file_name = relative
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(CoreError::InvalidRequest)?
            .to_owned();
        let modified_nanos = modified_nanos(metadata)?;
        let fallback_date = OffsetDateTime::from_unix_timestamp_nanos(modified_nanos)
            .map_err(|_| CoreError::InvalidRequest)?
            .to_offset(self.local_offset)
            .date();
        let kind = if has_wav_extension(&relative) {
            SelectedFileKind::RecordingWav
        } else {
            SelectedFileKind::Companion
        };
        self.selected.push(StableObservation {
            file: RuleFileObservation {
                relative_path: relative,
                file_name: file_name.clone(),
                kind,
                session_relative_path: active_session.map(Path::to_path_buf),
                size: metadata.len(),
                modified_nanos,
                archive_date: archive_date_for_filename(
                    self.rule.rule.filename_profile,
                    &file_name,
                    fallback_date,
                ),
            },
            identity: file_identity(metadata)?,
        });
        Ok(())
    }

    fn push_fingerprint(&mut self, relative: &Path, metadata: &Metadata) -> Result<(), CoreError> {
        self.fingerprint_entries.push((
            relative.to_path_buf(),
            metadata.len(),
            modified_nanos(metadata)?,
        ));
        Ok(())
    }

    fn mark_unsafe(&mut self, active_session: Option<&Path>) {
        if active_session.is_some() {
            self.unsafe_session_count = self.unsafe_session_count.saturating_add(1);
        }
    }
}

fn public_result(scan: InternalScan) -> RuleScanResult {
    RuleScanResult {
        files: scan
            .files
            .into_iter()
            .map(|observation| observation.file)
            .collect(),
        fingerprint: scan.fingerprint,
        unsafe_session_count: scan.unsafe_session_count,
    }
}

fn has_wav_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))
}

#[cfg(unix)]
fn file_identity(metadata: &Metadata) -> Result<FileIdentity, CoreError> {
    use std::os::unix::fs::MetadataExt;

    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
fn file_identity(metadata: &Metadata) -> Result<FileIdentity, CoreError> {
    Ok((metadata.len(), modified_nanos(metadata)?))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::rule::{BackupRule, DeviceConstraintProfile, FilenameProfile, RuleId, compile_rule};

    use super::*;

    fn rule() -> CompiledBackupRule {
        compile_rule(BackupRule {
            id: RuleId::new(),
            name: "Test".to_owned(),
            archive_directory_name: "Test".to_owned(),
            enabled: true,
            volume_name_glob: "*".to_owned(),
            required_path_globs: Vec::new(),
            backup_file_globs: vec!["*.wav".to_owned()],
            session_directory_globs: Vec::new(),
            filename_prefix: String::new(),
            filename_suffix: String::new(),
            date_folder_layout: Default::default(),
            filename_profile: FilenameProfile::Preserve,
            device_constraint_profile: DeviceConstraintProfile::GenericExternal,
            preset_kind: None,
            preset_revision: None,
            archive_directory_locked: false,
            archived_at: None,
            created_at: "2026-08-10T00:00:00Z".to_owned(),
            updated_at: "2026-08-10T00:00:00Z".to_owned(),
        })
        .unwrap()
    }

    #[test]
    fn entry_limit_counts_every_directory_entry() {
        let source = tempdir().unwrap();
        for name in ["one.wav", "two.wav", "three.wav"] {
            fs::write(source.path().join(name), name).unwrap();
        }
        let error = scan_internal(
            source.path(),
            &rule(),
            UtcOffset::UTC,
            32,
            2,
            &CancellationToken::default(),
        )
        .unwrap_err();
        assert!(matches!(error, CoreError::RuleScanLimit));
    }
}
