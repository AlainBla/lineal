//! SLA-Fristen, Arbeitstage und Risikostufen (docs/03 §11).

use super::cycle_calendar::local_to_utc;
use crate::enums::SlaType;
use crate::time::Timestamp;
use chrono::{Datelike, Duration, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

/// Obergrenze für Arbeitstage-Rechnung (Schutz vor absurden Eingaben).
const MAX_BUSINESS_DAYS: i64 = 10_000;

/// Arbeitswoche (`Team.slaWorkWeek`, docs/03 §11.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkWeek {
    /// Montag bis Freitag (Default).
    #[default]
    MonFri,
    /// Sonntag bis Donnerstag.
    SunThu,
}

impl WorkWeek {
    /// Ist `day` ein Arbeitstag?
    pub fn is_workday(&self, day: Weekday) -> bool {
        match self {
            Self::MonFri => !matches!(day, Weekday::Sat | Weekday::Sun),
            Self::SunThu => !matches!(day, Weekday::Fri | Weekday::Sat),
        }
    }
}

/// `slaStatus` (docs/03 §11.4); Wire-Schreibweise camelCase wie in docs/05 §1.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SlaStatus {
    /// Mehr als eine Woche bis zum Breach.
    LowRisk,
    /// Weniger als eine Woche.
    MediumRisk,
    /// Weniger als 24 h.
    HighRisk,
    /// Frist überschritten (offenes Issue).
    Breached,
    /// Geschlossen vor der Frist.
    Completed,
    /// Geschlossen nach der Frist.
    Failed,
}

/// Fälligkeit und Risikoschwellen eines Issues (`slaBreachesAt`, `slaMediumRiskAt`, `slaHighRiskAt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlaThresholds {
    /// Ab hier Risiko „mittel“.
    pub medium_risk_at: Timestamp,
    /// Ab hier Risiko „hoch“.
    pub high_risk_at: Timestamp,
}

/// Fälligkeitszeitpunkt der SLA (docs/03 §11.2/§11.3, `addBusinessDays`).
///
/// `amount` ist bei `all` die Dauer in **Millisekunden**, bei `onlyBusinessDays` die Anzahl
/// **Arbeitstage** (wie `SlaConfiguration.sla`). Arbeitstage werden in `tz` auf Kalendertagen
/// gerechnet und behalten die lokale Uhrzeit (über Sommerzeitwechsel hinweg); fällt die
/// Ziel-Wandzeit in eine Lücke, gilt die erste gültige Minute danach. Startet die Uhr an einem
/// Nicht-Arbeitstag, zählt der nächste Arbeitstag als erster (gleiche Uhrzeit, Festlegung nach
/// dem Pseudocode in 11.3). Negative Werte zählen als 0.
pub fn breach_at(
    start: Timestamp,
    amount: i64,
    sla_type: SlaType,
    tz: Tz,
    week: WorkWeek,
) -> Timestamp {
    match sla_type {
        SlaType::All => Duration::try_milliseconds(amount.max(0))
            .and_then(|d| start.checked_add_signed(d))
            .unwrap_or(start),
        SlaType::OnlyBusinessDays => add_business_days(start, amount, tz, week),
    }
}

/// `start` plus `n` Arbeitstage in `tz` (gleiche lokale Uhrzeit).
pub fn add_business_days(start: Timestamp, n: i64, tz: Tz, week: WorkWeek) -> Timestamp {
    shift_business_days(start, n.clamp(0, MAX_BUSINESS_DAYS), 1, tz, week)
}

/// `end` minus `n` Arbeitstage in `tz` (Gegenstück zu [`add_business_days`]).
pub fn sub_business_days(end: Timestamp, n: i64, tz: Tz, week: WorkWeek) -> Timestamp {
    shift_business_days(end, n.clamp(0, MAX_BUSINESS_DAYS), -1, tz, week)
}

fn shift_business_days(t: Timestamp, n: i64, dir: i64, tz: Tz, week: WorkWeek) -> Timestamp {
    let mut local = t.with_timezone(&tz).naive_local();
    let mut moved = 0;
    while moved < n {
        local += Duration::days(dir);
        if week.is_workday(local.weekday()) {
            moved += 1;
        }
    }
    if n == 0 {
        return t;
    }
    local_to_utc(local, tz)
}

/// Risikoschwellen (docs/03 §11.1): Medium = Breach − 7 Tage, High = Breach − 24 h; bei
/// `onlyBusinessDays` 5 bzw. 1 Arbeitstag davor. Nie vor `started_at`.
pub fn risk_levels(
    started_at: Timestamp,
    breaches_at: Timestamp,
    sla_type: SlaType,
    tz: Tz,
    week: WorkWeek,
) -> SlaThresholds {
    let (medium, high) = match sla_type {
        SlaType::All => (
            breaches_at - Duration::days(7),
            breaches_at - Duration::hours(24),
        ),
        SlaType::OnlyBusinessDays => (
            sub_business_days(breaches_at, 5, tz, week),
            sub_business_days(breaches_at, 1, tz, week),
        ),
    };
    SlaThresholds {
        medium_risk_at: medium.max(started_at),
        high_risk_at: high.max(started_at),
    }
}

/// `slaStatus` (docs/03 §11.4). `closed_at` = Abschlusszeit (Status geschlossen) oder `None`
/// für offene Issues.
pub fn status(
    now: Timestamp,
    breaches_at: Timestamp,
    thresholds: &SlaThresholds,
    closed_at: Option<Timestamp>,
) -> SlaStatus {
    if let Some(closed) = closed_at {
        return if closed < breaches_at {
            SlaStatus::Completed
        } else {
            SlaStatus::Failed
        };
    }
    if now >= breaches_at {
        SlaStatus::Breached
    } else if now >= thresholds.high_risk_at {
        SlaStatus::HighRisk
    } else if now >= thresholds.medium_risk_at {
        SlaStatus::MediumRisk
    } else {
        SlaStatus::LowRisk
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use chrono_tz::{America, Europe};

    fn utc(s: &str) -> Timestamp {
        chrono::DateTime::parse_from_rfc3339(s)
            .unwrap()
            .with_timezone(&Utc)
    }
    const HOUR: i64 = 3_600_000;

    #[test]
    fn calendar_sla_is_plain_duration() {
        let s = utc("2026-10-10T07:15:00Z");
        let b = breach_at(s, 24 * HOUR, SlaType::All, Europe::Berlin, WorkWeek::MonFri);
        assert_eq!(b, utc("2026-10-11T07:15:00Z"));
        // Ein Kalender-SLA ist absolute Zeit: über die Zeitumstellung 25 h Wanduhr.
        let s = utc("2026-10-24T22:00:00Z");
        let b = breach_at(s, 24 * HOUR, SlaType::All, Europe::Berlin, WorkWeek::MonFri);
        assert_eq!(b, utc("2026-10-25T22:00:00Z"));
        assert_eq!(
            breach_at(s, -5, SlaType::All, Europe::Berlin, WorkWeek::MonFri),
            s
        );
        assert_eq!(
            breach_at(s, i64::MAX, SlaType::All, Europe::Berlin, WorkWeek::MonFri),
            s
        );
    }

    #[test]
    fn business_days_skip_weekend_and_keep_local_time_over_dst() {
        // Fr 2026-10-23 10:00 CEST + 3 Arbeitstage = Mi 10-28 10:00 CET (Umstellung am 10-25).
        let s = utc("2026-10-23T08:00:00Z");
        let b = breach_at(
            s,
            3,
            SlaType::OnlyBusinessDays,
            Europe::Berlin,
            WorkWeek::MonFri,
        );
        assert_eq!(b, utc("2026-10-28T09:00:00Z"));
        // +1 ab Freitag = Montag.
        let b = breach_at(
            s,
            1,
            SlaType::OnlyBusinessDays,
            Europe::Berlin,
            WorkWeek::MonFri,
        );
        assert_eq!(b, utc("2026-10-26T09:00:00Z"));
        // +5 = nächster Freitag.
        let b = breach_at(
            s,
            5,
            SlaType::OnlyBusinessDays,
            Europe::Berlin,
            WorkWeek::MonFri,
        );
        assert_eq!(b, utc("2026-10-30T09:00:00Z"));
        // 0 Tage = Start.
        assert_eq!(
            breach_at(
                s,
                0,
                SlaType::OnlyBusinessDays,
                Europe::Berlin,
                WorkWeek::MonFri
            ),
            s
        );
    }

    #[test]
    fn business_days_start_on_weekend() {
        // Sa 2026-10-24 12:00 CEST, +1 => Mo 10-26 12:00 CET.
        let s = utc("2026-10-24T10:00:00Z");
        let b = breach_at(
            s,
            1,
            SlaType::OnlyBusinessDays,
            Europe::Berlin,
            WorkWeek::MonFri,
        );
        assert_eq!(b, utc("2026-10-26T11:00:00Z"));
    }

    #[test]
    fn sun_thu_week() {
        // Do 2026-10-22 10:00 CEST, +1 => So 10-25 10:00 CET (Fr/Sa übersprungen).
        let s = utc("2026-10-22T08:00:00Z");
        let b = breach_at(
            s,
            1,
            SlaType::OnlyBusinessDays,
            Europe::Berlin,
            WorkWeek::SunThu,
        );
        assert_eq!(b, utc("2026-10-25T09:00:00Z"));
        assert!(
            WorkWeek::SunThu.is_workday(Weekday::Sun) && !WorkWeek::SunThu.is_workday(Weekday::Fri)
        );
        assert!(
            WorkWeek::MonFri.is_workday(Weekday::Fri) && !WorkWeek::MonFri.is_workday(Weekday::Sun)
        );
    }

    #[test]
    fn business_days_in_dst_gap_use_first_valid_minute() {
        // LA: Sa 2026-03-07 02:30 PST (10:30Z), SunThu +1 => So 03-08 02:30 existiert nicht -> 03:00 PDT.
        let s = utc("2026-03-07T10:30:00Z");
        let b = breach_at(
            s,
            1,
            SlaType::OnlyBusinessDays,
            America::Los_Angeles,
            WorkWeek::SunThu,
        );
        assert_eq!(b, utc("2026-03-08T10:00:00Z"));
    }

    #[test]
    fn huge_business_day_counts_are_clamped() {
        let s = utc("2026-10-23T08:00:00Z");
        let b = breach_at(
            s,
            i64::MAX,
            SlaType::OnlyBusinessDays,
            Europe::Berlin,
            WorkWeek::MonFri,
        );
        assert!(b > s);
    }

    #[test]
    fn calendar_risk_levels() {
        let start = utc("2026-10-01T00:00:00Z");
        let breach = utc("2026-10-30T12:00:00Z");
        let r = risk_levels(
            start,
            breach,
            SlaType::All,
            Europe::Berlin,
            WorkWeek::MonFri,
        );
        assert_eq!(r.medium_risk_at, utc("2026-10-23T12:00:00Z"));
        assert_eq!(r.high_risk_at, utc("2026-10-29T12:00:00Z"));
    }

    #[test]
    fn risk_levels_never_before_start() {
        let start = utc("2026-10-29T00:00:00Z");
        let breach = utc("2026-10-29T12:00:00Z");
        let r = risk_levels(
            start,
            breach,
            SlaType::All,
            Europe::Berlin,
            WorkWeek::MonFri,
        );
        assert_eq!((r.medium_risk_at, r.high_risk_at), (start, start));
    }

    #[test]
    fn business_risk_levels_count_back_in_workdays() {
        // Breach Fr 2026-10-30 10:00 CET; Medium 5 AT davor = Fr 10-23 10:00 CEST; High = Do 10-29.
        let start = utc("2026-10-01T00:00:00Z");
        let breach = utc("2026-10-30T09:00:00Z");
        let r = risk_levels(
            start,
            breach,
            SlaType::OnlyBusinessDays,
            Europe::Berlin,
            WorkWeek::MonFri,
        );
        assert_eq!(r.medium_risk_at, utc("2026-10-23T08:00:00Z"));
        assert_eq!(r.high_risk_at, utc("2026-10-29T09:00:00Z"));
        // Breach Mo: 1 Arbeitstag davor = Freitag.
        let breach = utc("2026-10-26T09:00:00Z");
        let r = risk_levels(
            start,
            breach,
            SlaType::OnlyBusinessDays,
            Europe::Berlin,
            WorkWeek::MonFri,
        );
        assert_eq!(r.high_risk_at, utc("2026-10-23T08:00:00Z"));
    }

    #[test]
    fn status_table() {
        let breach = utc("2026-10-30T12:00:00Z");
        let th = SlaThresholds {
            medium_risk_at: utc("2026-10-23T12:00:00Z"),
            high_risk_at: utc("2026-10-29T12:00:00Z"),
        };
        let st = |now: &str, closed: Option<&str>| status(utc(now), breach, &th, closed.map(utc));
        assert_eq!(st("2026-10-10T00:00:00Z", None), SlaStatus::LowRisk);
        assert_eq!(st("2026-10-23T11:59:59Z", None), SlaStatus::LowRisk);
        assert_eq!(st("2026-10-23T12:00:00Z", None), SlaStatus::MediumRisk);
        assert_eq!(st("2026-10-29T12:00:00Z", None), SlaStatus::HighRisk);
        assert_eq!(st("2026-10-30T11:59:59Z", None), SlaStatus::HighRisk);
        assert_eq!(st("2026-10-30T12:00:00Z", None), SlaStatus::Breached);
        assert_eq!(
            st("2026-11-30T12:00:00Z", Some("2026-10-30T11:00:00Z")),
            SlaStatus::Completed
        );
        assert_eq!(
            st("2026-11-30T12:00:00Z", Some("2026-10-30T12:00:00Z")),
            SlaStatus::Failed
        );
        assert_eq!(
            st("2026-10-10T00:00:00Z", Some("2026-11-01T00:00:00Z")),
            SlaStatus::Failed
        );
    }

    #[test]
    fn status_wire_format() {
        assert_eq!(
            serde_json::to_string(&SlaStatus::HighRisk).unwrap(),
            "\"highRisk\""
        );
        assert_eq!(
            serde_json::to_string(&SlaStatus::Breached).unwrap(),
            "\"breached\""
        );
    }
}
