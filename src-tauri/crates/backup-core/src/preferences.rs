use std::time::Duration;

use crate::error::CoreError;

const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;

/// A setting the user turns on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferenceFlag {
    AutomaticBackup,
    M4aConversion,
    AutomaticTrash,
}

impl PreferenceFlag {
    /// The settings-table key, which is also the name the audit log records.
    pub const fn storage_key(self) -> &'static str {
        match self {
            Self::AutomaticBackup => "automatic_backup",
            Self::M4aConversion => "m4a_conversion",
            Self::AutomaticTrash => "automatic_trash",
        }
    }
}

/// A whole-number policy limit the user can change.
///
/// Both of these were compile-time constants that the surrounding code already accepted as
/// ordinary parameters, so persisting them changes only where the number comes from — no
/// verification, barrier, or retirement rule depends on the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferenceLimit {
    /// Free space to leave on the destination volume, in GiB.
    ///
    /// The staging WAV and its M4A exist at the same time during conversion, so the reserve has
    /// to cover the larger of the two runs a user expects to make, not one file.
    FreeSpaceReserveGib,
    /// How often a still-connected recorder is re-fingerprinted, in seconds.
    ///
    /// Shorter notices a new recording sooner; longer keeps a bus-powered recorder idle. The
    /// scan only reads directory metadata, so neither end is expensive.
    RescanIntervalSeconds,
}

impl PreferenceLimit {
    /// The settings-table key, which is also the name the audit log records.
    pub const fn storage_key(self) -> &'static str {
        match self {
            Self::FreeSpaceReserveGib => "free_space_reserve_gib",
            Self::RescanIntervalSeconds => "rescan_interval_seconds",
        }
    }

    pub const fn default_value(self) -> u32 {
        match self {
            Self::FreeSpaceReserveGib => 10,
            Self::RescanIntervalSeconds => 15,
        }
    }

    /// The inclusive range this limit accepts.
    ///
    /// A range rather than a fixed list of choices: the settings window offers a few presets,
    /// but a stored value stays valid even if a later version changes which presets it shows.
    pub const fn range(self) -> (u32, u32) {
        match self {
            Self::FreeSpaceReserveGib => (1, 512),
            Self::RescanIntervalSeconds => (5, 3600),
        }
    }

    /// Accepts `value` or refuses it. Out-of-range input is rejected rather than clamped, so the
    /// settings window can never end up showing a number the ledger does not hold.
    pub fn accept(self, value: u32) -> Result<u32, CoreError> {
        let (low, high) = self.range();
        if (low..=high).contains(&value) {
            Ok(value)
        } else {
            Err(CoreError::InvalidRequest)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupPreferences {
    pub automatic_backup: bool,
    pub m4a_conversion: bool,
    pub automatic_trash: bool,
    pub free_space_reserve_gib: u32,
    pub rescan_interval_seconds: u32,
}

impl BackupPreferences {
    /// The reserve the destination capacity preflight must leave free.
    pub const fn free_space_reserve_bytes(&self) -> u64 {
        self.free_space_reserve_gib as u64 * BYTES_PER_GIB
    }

    /// How long a mounted recorder waits between metadata rescans.
    pub const fn rescan_interval(&self) -> Duration {
        Duration::from_secs(self.rescan_interval_seconds as u64)
    }
}

impl Default for BackupPreferences {
    fn default() -> Self {
        Self {
            automatic_backup: true,
            m4a_conversion: true,
            automatic_trash: false,
            free_space_reserve_gib: PreferenceLimit::FreeSpaceReserveGib.default_value(),
            rescan_interval_seconds: PreferenceLimit::RescanIntervalSeconds.default_value(),
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

/// Reads a stored limit, treating an unset row as the default.
///
/// A row that is not a number in range means the ledger was edited outside the app, which is the
/// same class of problem as any other corrupt row and gets the same fail-closed answer.
pub(crate) fn decode_limit(
    value: Option<String>,
    limit: PreferenceLimit,
) -> Result<u32, CoreError> {
    match value.as_deref() {
        None => Ok(limit.default_value()),
        Some(stored) => stored
            .parse::<u32>()
            .map_err(|_| CoreError::LedgerCorrupt)
            .and_then(|value| limit.accept(value).map_err(|_| CoreError::LedgerCorrupt)),
    }
}

#[cfg(test)]
mod tests {
    use super::{BackupPreferences, PreferenceLimit, decode_limit};
    use crate::error::CoreError;

    #[test]
    fn defaults_keep_the_previous_compiled_in_policy() {
        let defaults = BackupPreferences::default();
        assert_eq!(defaults.free_space_reserve_gib, 10);
        assert_eq!(defaults.free_space_reserve_bytes(), 10 * 1024 * 1024 * 1024);
        assert_eq!(defaults.rescan_interval_seconds, 15);
        assert_eq!(defaults.rescan_interval().as_secs(), 15);
    }

    #[test]
    fn an_unset_row_reads_as_the_default() {
        for limit in [
            PreferenceLimit::FreeSpaceReserveGib,
            PreferenceLimit::RescanIntervalSeconds,
        ] {
            assert_eq!(decode_limit(None, limit).unwrap(), limit.default_value());
        }
    }

    #[test]
    fn limits_accept_their_range_and_refuse_everything_outside_it() {
        for limit in [
            PreferenceLimit::FreeSpaceReserveGib,
            PreferenceLimit::RescanIntervalSeconds,
        ] {
            let (low, high) = limit.range();
            assert_eq!(limit.accept(low).unwrap(), low);
            assert_eq!(limit.accept(high).unwrap(), high);
            for refused in [0, low - 1, high + 1] {
                assert!(
                    matches!(limit.accept(refused), Err(CoreError::InvalidRequest)),
                    "{refused} must be refused, not clamped"
                );
            }
        }
    }

    #[test]
    fn a_stored_row_outside_the_range_or_shape_is_corruption() {
        let limit = PreferenceLimit::FreeSpaceReserveGib;
        assert_eq!(
            decode_limit(Some("10".to_owned()), limit).unwrap(),
            10,
            "a valid row round-trips"
        );
        for stored in ["0", "513", "-1", "10.5", "ten", "", " 10"] {
            assert!(
                matches!(
                    decode_limit(Some(stored.to_owned()), limit),
                    Err(CoreError::LedgerCorrupt)
                ),
                "{stored} must be refused"
            );
        }
    }

    #[test]
    fn the_reserve_converts_gibibytes_without_overflowing() {
        let preferences = BackupPreferences {
            free_space_reserve_gib: PreferenceLimit::FreeSpaceReserveGib.range().1,
            ..BackupPreferences::default()
        };
        assert_eq!(
            preferences.free_space_reserve_bytes(),
            512 * 1024 * 1024 * 1024
        );
    }
}
