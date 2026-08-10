use backup_core::{
    error::PublicError,
    events::ActivityEntry,
    state::{BackupPhase, CurrentStage, DeletionPhase, Progress, Transmitter},
};
use serde::{Deserialize, Serialize};

use crate::pairing::PairingCandidateSummary;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressDto {
    pub percent: u8,
    pub copied_bytes: u64,
    pub bytes_requiring_copy: u64,
    pub verified_files: u64,
    pub total_files: u64,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransmitterSnapshotDto {
    pub transmitter: Transmitter,
    pub mounted: bool,
    pub phase: BackupPhase,
    pub progress: ProgressDto,
    #[serde(rename = "retirement_outcome")]
    pub deletion_phase: DeletionPhase,
    pub deletion_ready: bool,
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
    NeedsPairing,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSettingsDto {
    pub automatic_backup: bool,
    pub m4a_conversion: bool,
    pub automatic_trash: bool,
    pub autostart: bool,
}

impl Default for BackupSettingsDto {
    fn default() -> Self {
        Self {
            automatic_backup: true,
            m4a_conversion: true,
            automatic_trash: false,
            autostart: false,
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
pub struct AppSnapshotDto {
    pub revision: u64,
    pub phase: BackupPhase,
    pub message_code: String,
    pub overall_progress: ProgressDto,
    pub transmitters: Vec<TransmitterSnapshotDto>,
    pub current_stage: Option<CurrentStage>,
    pub failure_stage: Option<CurrentStage>,
    pub setting_applies_next_run: bool,
    pub current_item_ordinal: Option<u64>,
    pub last_success_at: Option<String>,
    pub artifact_format: ArtifactFormatDto,
    pub retirement_mode: RetirementModeDto,
    pub current_log_available: bool,
    #[serde(default)]
    pub settings: BackupSettingsDto,
    pub notification_status: NotificationStatusDto,
    pub setup_state: SetupStateDto,
    pub pairing_candidates: Vec<PairingCandidateSummary>,
    pub recent_activity: Vec<ActivityEntry>,
    pub error: Option<PublicError>,
}

impl AppSnapshotDto {
    pub fn with_activity(mut self, recent_activity: Vec<ActivityEntry>) -> Self {
        self.recent_activity = recent_activity.into_iter().rev().take(8).collect();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashProposalSummaryDto {
    pub proposal_id: String,
    pub transmitter: Transmitter,
    pub session_count: u64,
    pub file_count: u64,
    pub byte_count: u64,
    pub destination_summary: String,
    pub expires_at: String,
}

#[cfg(test)]
mod tests {
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
            transmitters: Vec::new(),
            current_stage: None,
            failure_stage: None,
            setting_applies_next_run: false,
            current_item_ordinal: None,
            last_success_at: None,
            artifact_format: ArtifactFormatDto::M4a,
            retirement_mode: RetirementModeDto::Manual,
            current_log_available: false,
            settings: BackupSettingsDto::default(),
            notification_status: NotificationStatusDto::Unknown,
            setup_state: SetupStateDto::NeedsDestination,
            pairing_candidates: Vec::new(),
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
            "filename",
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
