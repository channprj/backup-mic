use globset::{GlobBuilder, GlobMatcher};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::CoreError;

pub const MAX_RULE_TEXT_CHARS: usize = 64;
pub const MAX_GLOB_CHARS: usize = 256;
pub const MAX_PATTERNS_PER_GROUP: usize = 32;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuleId(String);

impl RuleId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().hyphenated().to_string())
    }

    pub fn parse(value: &str) -> Result<Self, CoreError> {
        Uuid::parse_str(value)
            .map(|uuid| Self(uuid.hyphenated().to_string()))
            .map_err(|_| CoreError::InvalidRule)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for RuleId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilenameProfile {
    Preserve,
    DjiTxShort,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceConstraintProfile {
    GenericExternal,
    DjiMicMini2s,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BackupRuleDraft {
    pub id: Option<RuleId>,
    pub name: String,
    pub archive_directory_name: String,
    pub enabled: bool,
    pub volume_name_glob: String,
    pub required_path_globs: Vec<String>,
    pub backup_file_globs: Vec<String>,
    pub session_directory_globs: Vec<String>,
    pub filename_prefix: String,
    pub filename_suffix: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BackupRule {
    pub id: RuleId,
    pub name: String,
    pub archive_directory_name: String,
    pub enabled: bool,
    pub volume_name_glob: String,
    pub required_path_globs: Vec<String>,
    pub backup_file_globs: Vec<String>,
    pub session_directory_globs: Vec<String>,
    pub filename_prefix: String,
    pub filename_suffix: String,
    pub filename_profile: FilenameProfile,
    pub device_constraint_profile: DeviceConstraintProfile,
    pub preset_kind: Option<String>,
    pub preset_revision: Option<u32>,
    pub archive_directory_locked: bool,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub struct CompiledBackupRule {
    pub rule: BackupRule,
    volume_matcher: GlobMatcher,
    required_matchers: Vec<GlobMatcher>,
    backup_matchers: Vec<GlobMatcher>,
    session_matchers: Vec<GlobMatcher>,
}

impl CompiledBackupRule {
    pub fn matches_volume_name(&self, value: &str) -> bool {
        self.volume_matcher.is_match(normalize_separators(value))
    }

    pub fn required_paths_match<I, S>(&self, paths: I) -> bool
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let paths = paths
            .into_iter()
            .map(|path| normalize_separators(path.as_ref()))
            .collect::<Vec<_>>();

        self.required_matchers
            .iter()
            .all(|matcher| paths.iter().any(|path| matcher.is_match(path)))
    }

    pub fn selects_backup_file(&self, path: &str) -> bool {
        let path = normalize_separators(path);
        self.backup_matchers
            .iter()
            .any(|matcher| matcher.is_match(&path))
    }

    pub fn matches_session_directory(&self, path: &str) -> bool {
        let path = normalize_separators(path);
        self.session_matchers
            .iter()
            .any(|matcher| matcher.is_match(&path))
    }
}

pub fn validate_rule(draft: BackupRuleDraft) -> Result<BackupRuleDraft, CoreError> {
    validate_required_component(&draft.name)?;
    validate_required_component(&draft.archive_directory_name)?;
    validate_optional_component(&draft.filename_prefix)?;
    validate_optional_component(&draft.filename_suffix)?;

    validate_pattern_group(std::slice::from_ref(&draft.volume_name_glob), true)?;
    validate_pattern_group(&draft.required_path_globs, false)?;
    validate_pattern_group(&draft.backup_file_globs, true)?;
    validate_pattern_group(&draft.session_directory_globs, false)?;

    Ok(draft)
}

pub fn compile_rule(rule: BackupRule) -> Result<CompiledBackupRule, CoreError> {
    validate_rule(BackupRuleDraft {
        id: Some(rule.id.clone()),
        name: rule.name.clone(),
        archive_directory_name: rule.archive_directory_name.clone(),
        enabled: rule.enabled,
        volume_name_glob: rule.volume_name_glob.clone(),
        required_path_globs: rule.required_path_globs.clone(),
        backup_file_globs: rule.backup_file_globs.clone(),
        session_directory_globs: rule.session_directory_globs.clone(),
        filename_prefix: rule.filename_prefix.clone(),
        filename_suffix: rule.filename_suffix.clone(),
    })?;

    Ok(CompiledBackupRule {
        volume_matcher: compile_glob(&rule.volume_name_glob)?,
        required_matchers: compile_globs(&rule.required_path_globs)?,
        backup_matchers: compile_globs(&rule.backup_file_globs)?,
        session_matchers: compile_globs(&rule.session_directory_globs)?,
        rule,
    })
}

pub fn apply_filename_profile(profile: FilenameProfile, stem: &str) -> String {
    match profile {
        FilenameProfile::Preserve => stem.to_owned(),
        FilenameProfile::DjiTxShort => {
            let prefix = stem.get(..5);
            match prefix {
                Some(value) if value.eq_ignore_ascii_case("TX01_") => format!("T01_{}", &stem[5..]),
                Some(value) if value.eq_ignore_ascii_case("TX02_") => format!("T02_{}", &stem[5..]),
                _ => stem.to_owned(),
            }
        }
    }
}

pub fn destination_stem(rule: &BackupRule, source_stem: &str) -> Result<String, CoreError> {
    let profiled_stem = apply_filename_profile(rule.filename_profile, source_stem);
    let stem = format!(
        "{}{}{}",
        rule.filename_prefix, profiled_stem, rule.filename_suffix
    );
    validate_required_component(&stem)?;
    Ok(stem)
}

fn validate_pattern_group(patterns: &[String], required: bool) -> Result<(), CoreError> {
    if patterns.len() > MAX_PATTERNS_PER_GROUP || (required && patterns.is_empty()) {
        return Err(CoreError::InvalidRule);
    }

    for pattern in patterns {
        if pattern.chars().count() > MAX_GLOB_CHARS
            || pattern.trim().is_empty()
            || pattern.chars().any(char::is_control)
        {
            return Err(CoreError::InvalidRule);
        }
        compile_glob(pattern)?;
    }

    Ok(())
}

fn validate_required_component(value: &str) -> Result<(), CoreError> {
    if value.trim().is_empty() || !is_safe_component(value) {
        return Err(CoreError::InvalidRule);
    }
    Ok(())
}

fn validate_optional_component(value: &str) -> Result<(), CoreError> {
    if value.is_empty() {
        return Ok(());
    }
    if value.trim().is_empty() || !is_safe_component(value) {
        return Err(CoreError::InvalidRule);
    }
    Ok(())
}

fn is_safe_component(value: &str) -> bool {
    value.chars().count() <= MAX_RULE_TEXT_CHARS
        && !value.starts_with('.')
        && value != "."
        && value != ".."
        && !value
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
}

fn compile_globs(patterns: &[String]) -> Result<Vec<GlobMatcher>, CoreError> {
    patterns
        .iter()
        .map(|pattern| compile_glob(pattern))
        .collect()
}

fn compile_glob(pattern: &str) -> Result<GlobMatcher, CoreError> {
    GlobBuilder::new(pattern)
        .case_insensitive(true)
        .literal_separator(true)
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|_| CoreError::InvalidRule)
}

fn normalize_separators(value: &str) -> String {
    value.replace('\\', "/")
}
