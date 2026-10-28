//! When each transcript line was spoken, in the recorder's own wall-clock time.
//!
//! Recorders put the start time in the file name (`TX01_MIC001_20251028_143022.wav`), so the
//! name is the first source. A name without one falls back to the file's modification time
//! minus the recording's length, which is when a recorder that stamps the file at the end of a
//! take started it. Both are local wall-clock times: the recorder knows no time zone, and the
//! transcript should read the way the user's clock did.

use std::sync::LazyLock;

use regex::Regex;
use time::{
    Date, Duration, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset,
    macros::format_description,
};

/// Where a recording's start time came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartSource {
    FileName,
    ModifiedTime,
}

/// The start time a file name encodes, if it encodes a valid one.
///
/// Accepts `YYYYMMDD` followed by `HHMMSS` with at most one `_`, `-`, `T`, or space between
/// them, and `YYYY-MM-DD HH-MM-SS` with `-`, `.`, or `:` between the time parts. The digits
/// must not run on into more digits, so a counter like `REC00012025…` is not misread.
pub fn start_from_name(file_name: &str) -> Option<PrimitiveDateTime> {
    static COMPACT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?:^|[^0-9])(?P<y>(?:19|20)[0-9]{2})(?P<mo>[0-9]{2})(?P<d>[0-9]{2})[_\-T ]?(?P<h>[0-9]{2})(?P<mi>[0-9]{2})(?P<s>[0-9]{2})(?:[^0-9]|$)",
        )
        .expect("compact recording time pattern is valid")
    });
    static SEPARATED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?:^|[^0-9])(?P<y>(?:19|20)[0-9]{2})-(?P<mo>[0-9]{2})-(?P<d>[0-9]{2})[_T ](?P<h>[0-9]{2})[\-.:](?P<mi>[0-9]{2})[\-.:](?P<s>[0-9]{2})(?:[^0-9]|$)",
        )
        .expect("separated recording time pattern is valid")
    });
    [&*COMPACT, &*SEPARATED].into_iter().find_map(|pattern| {
        pattern.captures_iter(file_name).find_map(|captures| {
            let number = |name: &str| captures.name(name)?.as_str().parse::<u16>().ok();
            let date = Date::from_calendar_date(
                i32::from(number("y")?),
                Month::try_from(u8::try_from(number("mo")?).ok()?).ok()?,
                u8::try_from(number("d")?).ok()?,
            )
            .ok()?;
            let time = Time::from_hms(
                u8::try_from(number("h")?).ok()?,
                u8::try_from(number("mi")?).ok()?,
                u8::try_from(number("s")?).ok()?,
            )
            .ok()?;
            Some(PrimitiveDateTime::new(date, time))
        })
    })
}

/// The file's modification time in the local wall clock, for when the name has no time.
pub fn local_modified_time(modified_ns: i128, offset: UtcOffset) -> Option<PrimitiveDateTime> {
    let modified = OffsetDateTime::from_unix_timestamp_nanos(modified_ns)
        .ok()?
        .to_offset(offset);
    Some(PrimitiveDateTime::new(modified.date(), modified.time()))
}

/// The start time to stamp lines with: the name's time, or the modification time less the
/// recording's length.
pub fn recording_start(
    name_start: Option<PrimitiveDateTime>,
    modified_at: Option<PrimitiveDateTime>,
    duration_ms: u64,
) -> Option<(PrimitiveDateTime, StartSource)> {
    if let Some(start) = name_start {
        return Some((start, StartSource::FileName));
    }
    let length = Duration::milliseconds(i64::try_from(duration_ms).ok()?);
    Some((modified_at?.checked_sub(length)?, StartSource::ModifiedTime))
}

/// `YYYY-MM-DD HH:MM:SS` for the moment `offset_ms` into a recording that began at `start`,
/// the stamp WakeNote puts in front of each exported transcript line.
pub fn line_timestamp(start: PrimitiveDateTime, offset_ms: u64) -> String {
    let at = i64::try_from(offset_ms)
        .ok()
        .and_then(|offset| start.checked_add(Duration::milliseconds(offset)))
        .unwrap_or(start);
    at.format(format_description!(
        "[year]-[month]-[day] [hour]:[minute]:[second]"
    ))
    .unwrap_or_default()
}

/// `HH:MM:SS` into the recording, for a transcript whose start time is unknown.
pub fn elapsed_timestamp(offset_ms: u64) -> String {
    let seconds = offset_ms / 1_000;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3_600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// The label in brackets after each stamp: the DJI transmitter when the name says which one,
/// otherwise the recorder rule's name.
pub fn source_label(source_file_name: &str, rule_name: Option<&str>) -> String {
    let prefix = source_file_name.get(..5).unwrap_or_default();
    for transmitter in ["TX01", "TX02"] {
        if prefix.eq_ignore_ascii_case(&format!("{transmitter}_")) {
            return transmitter.to_owned();
        }
    }
    rule_name
        .map(|name| crate::dto::safe_label(name.trim()))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "녹음".to_owned())
}

#[cfg(test)]
mod tests {
    use time::macros::{datetime, offset};

    use super::*;

    #[test]
    fn reads_the_start_time_recorders_put_in_file_names() {
        for (name, expected) in [
            (
                "TX01_MIC001_20251028_143022.wav",
                datetime!(2025-10-28 14:30:22),
            ),
            (
                "TX02_MIC003_20251028_143022_orig.WAV",
                datetime!(2025-10-28 14:30:22),
            ),
            (
                "251028-TX01_MIC001_20251028_143022.m4a",
                datetime!(2025-10-28 14:30:22),
            ),
            (
                "DJI_20250101235959_0001_D.WAV",
                datetime!(2025-01-01 23:59:59),
            ),
            (
                "Recording 2025-10-28 09.05.07.m4a",
                datetime!(2025-10-28 09:05:07),
            ),
            ("2025-10-28_09-05-07.wav", datetime!(2025-10-28 09:05:07)),
        ] {
            assert_eq!(start_from_name(name), Some(expected), "{name}");
        }
    }

    #[test]
    fn names_without_a_valid_time_give_none() {
        for name in [
            "ZOOM0001.WAV",
            "REC0001.WAV",
            "TX01_MIC001_20251328_143022.wav",
            "TX01_MIC001_20251028_256022.wav",
            "1202510281430220.wav",
            "20251028.wav",
        ] {
            assert_eq!(start_from_name(name), None, "{name}");
        }
    }

    #[test]
    fn a_name_without_a_time_falls_back_to_modification_less_length() {
        let modified = local_modified_time(1_761_633_022_000_000_000, offset!(+9)).unwrap();
        assert_eq!(modified, datetime!(2025-10-28 15:30:22));
        assert_eq!(
            recording_start(None, Some(modified), 90_500),
            Some((datetime!(2025-10-28 15:28:51.5), StartSource::ModifiedTime))
        );
        assert_eq!(
            recording_start(Some(datetime!(2025-10-28 14:30:22)), Some(modified), 90_500),
            Some((datetime!(2025-10-28 14:30:22), StartSource::FileName)),
            "the name wins over the modification time"
        );
        assert_eq!(recording_start(None, None, 1_000), None);
    }

    #[test]
    fn stamps_lines_in_wakenote_form_and_crosses_midnight() {
        let start = datetime!(2025-10-28 23:59:58);
        assert_eq!(line_timestamp(start, 0), "2025-10-28 23:59:58");
        assert_eq!(line_timestamp(start, 3_999), "2025-10-29 00:00:01");
        assert_eq!(elapsed_timestamp(3_723_456), "01:02:03");
    }

    #[test]
    fn labels_dji_transmitters_and_falls_back_to_the_rule_name() {
        assert_eq!(
            source_label("TX01_MIC001_20251028_143022.wav", Some("DJI")),
            "TX01"
        );
        assert_eq!(
            source_label("tx02_mic001_20251028_143022.wav", None),
            "TX02"
        );
        assert_eq!(source_label("ZOOM0001.WAV", Some("Zoom H1n")), "Zoom H1n");
        assert_eq!(source_label("REC0001.WAV", None), "녹음");
    }
}
