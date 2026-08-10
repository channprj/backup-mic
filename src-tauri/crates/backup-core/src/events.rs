use serde::{Deserialize, Serialize};

use crate::source::SourceId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivitySeverity {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityEntry {
    pub occurred_at: String,
    pub code: String,
    pub source_id: Option<SourceId>,
    pub source_label: Option<String>,
    pub count_value: Option<u64>,
    pub byte_value: Option<u64>,
    pub severity: ActivitySeverity,
}

pub trait EventSink: Send + Sync {
    fn publish(&self, entry: ActivityEntry);
}
