use serde::{Deserialize, Serialize};

use crate::state::Transmitter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivitySeverity {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEntry {
    pub occurred_at: String,
    pub code: String,
    pub transmitter: Option<Transmitter>,
    pub count_value: Option<u64>,
    pub byte_value: Option<u64>,
    pub severity: ActivitySeverity,
}

pub trait EventSink: Send + Sync {
    fn publish(&self, entry: ActivityEntry);
}
