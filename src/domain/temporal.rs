//! Timestamp input contract. Observation time is caller provenance; recording
//! time is assigned by the application's Clock and cannot be supplied here.
use chrono::{DateTime, Utc};

use super::DomainError;

/// RFC3339's four-digit year, nanosecond fraction and numeric offset fit in 35
/// bytes. Reject extra precision rather than silently truncating caller input.
pub const MAX_OBSERVED_AT_BYTES: usize = 35;

pub fn validate_observed_at(value: &str) -> Result<DateTime<Utc>, DomainError> {
    let bytes = value.as_bytes();
    let invalid = || DomainError::InvalidObservedAt;
    if !(20..=MAX_OBSERVED_AT_BYTES).contains(&bytes.len())
        || !value.is_ascii()
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't')
        || bytes[13] != b':'
        || bytes[16] != b':'
        || [0..4, 5..7, 8..10, 11..13, 14..16, 17..19]
            .iter()
            .any(|range| !bytes[range.clone()].iter().all(u8::is_ascii_digit))
    {
        return Err(invalid());
    }
    let mut zone_start = 19;
    if bytes[zone_start] == b'.' {
        zone_start += 1;
        let fraction_start = zone_start;
        while bytes.get(zone_start).is_some_and(u8::is_ascii_digit) {
            zone_start += 1;
        }
        if !(1..=9).contains(&(zone_start - fraction_start)) {
            return Err(invalid());
        }
    }
    let zone = &bytes[zone_start..];
    if !(matches!(zone, [b'Z' | b'z'])
        || (zone.len() == 6
            && matches!(zone[0], b'+' | b'-')
            && zone[3] == b':'
            && zone[1..3].iter().all(u8::is_ascii_digit)
            && zone[4..6].iter().all(u8::is_ascii_digit)))
    {
        return Err(invalid());
    }
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| invalid())
}
