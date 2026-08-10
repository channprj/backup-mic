use std::{path::PathBuf, sync::LazyLock};

use regex::Regex;
use time::{Date, Month};

use crate::{additional_file::AdditionalFileClass, rule::FilenameProfile, state::Transmitter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdditionalFileObservation {
    pub relative_path: PathBuf,
    pub file_name: String,
    pub size: u64,
    pub modified_nanos: i128,
    pub classification: AdditionalFileClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRecordingName {
    pub transmitter_hint: Option<Transmitter>,
    pub destination_date: Date,
    pub used_fallback_date: bool,
}

pub fn parse_recording_name(file_name: &str, fallback_date: Date) -> ParsedRecordingName {
    static RECORDING_NAME: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)^TX(?P<tx>01|02)_MIC[0-9]+_(?P<date>[0-9]{8})_(?P<time>[0-9]{6})(?:_[^/]+)?\.wav$",
        )
        .expect("recording regex is valid")
    });

    let upper = file_name.to_ascii_uppercase();
    let transmitter_hint = if upper.starts_with("TX01_") {
        Some(Transmitter::Tx01)
    } else if upper.starts_with("TX02_") {
        Some(Transmitter::Tx02)
    } else {
        None
    };
    let Some(captures) = RECORDING_NAME.captures(file_name) else {
        return ParsedRecordingName {
            transmitter_hint,
            destination_date: fallback_date,
            used_fallback_date: true,
        };
    };
    let date = captures
        .name("date")
        .map(|value| value.as_str())
        .unwrap_or_default();
    let time = captures
        .name("time")
        .map(|value| value.as_str())
        .unwrap_or_default();
    if let Some(destination_date) = valid_encoded_date_time(date, time) {
        return ParsedRecordingName {
            transmitter_hint,
            destination_date,
            used_fallback_date: false,
        };
    }
    ParsedRecordingName {
        transmitter_hint,
        destination_date: fallback_date,
        used_fallback_date: true,
    }
}

pub fn archive_date_for_filename(
    profile: FilenameProfile,
    file_name: &str,
    fallback_date: Date,
) -> Date {
    match profile {
        FilenameProfile::Preserve => fallback_date,
        FilenameProfile::DjiTxShort => {
            parse_recording_name(file_name, fallback_date).destination_date
        }
    }
}

fn valid_encoded_date_time(date: &str, time: &str) -> Option<Date> {
    let year = date.get(0..4)?.parse().ok()?;
    let month = Month::try_from(date.get(4..6)?.parse::<u8>().ok()?).ok()?;
    let day = date.get(6..8)?.parse().ok()?;
    let hour = time.get(0..2)?.parse::<u8>().ok()?;
    let minute = time.get(2..4)?.parse::<u8>().ok()?;
    let second = time.get(4..6)?.parse::<u8>().ok()?;
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Date::from_calendar_date(year, month, day).ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingObservation {
    pub relative_path: PathBuf,
    pub file_name: String,
    pub size: u64,
    pub modified_nanos: i128,
    pub parsed_name: ParsedRecordingName,
}

#[cfg(test)]
mod tests {
    use time::{Month, macros::date};

    use super::*;

    #[test]
    fn parses_observed_tx01_name_and_encoded_date() {
        let parsed = parse_recording_name(
            "TX01_MIC009_20260809_021728_edit.wav",
            date!(2024 - 01 - 01),
        );
        assert_eq!(parsed.transmitter_hint, Some(Transmitter::Tx01));
        assert_eq!(
            parsed.destination_date,
            Date::from_calendar_date(2026, Month::August, 9).unwrap()
        );
        assert!(!parsed.used_fallback_date);
    }

    #[test]
    fn parses_tx02_case_insensitively() {
        let parsed = parse_recording_name(
            "tx02_MIC002_20260809_021844_edit.WAV",
            date!(2024 - 01 - 01),
        );
        assert_eq!(parsed.transmitter_hint, Some(Transmitter::Tx02));
        assert!(!parsed.used_fallback_date);
    }

    #[test]
    fn malformed_or_unknown_names_use_the_modification_date() {
        let fallback = date!(2026 - 07 - 31);
        for name in ["TX01_MIC001_20261340_250000.wav", "conversation.wav"] {
            let parsed = parse_recording_name(name, fallback);
            assert_eq!(parsed.destination_date, fallback);
            assert!(parsed.used_fallback_date);
        }
    }
}
