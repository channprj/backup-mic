use std::path::Path;

use backup_core::{
    error::PublicError,
    events::ActivityEntry,
    rule::{BackupRule, DateFolderLayout},
    state::{BackupPhase, CurrentStage, DeletionPhase, Progress},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressDto {
    pub percent: u8,
    pub copied_bytes: u64,
    pub bytes_requiring_copy: u64,
    pub verified_files: u64,
    pub total_files: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupRuleDto {
    pub id: String,
    pub name: String,
    pub archive_directory_name: String,
    pub archive_directory_locked: bool,
    pub enabled: bool,
    pub volume_name_glob: String,
    pub required_path_globs: Vec<String>,
    pub backup_file_globs: Vec<String>,
    pub session_directory_globs: Vec<String>,
    pub filename_prefix: String,
    pub filename_suffix: String,
    #[serde(default)]
    pub date_folder_layout: DateFolderLayout,
    pub is_dji_preset: bool,
    pub archived: bool,
}

impl From<&BackupRule> for BackupRuleDto {
    fn from(rule: &BackupRule) -> Self {
        Self {
            id: rule.id.as_str().to_owned(),
            name: rule.name.clone(),
            archive_directory_name: rule.archive_directory_name.clone(),
            archive_directory_locked: rule.archive_directory_locked,
            enabled: rule.enabled,
            volume_name_glob: rule.volume_name_glob.clone(),
            required_path_globs: rule.required_path_globs.clone(),
            backup_file_globs: rule.backup_file_globs.clone(),
            session_directory_globs: rule.session_directory_globs.clone(),
            filename_prefix: rule.filename_prefix.clone(),
            filename_suffix: rule.filename_suffix.clone(),
            date_folder_layout: rule.date_folder_layout,
            is_dji_preset: rule.preset_kind.is_some(),
            archived: rule.archived_at.is_some(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSnapshotDto {
    pub source_id: String,
    pub rule_name: String,
    pub volume_name: String,
    pub legacy_slot: Option<String>,
    pub mounted: bool,
    pub phase: BackupPhase,
    pub progress: ProgressDto,
    pub retirement_outcome: DeletionPhase,
    pub deletion_ready: bool,
    pub error: Option<PublicError>,
}

impl SourceSnapshotDto {
    pub fn idle(
        source_id: String,
        rule_name: String,
        volume_name: String,
        legacy_slot: Option<String>,
    ) -> Self {
        Self {
            source_id,
            rule_name,
            volume_name: safe_label(&volume_name),
            legacy_slot,
            mounted: false,
            phase: BackupPhase::Idle,
            progress: ProgressDto::from(&Progress::default()),
            retirement_outcome: DeletionPhase::Inactive,
            deletion_ready: false,
            error: None,
        }
    }
}

pub(crate) fn safe_label(value: &str) -> String {
    let value = value
        .chars()
        .filter(|character| !character.is_control() && !matches!(character, '/' | '\\'))
        .take(128)
        .collect::<String>();
    if value.trim().is_empty() {
        "External Recorder".to_owned()
    } else {
        value
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleTestResultDto {
    pub matched_volumes: Vec<String>,
    pub matched_file_count: u64,
    pub conflict_rule_names: Vec<String>,
}

impl From<&Progress> for ProgressDto {
    fn from(progress: &Progress) -> Self {
        Self {
            percent: progress.percent(),
            copied_bytes: progress.copied_bytes,
            bytes_requiring_copy: progress.bytes_requiring_copy,
            verified_files: progress.verified_files,
            total_files: progress.total_files,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationStatusDto {
    Unknown,
    Granted,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupStateDto {
    NeedsDestination,
    NeedsSettingsReview,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSettingsDto {
    pub automatic_backup: bool,
    pub m4a_conversion: bool,
    pub automatic_trash: bool,
    pub autostart: bool,
    pub free_space_reserve_gib: u32,
    pub rescan_interval_seconds: u32,
}

impl Default for BackupSettingsDto {
    fn default() -> Self {
        let preferences = backup_core::preferences::BackupPreferences::default();
        Self {
            automatic_backup: preferences.automatic_backup,
            m4a_conversion: preferences.m4a_conversion,
            automatic_trash: preferences.automatic_trash,
            autostart: false,
            free_space_reserve_gib: preferences.free_space_reserve_gib,
            rescan_interval_seconds: preferences.rescan_interval_seconds,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactFormatDto {
    Wav,
    M4a,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetirementModeDto {
    Manual,
    Automatic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppSnapshotDto {
    pub revision: u64,
    pub phase: BackupPhase,
    pub message_code: String,
    pub overall_progress: ProgressDto,
    pub sources: Vec<SourceSnapshotDto>,
    pub backup_rules: Vec<BackupRuleDto>,
    pub current_stage: Option<CurrentStage>,
    pub failure_stage: Option<CurrentStage>,
    pub setting_applies_next_run: bool,
    pub current_item_ordinal: Option<u64>,
    pub last_success_at: Option<String>,
    pub artifact_format: ArtifactFormatDto,
    pub retirement_mode: RetirementModeDto,
    pub current_log_available: bool,
    #[serde(default)]
    pub destination_display: Option<String>,
    #[serde(default)]
    pub settings: BackupSettingsDto,
    pub notification_status: NotificationStatusDto,
    pub setup_state: SetupStateDto,
    pub recent_activity: Vec<ActivityEntry>,
    pub error: Option<PublicError>,
}

impl AppSnapshotDto {
    pub fn with_activity(mut self, recent_activity: Vec<ActivityEntry>) -> Self {
        self.recent_activity = recent_activity
            .into_iter()
            .rev()
            .take(8)
            .map(|mut activity| {
                if let Some(source_id) = &activity.source_id
                    && let Some(source) = self
                        .sources
                        .iter()
                        .find(|source| source.source_id == source_id.as_str())
                {
                    activity.source_label = Some(source.volume_name.clone());
                }
                activity
            })
            .collect();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrashProposalSummaryDto {
    pub proposal_id: String,
    pub source_id: String,
    pub source_label: String,
    pub session_count: u64,
    pub file_count: u64,
    pub byte_count: u64,
    pub destination_summary: String,
    pub expires_at: String,
}

pub(crate) fn destination_display_for(
    destination: &Path,
    home: Option<&Path>,
    configured: bool,
) -> Option<String> {
    if !configured {
        return None;
    }
    let raw = home
        .and_then(|home| destination.strip_prefix(home).ok())
        .map_or_else(
            || destination.to_string_lossy().into_owned(),
            |relative| {
                if relative.as_os_str().is_empty() {
                    "~".to_owned()
                } else {
                    format!("~/{}", relative.to_string_lossy())
                }
            },
        );
    let display = raw
        .chars()
        .filter(|character| !character.is_control())
        .take(2_048)
        .collect::<String>();
    (!display.is_empty()).then_some(display)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use backup_core::events::ActivitySeverity;

    use super::*;

    fn activity(index: u8) -> ActivityEntry {
        ActivityEntry {
            occurred_at: format!("2026-08-09T00:00:{index:02}Z"),
            code: "device_detected".to_owned(),
            source_id: Some(
                backup_core::source::SourceId::parse(backup_core::source::LEGACY_TX01_SOURCE_ID)
                    .unwrap(),
            ),
            source_label: Some("MIC_TX".to_owned()),
            count_value: None,
            byte_value: None,
            severity: ActivitySeverity::Info,
        }
    }

    fn empty_snapshot() -> AppSnapshotDto {
        AppSnapshotDto {
            revision: 1,
            phase: BackupPhase::Idle,
            message_code: "idle".to_owned(),
            overall_progress: ProgressDto::from(&Progress::default()),
            sources: Vec::new(),
            backup_rules: Vec::new(),
            current_stage: None,
            failure_stage: None,
            setting_applies_next_run: false,
            current_item_ordinal: None,
            last_success_at: None,
            artifact_format: ArtifactFormatDto::M4a,
            retirement_mode: RetirementModeDto::Manual,
            current_log_available: false,
            destination_display: None,
            settings: BackupSettingsDto::default(),
            notification_status: NotificationStatusDto::Unknown,
            setup_state: SetupStateDto::NeedsDestination,
            recent_activity: Vec::new(),
            error: None,
        }
    }

    #[test]
    fn settings_defaults_keep_automation_safe_and_use_m4a() {
        let settings = BackupSettingsDto::default();
        assert!(settings.automatic_backup);
        assert!(settings.m4a_conversion);
        assert!(!settings.automatic_trash);
        assert!(!settings.autostart);
    }

    #[test]
    fn destination_display_is_home_relative_or_external_absolute() {
        assert_eq!(
            destination_display_for(
                Path::new("/Users/example/Documents/Backup Mic"),
                Some(Path::new("/Users/example")),
                true,
            ),
            Some("~/Documents/Backup Mic".to_owned())
        );
        assert_eq!(
            destination_display_for(
                Path::new("/Volumes/Recorder Backups/Backup Mic"),
                Some(Path::new("/Users/example")),
                true,
            ),
            Some("/Volumes/Recorder Backups/Backup Mic".to_owned())
        );
        assert_eq!(
            destination_display_for(
                Path::new("/Users/example/Documents/Backup Mic"),
                Some(Path::new("/Users/example")),
                false,
            ),
            None
        );
        assert_eq!(
            destination_display_for(
                Path::new("/Volumes/Recorder\nBackups/Backup Mic"),
                Some(Path::new("/Users/example")),
                true,
            ),
            Some("/Volumes/RecorderBackups/Backup Mic".to_owned())
        );
        let long = format!("/Volumes/{}", "a".repeat(3_000));
        assert_eq!(
            destination_display_for(Path::new(&long), None, true)
                .unwrap()
                .chars()
                .count(),
            2_048
        );
    }

    #[test]
    fn snapshot_exposes_only_the_newest_eight_activity_entries() {
        let entries = (0..10).map(activity).collect();
        let snapshot = empty_snapshot().with_activity(entries);
        assert_eq!(snapshot.recent_activity.len(), 8);
        assert_eq!(
            snapshot.recent_activity[0].occurred_at,
            "2026-08-09T00:00:09Z"
        );
        assert_eq!(
            snapshot.recent_activity[7].occurred_at,
            "2026-08-09T00:00:02Z"
        );
    }

    #[test]
    fn serialized_snapshot_does_not_contain_sensitive_field_names() {
        let json = serde_json::to_string(&empty_snapshot()).expect("snapshot serializes");
        for forbidden in [
            "path",
            "uuid",
            "hash",
            "source_filename",
            "ledger_id",
            "recording_id",
        ] {
            assert!(
                !json.to_ascii_lowercase().contains(forbidden),
                "found {forbidden}"
            );
        }
    }

    #[test]
    fn source_revalidation_has_a_stable_public_stage_name() {
        assert_eq!(
            serde_json::to_value(CurrentStage::SourceRevalidation).unwrap(),
            "source_revalidation"
        );
    }
}
