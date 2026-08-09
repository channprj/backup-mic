use crate::error::CoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferenceKey {
    AutomaticBackup,
    M4aConversion,
    AutomaticTrash,
}

impl PreferenceKey {
    pub(crate) const fn storage_key(self) -> &'static str {
        match self {
            Self::AutomaticBackup => "automatic_backup",
            Self::M4aConversion => "m4a_conversion",
            Self::AutomaticTrash => "automatic_trash",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupPreferences {
    pub automatic_backup: bool,
    pub m4a_conversion: bool,
    pub automatic_trash: bool,
}

impl Default for BackupPreferences {
    fn default() -> Self {
        Self {
            automatic_backup: true,
            m4a_conversion: true,
            automatic_trash: false,
        }
    }
}

pub(crate) fn decode_bool(value: Option<String>, default: bool) -> Result<bool, CoreError> {
    match value.as_deref() {
        None => Ok(default),
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(_) => Err(CoreError::LedgerCorrupt),
    }
}
