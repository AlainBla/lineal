//! Cycle-Kalender (docs/04 §1.4): Grenzen sind Mitternacht in der Team-Zeitzone,
//! berechnet auf Kalenderdaten und erst am Ende nach UTC umgerechnet (DST-sicher).

use crate::error::DomainError;
use crate::time::Timestamp;
use chrono::{Datelike, Duration, LocalResult, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;

/// Zeitplan-Einstellungen eines Teams (docs/04 §1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleSettings {
    /// `cycleDuration` in Wochen, 1..=8.
    pub duration_weeks: u32,
    /// `cycleCooldownTime` in Wochen, 0..=8.
    pub cooldown_weeks: u32,
    /// `cycleStartDay`: 0 = Sonntag … 6 = Samstag.
    pub start_day: u8,
}

impl CycleSettings {
    /// Prüft die Wertebereiche (docs/04 §1.2); Verstoß → `Validation` mit dem Feldnamen.
    pub fn validate(&self) -> Result<(), DomainError> {
        if !(1..=8).contains(&self.duration_weeks) {
            return Err(DomainError::validation(
                "cycleDuration",
                "must be between 1 and 8 weeks",
            ));
        }
        if self.cooldown_weeks > 8 {
            return Err(DomainError::validation(
                "cycleCooldownTime",
                "must be between 0 and 8 weeks",
            ));
        }
        if self.start_day > 6 {
            return Err(DomainError::validation(
                "cycleStartDay",
                "must be between 0 (Sunday) and 6 (Saturday)",
            ));
        }
        Ok(())
    }
}

/// Ein berechneter Cycle (ohne Nummer; die vergibt der Zähler im Core).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CycleWindow {
    /// Erster lokaler Tag (Team-Zeitzone).
    pub start_date: NaiveDate,
    /// Lokales Datum von `ends_at` (= Tag nach dem letzten Tag, exklusiv).
    pub end_date: NaiveDate,
    /// Mitternacht von `start_date` als UTC.
    pub starts_at: Timestamp,
    /// Mitternacht von `end_date` als UTC.
    pub ends_at: Timestamp,
}

/// UTC-Zeitpunkt von `date` 00:00 in `tz` (docs/04 §1.4, `localMidnightToUtc`).
///
/// Existiert 00:00 wegen einer Zeitumstellung nicht, gilt die erste gültige Minute danach;
/// ist 00:00 doppelt vorhanden, gilt das frühere Vorkommen.
pub fn local_midnight_to_utc(date: NaiveDate, tz: Tz) -> Timestamp {
    local_to_utc(date.and_time(chrono::NaiveTime::MIN), tz)
}

/// Lokale Wandzeit nach UTC mit denselben Auflösungsregeln wie [`local_midnight_to_utc`].
pub(crate) fn local_to_utc(local: chrono::NaiveDateTime, tz: Tz) -> Timestamp {
    match tz.from_local_datetime(&local) {
        LocalResult::Single(t) => t.with_timezone(&Utc),
        LocalResult::Ambiguous(first, _) => first.with_timezone(&Utc),
        LocalResult::None => {
            // Lücke: erste gültige Minute danach (Lücken sind höchstens wenige Stunden lang).
            let mut t = local;
            for _ in 0..(48 * 60) {
                t += Duration::minutes(1);
                if let LocalResult::Single(x) | LocalResult::Ambiguous(x, _) =
                    tz.from_local_datetime(&t)
                {
                    return x.with_timezone(&Utc);
                }
            }
            // Unerreichbar bei realen Zeitzonen; sichere Rückfallebene: als UTC lesen.
            Utc.from_utc_datetime(&local)
        }
    }
}

fn next_weekday_on_or_after(date: NaiveDate, weekday_from_sunday: u8) -> NaiveDate {
    let cur = date.weekday().num_days_from_sunday() as i64;
    let delta = (weekday_from_sunday as i64 - cur).rem_euclid(7);
    date + Duration::days(delta)
}

fn window(start: NaiveDate, end: NaiveDate, tz: Tz) -> CycleWindow {
    CycleWindow {
        start_date: start,
        end_date: end,
        starts_at: local_midnight_to_utc(start, tz),
        ends_at: local_midnight_to_utc(end, tz),
    }
}

fn follow(prev_end: NaiveDate, s: &CycleSettings) -> (NaiveDate, NaiveDate) {
    let start = next_weekday_on_or_after(
        prev_end + Duration::days(7 * s.cooldown_weeks as i64),
        s.start_day,
    );
    (start, start + Duration::days(7 * s.duration_weeks as i64))
}

/// Erzeugt `count` aufeinanderfolgende Cycles ab dem lokalen Starttag `from`
/// (docs/04 §1.4, `ensureUpcomingCycles`).
///
/// * Der erste Cycle beginnt am Kalenderdatum `from` (00:00 Team-Zeit). Liegt `from` nicht
///   auf `start_day`, endet er am nächsten Starttag, der mindestens eine Woche entfernt ist.
/// * Jeder weitere Cycle beginnt nach `cooldown_weeks` am nächsten Starttag und dauert
///   `duration_weeks`. Ohne Cooldown gilt `ends_at(n) == starts_at(n+1)`.
/// * Die Stundenlänge eines Cycles darf wegen Sommerzeit um ±1 h schwanken (gewollt).
pub fn generate(
    settings: &CycleSettings,
    tz: Tz,
    from: NaiveDate,
    count: usize,
) -> Result<Vec<CycleWindow>, DomainError> {
    settings.validate()?;
    let mut out = Vec::with_capacity(count);
    if count == 0 {
        return Ok(out);
    }
    let first_end = if from.weekday().num_days_from_sunday() as u8 != settings.start_day {
        next_weekday_on_or_after(from + Duration::days(7), settings.start_day)
    } else {
        from + Duration::days(7 * settings.duration_weeks as i64)
    };
    out.push(window(from, first_end, tz));
    let mut prev_end = first_end;
    for _ in 1..count {
        let (s, e) = follow(prev_end, settings);
        out.push(window(s, e, tz));
        prev_end = e;
    }
    Ok(out)
}

/// Wie [`generate`], aber für Folge-Cycles nach einem bestehenden Cycle, der lokal am
/// Datum `last_end` endet (z. B. `utc_to_local_date(last.ends_at, tz)`).
pub fn generate_following(
    settings: &CycleSettings,
    tz: Tz,
    last_end: NaiveDate,
    count: usize,
) -> Result<Vec<CycleWindow>, DomainError> {
    settings.validate()?;
    let mut out = Vec::with_capacity(count);
    let mut prev_end = last_end;
    for _ in 0..count {
        let (s, e) = follow(prev_end, settings);
        out.push(window(s, e, tz));
        prev_end = e;
    }
    Ok(out)
}

/// Lokales Kalenderdatum eines Zeitpunkts in `tz` (docs/04 §1.4, `utcToLocalDate`).
pub fn utc_to_local_date(instant: Timestamp, tz: Tz) -> NaiveDate {
    instant.with_timezone(&tz).date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;
    use chrono_tz::{America, Australia, Europe, Pacific};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }
    fn utc(s: &str) -> Timestamp {
        chrono::DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&Utc)
    }
    fn s(dur: u32, cool: u32, day: u8) -> CycleSettings {
        CycleSettings {
            duration_weeks: dur,
            cooldown_weeks: cool,
            start_day: day,
        }
    }

    /// Beispiel aus docs/04 §1.4 (Europe/Berlin, Zeitumstellung am 2026-10-25).
    #[test]
    fn berlin_spec_example_across_dst_end() {
        let w = generate(&s(2, 0, 1), Europe::Berlin, d(2026, 10, 5), 3).unwrap();
        let exp = [
            ("2026-10-04T22:00:00Z", "2026-10-18T22:00:00Z"),
            ("2026-10-18T22:00:00Z", "2026-11-01T23:00:00Z"),
            ("2026-11-01T23:00:00Z", "2026-11-15T23:00:00Z"),
        ];
        for (win, (a, b)) in w.iter().zip(exp) {
            assert_eq!((win.starts_at, win.ends_at), (utc(a), utc(b)));
        }
        assert_eq!(w[1].start_date, d(2026, 10, 19));
        assert_eq!(w[1].end_date, d(2026, 11, 2));
        // Cycle 13 dauert wegen Rückstellung 337 statt 336 Stunden.
        assert_eq!((w[1].ends_at - w[1].starts_at).num_hours(), 337);
    }

    #[test]
    fn berlin_cooldown_one_week() {
        let w = generate(&s(2, 1, 1), Europe::Berlin, d(2026, 10, 5), 2).unwrap();
        assert_eq!(w[1].start_date, d(2026, 10, 26));
        assert_eq!(w[0].end_date, d(2026, 10, 19));
        assert_eq!(w[1].starts_at, utc("2026-10-25T23:00:00Z")); // schon Winterzeit
    }

    #[test]
    fn berlin_dst_start_in_spring() {
        // Umstellung 2026-03-29 (Sonntag): Cycle Mo 03-23 .. Mo 04-06.
        let w = generate(&s(2, 0, 1), Europe::Berlin, d(2026, 3, 23), 1).unwrap();
        assert_eq!(w[0].starts_at, utc("2026-03-22T23:00:00Z")); // UTC+1
        assert_eq!(w[0].ends_at, utc("2026-04-05T22:00:00Z")); // UTC+2
        assert_eq!((w[0].ends_at - w[0].starts_at).num_hours(), 335);
    }

    #[test]
    fn los_angeles_fall_back_and_spring_forward() {
        // Rückstellung 2026-11-01.
        let w = generate(&s(2, 0, 1), America::Los_Angeles, d(2026, 10, 26), 2).unwrap();
        assert_eq!(w[0].starts_at, utc("2026-10-26T07:00:00Z")); // PDT
        assert_eq!(w[0].ends_at, utc("2026-11-09T08:00:00Z")); // PST
        assert_eq!(w[1].starts_at, w[0].ends_at);
        // Vorstellung 2026-03-08.
        let w = generate(&s(2, 0, 1), America::Los_Angeles, d(2026, 3, 2), 1).unwrap();
        assert_eq!(w[0].starts_at, utc("2026-03-02T08:00:00Z")); // PST
        assert_eq!(w[0].ends_at, utc("2026-03-16T07:00:00Z")); // PDT
        assert_eq!((w[0].ends_at - w[0].starts_at).num_hours(), 335);
    }

    #[test]
    fn default_team_timezone_is_los_angeles_one_week_cycles() {
        let w = generate(
            &s(1, 0, 1),
            crate::time::DEFAULT_TEAM_TIMEZONE,
            d(2026, 10, 26),
            3,
        )
        .unwrap();
        let ends: Vec<_> = w.iter().map(|x| x.ends_at).collect();
        assert_eq!(
            ends,
            [
                utc("2026-11-02T08:00:00Z"), // 00:00 PST (Umstellung am 11-01)
                utc("2026-11-09T08:00:00Z"),
                utc("2026-11-16T08:00:00Z")
            ]
        );
    }

    /// Mitternacht existiert nicht (Lücke): erste gültige Minute danach.
    #[test]
    fn nonexistent_midnight_uses_first_valid_minute() {
        // São Paulo: Sommerzeit begann 2018-11-04 um 00:00 -> 01:00 (UTC-3 -> UTC-2).
        assert_eq!(
            local_midnight_to_utc(d(2018, 11, 4), America::Sao_Paulo),
            utc("2018-11-04T03:00:00Z") // 01:00 lokal bei UTC-2
        );
        // Havana 2026: Lücke am 2026-03-08 um 00:00 -> 01:00 (UTC-5 -> UTC-4).
        assert_eq!(
            local_midnight_to_utc(d(2026, 3, 8), America::Havana),
            utc("2026-03-08T05:00:00Z")
        );
        let w = generate(&s(1, 0, 0), America::Havana, d(2026, 3, 1), 2).unwrap();
        assert_eq!(w[1].starts_at, utc("2026-03-08T05:00:00Z"));
        assert_eq!(w[0].ends_at, w[1].starts_at);
    }

    /// Mitternacht kommt doppelt vor: das frühere Vorkommen gilt.
    #[test]
    fn ambiguous_midnight_uses_first_occurrence() {
        // Havana: 2026-11-01 01:00 -> 00:00 (Rückstellung), 00:00 tritt zweimal auf.
        assert_eq!(
            local_midnight_to_utc(d(2026, 11, 1), America::Havana),
            utc("2026-11-01T04:00:00Z") // UTC-4 (Sommerzeit)
        );
    }

    #[test]
    fn first_cycle_off_start_day_ends_on_next_regular_start_day_at_least_a_week_away() {
        // Mittwoch 2026-10-07, Starttag Montag: erster Cycle bis Mo 10-19 (nicht 10-12).
        let w = generate(&s(2, 0, 1), Europe::Berlin, d(2026, 10, 7), 3).unwrap();
        assert_eq!(w[0].start_date, d(2026, 10, 7));
        assert_eq!(w[0].end_date, d(2026, 10, 19));
        assert_eq!(w[1].start_date, d(2026, 10, 19));
        assert_eq!(w[1].end_date, d(2026, 11, 2));
        // Montag-Start, Starttag Montag: regulär.
        let w = generate(&s(1, 0, 1), Europe::Berlin, d(2026, 10, 5), 1).unwrap();
        assert_eq!(w[0].end_date, d(2026, 10, 12));
    }

    #[test]
    fn generate_following_continues_after_existing_cycle() {
        let w = generate_following(&s(2, 1, 1), Europe::Berlin, d(2026, 10, 19), 2).unwrap();
        assert_eq!(w[0].start_date, d(2026, 10, 26));
        assert_eq!(w[0].end_date, d(2026, 11, 9));
        assert_eq!(w[1].start_date, d(2026, 11, 16));
        // Vorheriges Ende fällt nicht auf Starttag (z. B. verschobener Cycle): nächster Starttag.
        let w = generate_following(&s(1, 0, 1), Europe::Berlin, d(2026, 10, 21), 1).unwrap();
        assert_eq!(w[0].start_date, d(2026, 10, 26));
    }

    #[test]
    fn utc_roundtrip_to_local_date() {
        let w = generate(&s(2, 0, 1), Europe::Berlin, d(2026, 10, 5), 4).unwrap();
        for x in &w {
            assert_eq!(utc_to_local_date(x.starts_at, Europe::Berlin), x.start_date);
            assert_eq!(utc_to_local_date(x.ends_at, Europe::Berlin), x.end_date);
        }
    }

    /// Invarianten über viele Cycles und Zeitzonen mit und ohne DST (Nord/Süd, halbstündig).
    #[test]
    fn invariants_hold_across_zones_and_years() {
        let zones = [
            Europe::Berlin,
            America::Los_Angeles,
            America::New_York,
            Australia::Sydney,
            Australia::Lord_Howe,
            Pacific::Auckland,
            chrono_tz::Asia::Kolkata,
            chrono_tz::UTC,
        ];
        for tz in zones {
            for (dur, cool, day) in [(1, 0, 1), (2, 0, 1), (2, 1, 0), (3, 0, 6)] {
                let from = next_weekday_on_or_after(d(2026, 1, 5), day);
                let w = generate(&s(dur, cool, day), tz, from, 60).unwrap();
                for pair in w.windows(2) {
                    assert!(pair[0].starts_at < pair[0].ends_at, "{tz}");
                    if cool == 0 {
                        assert_eq!(pair[0].ends_at, pair[1].starts_at, "{tz}");
                    } else {
                        assert!(pair[0].ends_at < pair[1].starts_at, "{tz}");
                    }
                }
                for x in &w {
                    // Lokal immer Mitternacht, außer wenn Mitternacht in einer Lücke liegt.
                    for (date, ts) in [(x.start_date, x.starts_at), (x.end_date, x.ends_at)] {
                        let local = ts.with_timezone(&tz);
                        assert_eq!(local.date_naive(), date, "{tz} {date}");
                        assert!(local.hour() <= 1, "{tz} {date}: {local}");
                    }
                    assert_eq!(
                        x.start_date.weekday().num_days_from_sunday() as u8,
                        day,
                        "{tz}"
                    );
                    let hours = (x.ends_at - x.starts_at).num_hours();
                    let nominal = 7 * 24 * dur as i64;
                    assert!((hours - nominal).abs() <= 2, "{tz}: {hours} vs {nominal}");
                }
            }
        }
    }

    #[test]
    fn rejects_out_of_range_settings() {
        for bad in [s(0, 0, 1), s(9, 0, 1), s(2, 9, 1), s(2, 0, 7)] {
            assert!(generate(&bad, Europe::Berlin, d(2026, 10, 5), 1).is_err());
            assert!(generate_following(&bad, Europe::Berlin, d(2026, 10, 5), 1).is_err());
        }
        assert!(
            generate(&s(2, 0, 1), Europe::Berlin, d(2026, 10, 5), 0)
                .unwrap()
                .is_empty()
        );
    }
}
