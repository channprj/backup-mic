use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use uuid::Uuid;

use crate::{device::VolumeDescriptor, error::CoreError, rule::RuleId};

pub const LEGACY_TX01_SOURCE_ID: &str = "10da26f2-f143-4e3c-b8fe-464a265755f1";
pub const LEGACY_TX02_SOURCE_ID: &str = "20da26f2-f143-4e3c-b8fe-464a265755f2";

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SourceId(String);

impl SourceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().hyphenated().to_string())
    }

    pub fn parse(value: &str) -> Result<Self, CoreError> {
        Uuid::parse_str(value)
            .map(|uuid| Self(uuid.hyphenated().to_string()))
            .map_err(|_| CoreError::InvalidRequest)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SourceId {
    fn default() -> Self {
        Self::new()
    }
}

impl<'de> Deserialize<'de> for SourceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRecord {
    pub id: SourceId,
    pub rule_id: RuleId,
    pub volume_uuid: String,
    pub legacy_slot: Option<String>,
    pub display_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountedSourceAuthority {
    pub source: SourceRecord,
    pub descriptor: VolumeDescriptor,
}

pub(crate) fn validate_source(source: &SourceRecord) -> Result<(), CoreError> {
    if SourceId::parse(source.id.as_str())? != source.id
        || RuleId::parse(source.rule_id.as_str()).map_err(|_| CoreError::InvalidRequest)?
            != source.rule_id
        || !safe_text(&source.volume_uuid)
        || !safe_text(&source.display_name)
        || source
            .legacy_slot
            .as_deref()
            .is_some_and(|slot| !matches!(slot, "TX01" | "TX02"))
    {
        return Err(CoreError::InvalidRequest);
    }
    Ok(())
}

fn safe_text(value: &str) -> bool {
    !value.trim().is_empty() && value.chars().count() <= 256 && !value.chars().any(char::is_control)
}
