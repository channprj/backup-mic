use std::{
    fs::File,
    io::Write as _,
    path::{Path, PathBuf},
    sync::Mutex,
};

use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{error::CoreError, state::Transmitter};

mod storage;

static WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditDurability {
    Buffered,
    SyncData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditValue<'a> {
    Text(&'a str),
    Unsigned(u64),
    Boolean(bool),
}

#[derive(Debug, Clone, Copy)]
pub struct AuditEvent<'a> {
    pub occurred_at: OffsetDateTime,
    pub level: AuditLevel,
    pub code: &'static str,
    pub transmitter: Option<Transmitter>,
    pub fields: &'a [(&'static str, AuditValue<'a>)],
}

pub trait AuditSink: Send + Sync {
    fn append(
        &self,
        event: &AuditEvent<'_>,
        durability: AuditDurability,
    ) -> Result<PathBuf, CoreError>;
}

#[derive(Debug, Clone)]
pub struct FileAuditLog {
    root: PathBuf,
    log_root: PathBuf,
}

impl FileAuditLog {
    pub fn new(destination_root: impl AsRef<Path>) -> Self {
        Self {
            root: destination_root.as_ref().to_path_buf(),
            log_root: destination_root.as_ref().join("logs"),
        }
    }

    pub fn new_log_root(log_root: impl AsRef<Path>) -> Self {
        Self {
            root: log_root.as_ref().to_path_buf(),
            log_root: log_root.as_ref().to_path_buf(),
        }
    }

    pub fn path_for(&self, occurred_at: OffsetDateTime) -> PathBuf {
        let year = occurred_at.year();
        let month = u8::from(occurred_at.month());
        let day = occurred_at.day();
        self.log_root
            .join(format!("{year:04}"))
            .join(format!("{month:02}"))
            .join(format!(
                "{:02}{month:02}{day:02}-backup-mic.log",
                year % 100
            ))
    }

    /// Creates the calendar directory without following links below the configured root.
    /// Used by both the writer and the command that opens the log folder.
    pub fn ensure_directory(&self, occurred_at: OffsetDateTime) -> Result<PathBuf, CoreError> {
        let path = self.path_for(occurred_at);
        self.open_parent(&path)?;
        path.parent()
            .map(PathBuf::from)
            .ok_or(CoreError::InvalidAuditEvent)
    }

    fn open_parent(&self, path: &Path) -> Result<File, CoreError> {
        let relative = path
            .parent()
            .and_then(|parent| parent.strip_prefix(&self.root).ok())
            .ok_or(CoreError::InvalidAuditEvent)?;
        storage::directory(&self.root, relative).map_err(CoreError::AuditLogUnavailable)
    }
}

impl AuditSink for FileAuditLog {
    fn append(
        &self,
        event: &AuditEvent<'_>,
        durability: AuditDurability,
    ) -> Result<PathBuf, CoreError> {
        validate_event(event)?;
        let line = encode_event(event)?;
        let path = self.path_for(event.occurred_at);
        let _guard = WRITE_LOCK.lock().map_err(|_| {
            CoreError::AuditLogUnavailable(std::io::Error::other("audit writer lock poisoned"))
        })?;
        let parent = self.open_parent(&path)?;
        let name = path.file_name().ok_or(CoreError::InvalidAuditEvent)?;
        let mut file =
            storage::append_file(&parent, name).map_err(CoreError::AuditLogUnavailable)?;
        file.write_all(line.as_bytes())
            .map_err(CoreError::AuditLogUnavailable)?;
        if durability == AuditDurability::SyncData {
            file.sync_data().map_err(CoreError::AuditLogUnavailable)?;
        }
        Ok(path)
    }
}

fn validate_event(event: &AuditEvent<'_>) -> Result<(), CoreError> {
    if event.code.is_empty()
        || !event.code.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_')
        })
    {
        return Err(CoreError::InvalidAuditEvent);
    }
    for (key, value) in event.fields {
        if !matches!(
            *key,
            "source"
                | "output"
                | "source_bytes"
                | "output_bytes"
                | "format"
                | "count"
                | "reason"
                | "mode"
                | "sessions"
                | "files"
                | "bytes"
                | "operation"
                | "stage"
                | "error_code"
                | "os_kind"
                | "retryable"
                | "item"
                | "setting"
                | "enabled"
                | "value"
                | "applies"
        ) {
            return Err(CoreError::InvalidAuditEvent);
        }
        if let AuditValue::Text(text) = value
            && (Path::new(text).is_absolute()
                || is_full_hash(text)
                || uuid::Uuid::parse_str(text).is_ok()
                || text.len() > 1024)
        {
            return Err(CoreError::InvalidAuditEvent);
        }
    }
    Ok(())
}

fn encode_event(event: &AuditEvent<'_>) -> Result<String, CoreError> {
    use std::fmt::Write as _;

    let timestamp = event
        .occurred_at
        .format(&Rfc3339)
        .map_err(|_| CoreError::InvalidAuditEvent)?;
    let mut line = format!("{timestamp} {} {}", level_name(event.level), event.code);
    if let Some(transmitter) = event.transmitter {
        write!(&mut line, " tx={}", transmitter_name(transmitter))
            .map_err(|_| CoreError::InvalidAuditEvent)?;
    }
    for (key, value) in event.fields {
        write!(&mut line, " {key}=").map_err(|_| CoreError::InvalidAuditEvent)?;
        match value {
            AuditValue::Text(text) => {
                line.push('"');
                escape_text(&mut line, text);
                line.push('"');
            }
            AuditValue::Unsigned(value) => {
                write!(&mut line, "{value}").map_err(|_| CoreError::InvalidAuditEvent)?;
            }
            AuditValue::Boolean(value) => {
                write!(&mut line, "{value}").map_err(|_| CoreError::InvalidAuditEvent)?;
            }
        }
    }
    line.push('\n');
    Ok(line)
}

fn escape_text(output: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => output.push('\u{fffd}'),
            character => output.push(character),
        }
    }
}

fn is_full_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn level_name(level: AuditLevel) -> &'static str {
    match level {
        AuditLevel::Info => "INFO",
        AuditLevel::Warning => "WARN",
        AuditLevel::Error => "ERROR",
    }
}

fn transmitter_name(transmitter: Transmitter) -> &'static str {
    match transmitter {
        Transmitter::Tx01 => "TX01",
        Transmitter::Tx02 => "TX02",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use tempfile::tempdir;
    use time::macros::datetime;

    use super::*;

    #[test]
    fn accepts_namespaced_codes_with_snake_case_segments() {
        let directory = tempdir().unwrap();
        let log = FileAuditLog::new_log_root(directory.path());
        let event = AuditEvent {
            occurred_at: datetime!(2026-08-10 03:00 +09:00),
            level: AuditLevel::Info,
            code: "backup.copy_cohort_verified",
            transmitter: None,
            fields: &[],
        };

        let path = log.append(&event, AuditDurability::SyncData).unwrap();
        let written = fs::read_to_string(path).unwrap();

        assert!(written.contains("INFO backup.copy_cohort_verified"));
    }
}
