//! Zeit-Konventionen (00 §3): `DateTime<Utc>` gespeichert, `chrono_tz::Tz` für Team-Zeitzonen.

use crate::error::DomainError;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;

/// Gespeicherter Zeitpunkt (UTC).
pub type Timestamp = DateTime<Utc>;

/// Default-Zeitzone eines Teams (`Team.timezone`).
pub const DEFAULT_TEAM_TIMEZONE: Tz = chrono_tz::America::Los_Angeles;

/// Parst einen IANA-Zeitzonennamen.
pub fn parse_timezone(name: &str) -> Result<Tz, DomainError> {
    name.parse::<Tz>()
        .map_err(|_| DomainError::validation("timezone", format!("unknown time zone `{name}`")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_and_rejects_unknown() {
        assert_eq!(
            parse_timezone("Europe/Berlin").unwrap(),
            chrono_tz::Europe::Berlin
        );
        assert!(parse_timezone("Mars/Base").is_err());
        assert_eq!(DEFAULT_TEAM_TIMEZONE.name(), "America/Los_Angeles");
    }
}
