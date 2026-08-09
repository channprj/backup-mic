use std::{
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::Mutex,
};

use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{error::CoreError, state::Transmitter};

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
    destination_root: PathBuf,
}

impl FileAuditLog {
    pub fn new(destination_root: impl AsRef<Path>) -> Self {
        Self {
            destination_root: destination_root.as_ref().to_path_buf(),
        }
    }

    pub fn path_for(&self, occurred_at: OffsetDateTime) -> PathBuf {
        let year = occurred_at.year();
        let month = u8::from(occurred_at.month());
        let day = occurred_at.day();
        self.destination_root
            .join("logs")
            .join(format!("{year:04}"))
            .join(format!("{month:02}"))
            .join(format!(
                "{:02}{month:02}{day:02}-backup-mic.log",
                year % 100
            ))
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
        let parent = path.parent().ok_or(CoreError::InvalidAuditEvent)?;
        let _guard = WRITE_LOCK.lock().map_err(|_| {
            CoreError::AuditLogUnavailable(std::io::Error::other("audit writer lock poisoned"))
        })?;
        fs::create_dir_all(parent).map_err(CoreError::AuditLogUnavailable)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(CoreError::AuditLogUnavailable)?;
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
        || !event
            .code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.')
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
