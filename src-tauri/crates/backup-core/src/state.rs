use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Transmitter {
    #[serde(rename = "TX01")]
    Tx01,
    #[serde(rename = "TX02")]
    Tx02,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupPhase {
    Idle,
    Detecting,
    Scanning,
    CheckingCapacity,
    Copying,
    Verifying,
    CompletedDeletionPending,
    NothingNew,
    PartialFailure,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeletionPhase {
    Inactive,
    Preparing,
    AwaitingConfirmation,
    Revalidating,
    #[serde(rename = "moving_to_trash")]
    Deleting,
    #[serde(rename = "moved_to_trash")]
    Deleted,
    Refused,
    #[serde(rename = "partially_moved_to_trash")]
    PartiallyDeleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurrentStage {
    Copy,
    #[serde(rename = "source_verification")]
    Sha256Verification,
    Conversion,
    ArtifactVerification,
    Trash,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid transition from {from:?} to {to:?}")]
pub struct InvalidBackupTransition {
    pub from: BackupPhase,
    pub to: BackupPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid deletion transition from {from:?} to {to:?}")]
pub struct InvalidDeletionTransition {
    pub from: DeletionPhase,
    pub to: DeletionPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupStateMachine {
    phase: BackupPhase,
}

impl Default for BackupStateMachine {
    fn default() -> Self {
        Self {
            phase: BackupPhase::Idle,
        }
    }
}

impl BackupStateMachine {
    pub fn phase(&self) -> BackupPhase {
        self.phase
    }

    pub fn transition(&mut self, to: BackupPhase) -> Result<(), InvalidBackupTransition> {
        let from = self.phase;
        let allowed = from == to
            || matches!(
                (from, to),
                (BackupPhase::Idle, BackupPhase::Detecting)
                    | (
                        BackupPhase::Detecting,
                        BackupPhase::Scanning | BackupPhase::Error
                    )
                    | (
                        BackupPhase::Scanning,
                        BackupPhase::CheckingCapacity
                            | BackupPhase::NothingNew
                            | BackupPhase::PartialFailure
                            | BackupPhase::Error
                    )
                    | (
                        BackupPhase::CheckingCapacity,
                        BackupPhase::Copying
                            | BackupPhase::Verifying
                            | BackupPhase::PartialFailure
                            | BackupPhase::Error
                    )
                    | (
                        BackupPhase::Copying,
                        BackupPhase::Verifying | BackupPhase::PartialFailure | BackupPhase::Error
                    )
                    | (
                        BackupPhase::Verifying,
                        BackupPhase::CompletedDeletionPending
                            | BackupPhase::NothingNew
                            | BackupPhase::PartialFailure
                            | BackupPhase::Error
                    )
                    | (
                        BackupPhase::CompletedDeletionPending
                            | BackupPhase::NothingNew
                            | BackupPhase::PartialFailure
                            | BackupPhase::Error,
                        BackupPhase::Detecting | BackupPhase::Idle
                    )
            );
        if !allowed {
            return Err(InvalidBackupTransition { from, to });
        }
        self.phase = to;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionStateMachine {
    phase: DeletionPhase,
}

impl Default for DeletionStateMachine {
    fn default() -> Self {
        Self {
            phase: DeletionPhase::Inactive,
        }
    }
}

impl DeletionStateMachine {
    pub fn phase(&self) -> DeletionPhase {
        self.phase
    }

    pub fn transition(&mut self, to: DeletionPhase) -> Result<(), InvalidDeletionTransition> {
        let from = self.phase;
        let allowed = from == to
            || matches!(
                (from, to),
                (DeletionPhase::Inactive, DeletionPhase::Preparing)
                    | (
                        DeletionPhase::Preparing,
                        DeletionPhase::AwaitingConfirmation | DeletionPhase::Refused
                    )
                    | (
                        DeletionPhase::AwaitingConfirmation,
                        DeletionPhase::Revalidating
                            | DeletionPhase::Refused
                            | DeletionPhase::Inactive
                    )
                    | (
                        DeletionPhase::Revalidating,
                        DeletionPhase::Deleting | DeletionPhase::Refused
                    )
                    | (
                        DeletionPhase::Deleting,
                        DeletionPhase::Deleted
                            | DeletionPhase::Refused
                            | DeletionPhase::PartiallyDeleted
                    )
                    | (
                        DeletionPhase::Deleted
                            | DeletionPhase::Refused
                            | DeletionPhase::PartiallyDeleted,
                        DeletionPhase::Inactive | DeletionPhase::Preparing
                    )
            );
        if !allowed {
            return Err(InvalidDeletionTransition { from, to });
        }
        self.phase = to;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Progress {
    pub completed_work_units: u64,
    pub total_work_units: u64,
    pub copied_bytes: u64,
    pub bytes_requiring_copy: u64,
    pub verified_files: u64,
    pub total_files: u64,
}

impl Progress {
    pub fn percent(&self) -> u8 {
        if self.total_work_units > 0 {
            return ((self.completed_work_units.min(self.total_work_units) as u128 * 100)
                / self.total_work_units as u128) as u8;
        }
        if self.total_files > 0 {
            return ((self.verified_files.min(self.total_files) as u128 * 100)
                / self.total_files as u128) as u8;
        }
        0
    }

    pub fn record_copy(&mut self, bytes: u64) {
        let remaining_copy = self.bytes_requiring_copy.saturating_sub(self.copied_bytes);
        let applied = bytes.min(remaining_copy);
        self.copied_bytes = self.copied_bytes.saturating_add(applied);
        self.completed_work_units = self
            .completed_work_units
            .saturating_add(applied)
            .min(self.total_work_units);
    }

    pub fn record_verification(&mut self, bytes: u64) {
        self.completed_work_units = self
            .completed_work_units
            .saturating_add(bytes)
            .min(self.total_work_units);
    }

    pub fn record_verified_file(&mut self) {
        self.verified_files = self.verified_files.saturating_add(1).min(self.total_files);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_documented_backup_path() {
        let mut state = BackupStateMachine::default();
        for phase in [
            BackupPhase::Detecting,
            BackupPhase::Scanning,
            BackupPhase::CheckingCapacity,
            BackupPhase::Copying,
            BackupPhase::Verifying,
            BackupPhase::CompletedDeletionPending,
        ] {
            state
                .transition(phase)
                .expect("documented transition must succeed");
        }
        assert_eq!(state.phase(), BackupPhase::CompletedDeletionPending);
    }

    #[test]
    fn rejects_backup_to_deletion_ready_from_idle() {
        let mut state = BackupStateMachine::default();
        assert!(
            state
                .transition(BackupPhase::CompletedDeletionPending)
                .is_err()
        );
        assert_eq!(state.phase(), BackupPhase::Idle);
    }

    #[test]
    fn accepts_the_documented_deletion_path() {
        let mut state = DeletionStateMachine::default();
        for phase in [
            DeletionPhase::Preparing,
            DeletionPhase::AwaitingConfirmation,
            DeletionPhase::Revalidating,
            DeletionPhase::Deleting,
            DeletionPhase::Deleted,
        ] {
            state
                .transition(phase)
                .expect("documented transition must succeed");
        }
        assert_eq!(state.phase(), DeletionPhase::Deleted);
    }

    #[test]
    fn progress_combines_copy_and_verification_work_without_exceeding_totals() {
        let mut progress = Progress {
            total_work_units: 300,
            bytes_requiring_copy: 100,
            total_files: 1,
            ..Progress::default()
        };
        progress.record_copy(100);
        assert_eq!(progress.percent(), 33);
        assert_eq!(progress.copied_bytes, 100);
        progress.record_verification(500);
        progress.record_verified_file();
        assert_eq!(progress.percent(), 100);
        assert_eq!(progress.completed_work_units, 300);
        assert_eq!(progress.verified_files, 1);
    }

    #[test]
    fn zero_byte_progress_falls_back_to_verified_files() {
        let mut progress = Progress {
            total_files: 2,
            ..Progress::default()
        };
        progress.record_verified_file();
        assert_eq!(progress.percent(), 50);
        progress.record_verified_file();
        progress.record_verified_file();
        assert_eq!(progress.percent(), 100);
        assert_eq!(progress.verified_files, 2);
    }
}
