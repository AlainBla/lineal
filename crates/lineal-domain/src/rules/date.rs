//! Strikter Datumsparser (Paket 8, B8.14): `YYYY-MM-DD` mit vierstelligem Jahr im
//! Plausibilitätsbereich; alle API-Eingaben vom Typ `Date` laufen hierüber.

use crate::error::DomainError;
use chrono::{Datelike, NaiveDate};

/// Frühestes zulässiges Jahr.
pub const MIN_YEAR: i32 = 1900;
/// Spätestes zulässiges Jahr.
pub const MAX_YEAR: i32 = 2200;

/// Parst `YYYY-MM-DD` streng (genau 10 ASCII-Zeichen, kein Vorzeichen, keine ein-/zweistelligen
/// Teile) und prüft den Jahresbereich [`MIN_YEAR`], [`MAX_YEAR`].
pub fn parse_iso_date(field: &str, s: &str) -> Result<NaiveDate, DomainError> {
    let b = s.as_bytes();
    let shape_ok = b.len() == 10
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            _ => c.is_ascii_digit(),
        });
    let bad = || DomainError::validation(field, "must be a date in the format YYYY-MM-DD");
    if !shape_ok {
        return Err(bad());
    }
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| bad())?;
    if !(MIN_YEAR..=MAX_YEAR).contains(&d.year()) {
        return Err(DomainError::validation(
            field,
            format!("year must be between {MIN_YEAR} and {MAX_YEAR}"),
        ));
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_strict_dates_only() {
        assert!(parse_iso_date("d", "2026-02-03").is_ok());
        for bad in [
            "2026-2-3",
            "+2026-02-03",
            "26-02-03",
            "0000-01-01",
            "0001-01-01",
            "9999-12-31",
            "1899-12-31",
            "2201-01-01",
            "2026-02-30",
            "2026-02-03 ",
            "２０２６-02-03",
            "",
        ] {
            assert!(parse_iso_date("d", bad).is_err(), "{bad}");
        }
        assert!(parse_iso_date("d", "1900-01-01").is_ok());
        assert!(parse_iso_date("d", "2200-12-31").is_ok());
    }
}
