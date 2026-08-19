//! Time helpers shared by the Tauri adapter.
//!
//! Two different clocks matter here. Ledger evidence is timestamped in UTC so that stored
//! records stay comparable across daylight-saving changes and machine moves. Audit logs and
//! the daily log path use the local calendar, because that is the date the user is looking
//! for when they open the log folder.

use time::{OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

/// The current local UTC offset, falling back to UTC when the platform cannot report one.
///
/// `current_local_offset` fails on some threading configurations rather than guessing, so a
/// caller that treated the error as fatal would refuse to back anything up over a clock
/// detail. Falling back to UTC keeps the run going and only shifts which calendar folder a
/// log lands in.
pub fn local_offset() -> UtcOffset {
    UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)
}

/// Now, expressed in the local calendar.
pub fn local_now() -> OffsetDateTime {
    OffsetDateTime::now_utc().to_offset(local_offset())
}

/// Now as an RFC 3339 UTC timestamp, for ledger evidence.
///
/// Formatting a well-known description cannot realistically fail, but the epoch fallback keeps
/// the signature infallible so no caller has to invent an error path for it.
pub fn now_string() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{local_now, now_string};
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};

    #[test]
    fn evidence_timestamps_are_utc_rfc3339() {
        let stamp = now_string();
        let parsed = OffsetDateTime::parse(&stamp, &Rfc3339).expect("timestamp is RFC 3339");
        assert_eq!(parsed.offset(), time::UtcOffset::UTC);
        assert!(stamp.ends_with('Z'), "{stamp} keeps the UTC designator");
    }

    #[test]
    fn the_local_clock_agrees_with_the_evidence_clock() {
        let local = local_now();
        let evidence =
            OffsetDateTime::parse(&now_string(), &Rfc3339).expect("timestamp is RFC 3339");
        assert!(
            (local - evidence).abs() < time::Duration::seconds(5),
            "the local calendar view must describe the same instant"
        );
    }
}
