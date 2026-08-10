use std::{collections::BTreeSet, path::PathBuf};

use crate::{error::CoreError, filesystem::is_safe_additional_relative_path, source::SourceId};

pub const M4A_PROFILE_ID: &str = "aac_lc_128k_v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BatchPhase {
    Inventory,
    Copying,
    CopiesVerified,
    Converting,
    M4aCohortVerified,
    SourcesRevalidated,
    Completed,
    Failed,
}

impl BatchPhase {
    pub(crate) const fn storage_name(self) -> &'static str {
        match self {
            Self::Inventory => "inventory",
            Self::Copying => "copying",
            Self::CopiesVerified => "copies_verified",
            Self::Converting => "converting",
            Self::M4aCohortVerified => "m4a_cohort_verified",
            Self::SourcesRevalidated => "sources_revalidated",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, CoreError> {
        match value {
            "inventory" => Ok(Self::Inventory),
            "copying" => Ok(Self::Copying),
            "copies_verified" => Ok(Self::CopiesVerified),
            "converting" => Ok(Self::Converting),
            "m4a_cohort_verified" => Ok(Self::M4aCohortVerified),
            "sources_revalidated" => Ok(Self::SourcesRevalidated),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            _ => Err(CoreError::LedgerCorrupt),
        }
    }

    pub(crate) const fn predecessor(self) -> Option<Self> {
        match self {
            Self::Inventory | Self::Failed => None,
            Self::Copying => Some(Self::Inventory),
            Self::CopiesVerified => Some(Self::Copying),
            Self::Converting => Some(Self::CopiesVerified),
            Self::M4aCohortVerified => Some(Self::Converting),
            Self::SourcesRevalidated => Some(Self::M4aCohortVerified),
            Self::Completed => Some(Self::SourcesRevalidated),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenPreferences {
    pub automatic_backup: bool,
    pub m4a_conversion: bool,
    pub automatic_trash: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BatchRunEvidence {
    pub source_id: Option<SourceId>,
    pub phase: BatchPhase,
    pub frozen_preferences: FrozenPreferences,
    pub m4a_profile_id: Option<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum BatchItemKey {
    Recording {
        source_id: SourceId,
        relative_path: PathBuf,
    },
    Additional {
        source_id: SourceId,
        relative_path: PathBuf,
    },
}

impl BatchItemKey {
    fn relative_path(&self) -> &PathBuf {
        match self {
            Self::Recording { relative_path, .. } | Self::Additional { relative_path, .. } => {
                relative_path
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CopyBarrier {
    expected: BTreeSet<BatchItemKey>,
    verified: BTreeSet<BatchItemKey>,
    failed: BTreeSet<BatchItemKey>,
}

impl CopyBarrier {
    pub fn new(expected: BTreeSet<BatchItemKey>) -> Result<Self, CoreError> {
        if expected
            .iter()
            .any(|key| !is_safe_additional_relative_path(key.relative_path()))
        {
            return Err(CoreError::InvalidRequest);
        }
        Ok(Self {
            expected,
            verified: BTreeSet::new(),
            failed: BTreeSet::new(),
        })
    }

    pub fn record_verified(&mut self, key: &BatchItemKey) -> Result<(), CoreError> {
        if !self.expected.contains(key) || self.verified.contains(key) || self.failed.contains(key)
        {
            return Err(CoreError::InvalidRequest);
        }
        self.verified.insert(key.clone());
        Ok(())
    }

    pub fn record_failed(&mut self, key: &BatchItemKey) -> Result<(), CoreError> {
        if !self.expected.contains(key) || self.verified.contains(key) || self.failed.contains(key)
        {
            return Err(CoreError::InvalidRequest);
        }
        self.failed.insert(key.clone());
        Ok(())
    }

    pub fn conversion_allowed(&self) -> bool {
        self.failed.is_empty() && self.verified == self.expected
    }
}
