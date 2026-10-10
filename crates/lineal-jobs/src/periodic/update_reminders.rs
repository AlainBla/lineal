//! Periodischer Job `update_reminders` (WP07, docs/04 §5.4): Erinnerungen an Status-Updates für
//! Lead (Projekt, Status `started`, nicht `indefinite`) bzw. Owner (Initiative, Status `Active`).
//!
//! * Konfiguration: Modus `followWorkspace` | `custom` | `never`, Kadenz in Wochen, Tag, Stunde
//!   (lokale Zeit des Empfängers, Fallback UTC).
//! * Keine Erinnerung, wenn in den letzten 24 Stunden oder seit dem fälligen Termin ein Update erschien.
//! * Nachfass-Erinnerungen nach 1 und weiteren 2 Arbeitstagen (Stufen 0, 1, 2); je Ziel, Termin und
//!   Stufe genau einmal (`update_reminder_sent`). Pro Lauf wird höchstens die höchste fällige,
//!   noch nicht gesendete Stufe erinnert.

use crate::{PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{
    DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc, Weekday,
};
use chrono_tz::Tz;
use lineal_app::ports::repos::project::{ReminderSettings, ReminderTarget};
use lineal_app::{App, AppResult, RequestCtx};
use std::time::Duration as StdDuration;

pub struct UpdateRemindersJob;

/// Arbeitstage-Versatz der drei Stufen (0, 1, 1 + 2).
const STAGE_WORKING_DAYS: [i64; 3] = [0, 1, 3];

fn weekday_of(name: &str) -> Option<Weekday> {
    match name {
        "Monday" => Some(Weekday::Mon),
        "Tuesday" => Some(Weekday::Tue),
        "Wednesday" => Some(Weekday::Wed),
        "Thursday" => Some(Weekday::Thu),
        "Friday" => Some(Weekday::Fri),
        "Saturday" => Some(Weekday::Sat),
        "Sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

fn local_to_utc(d: NaiveDate, hour: u32, tz: Tz) -> Option<DateTime<Utc>> {
    let t: NaiveDateTime = d.and_time(NaiveTime::from_hms_opt(hour, 0, 0)?);
    tz.from_local_datetime(&t)
        .earliest()
        .map(|x| x.with_timezone(&Utc))
}

/// Jüngster fälliger Termin `<= now`: Wochentag `day` um `hour` (lokal), alle `weeks` Wochen ab dem
/// ersten passenden Termin nach `since`. `None`, wenn noch kein Termin erreicht ist.
pub fn due_slot(
    now: DateTime<Utc>,
    since: DateTime<Utc>,
    day: Weekday,
    hour: u32,
    weeks: i64,
    tz: Tz,
) -> Option<DateTime<Utc>> {
    let weeks = weeks.max(1);
    let now_local = now.with_timezone(&tz).date_naive();
    // Letzter Termin `<= now` (bis zu 7 Tage zurück).
    let mut slot_date = None;
    for back in 0..=7 {
        let d = now_local - Duration::days(back);
        if d.weekday() == day
            && let Some(u) = local_to_utc(d, hour, tz)
            && u <= now
        {
            slot_date = Some(d);
            break;
        }
    }
    let slot_date = slot_date?;
    // Erster Termin ab `since`.
    let since_local = since.with_timezone(&tz).date_naive();
    let mut first = since_local;
    while first.weekday() != day {
        first += Duration::days(1);
    }
    if local_to_utc(first, hour, tz).is_some_and(|u| u < since) {
        first += Duration::days(7);
    }
    if slot_date < first {
        return None;
    }
    let elapsed_weeks = (slot_date - first).num_days() / 7;
    let aligned = slot_date - Duration::days((elapsed_weeks % weeks) * 7);
    local_to_utc(aligned, hour, tz)
}

/// `due + n` Arbeitstage (Samstag und Sonntag zählen nicht).
pub fn add_working_days(due: DateTime<Utc>, n: i64) -> DateTime<Utc> {
    let mut t = due;
    let mut left = n;
    while left > 0 {
        t += Duration::days(1);
        if !matches!(t.weekday(), Weekday::Sat | Weekday::Sun) {
            left -= 1;
        }
    }
    t
}

struct Config {
    weeks: i64,
    day: Weekday,
    hour: u32,
}

fn effective_config(t: &ReminderTarget, s: &ReminderSettings, initiative: bool) -> Option<Config> {
    let (org_weeks, org_day, org_hour) = if initiative {
        (
            s.initiative_frequency_in_weeks,
            s.initiative_day.as_str(),
            s.initiative_hour,
        )
    } else {
        (
            s.project_frequency_in_weeks,
            s.project_day.as_str(),
            s.project_hour,
        )
    };
    let (weeks, day, hour) = match t.mode.as_str() {
        "never" => return None,
        "custom" => (
            t.frequency_in_weeks.or(org_weeks),
            t.day.as_deref().unwrap_or(org_day),
            t.hour.unwrap_or(org_hour),
        ),
        _ => (org_weeks, org_day, org_hour),
    };
    let weeks = i64::from(weeks.filter(|w| *w > 0)?);
    Some(Config {
        weeks,
        day: weekday_of(day)?,
        hour: u32::try_from(hour).ok().filter(|h| *h < 24)?,
    })
}

#[async_trait]
impl PeriodicJob for UpdateRemindersJob {
    fn name(&self) -> &'static str {
        "update_reminders"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(StdDuration::from_secs(3600))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        let mut sys = app.store.begin_system().await?;
        let orgs = sys.projects().orgs_with_planning().await?;
        drop(sys);
        for org in orgs {
            let ctx = RequestCtx::system(org, "job-update-reminders");
            if let Err(e) = remind_org(app, &ctx, now).await {
                tracing::warn!(org = %org, error = %e, "update reminders failed");
            }
        }
        Ok(())
    }
}

async fn remind_org(app: &App, ctx: &RequestCtx, now: DateTime<Utc>) -> AppResult<()> {
    let mut tx = app.store.begin(ctx.org_id).await?;
    let Some(settings) = tx.projects().reminder_settings().await? else {
        return Ok(());
    };
    let initiatives_enabled = tx
        .orgs()
        .get()
        .await?
        .is_some_and(|o| o.initiatives_enabled);
    let mut work: Vec<(&'static str, ReminderTarget)> = tx
        .projects()
        .reminder_targets()
        .await?
        .into_iter()
        .map(|t| ("project", t))
        .collect();
    if initiatives_enabled {
        work.extend(
            tx.initiatives()
                .reminder_targets()
                .await?
                .into_iter()
                .map(|t| ("initiative", t)),
        );
    }
    for (kind, t) in work {
        let Some(recipient) = t.recipient_id else {
            continue;
        };
        if t.paused_until.is_some_and(|p| p > now) {
            continue;
        }
        let Some(cfg) = effective_config(&t, &settings, kind == "initiative") else {
            continue;
        };
        let tz = t
            .recipient_timezone
            .as_deref()
            .and_then(|z| z.parse::<Tz>().ok())
            .unwrap_or(chrono_tz::UTC);
        let Some(due) = due_slot(now, t.since, cfg.day, cfg.hour, cfg.weeks, tz) else {
            continue;
        };
        if let Some(last) = t.last_update_at
            && (last >= due || last > now - Duration::hours(24))
        {
            continue;
        }
        // Höchste fällige, noch nicht gesendete Stufe.
        for stage in (0..STAGE_WORKING_DAYS.len()).rev() {
            if now < add_working_days(due, STAGE_WORKING_DAYS[stage]) {
                continue;
            }
            let stage = stage as i32;
            if tx
                .projects()
                .reminder_sent(kind, t.target_id, due, stage)
                .await?
            {
                break;
            }
            tx.projects()
                .record_reminder(kind, t.target_id, recipient, due, stage, now)
                .await?;
            break;
        }
    }
    tx.commit().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).single().unwrap()
    }

    #[test]
    fn weekly_slot_is_the_last_matching_friday() {
        // Samstag 2026-10-10: letzter Freitag 14:00 war der 9. (UTC)
        let due = due_slot(
            utc(2026, 10, 10, 7),
            utc(2026, 1, 5, 9),
            Weekday::Fri,
            14,
            1,
            chrono_tz::UTC,
        );
        assert_eq!(due, Some(utc(2026, 10, 9, 14)));
        // Freitag vor 14:00 → Vorwoche
        let due = due_slot(
            utc(2026, 10, 9, 10),
            utc(2026, 1, 5, 9),
            Weekday::Fri,
            14,
            1,
            chrono_tz::UTC,
        );
        assert_eq!(due, Some(utc(2026, 10, 2, 14)));
    }

    #[test]
    fn biweekly_aligns_to_the_first_slot_and_local_time_counts() {
        // erster Freitag nach 2026-01-05 ist der 9.1.; alle 2 Wochen → 9.10. liegt 39 Wochen später (ungerade) → 2.10.
        let due = due_slot(
            utc(2026, 10, 10, 7),
            utc(2026, 1, 5, 9),
            Weekday::Fri,
            14,
            2,
            chrono_tz::UTC,
        );
        assert_eq!(due, Some(utc(2026, 10, 2, 14)));
        // Berlin (UTC+2 im Oktober): 14:00 lokal = 12:00 UTC
        let due = due_slot(
            utc(2026, 10, 10, 7),
            utc(2026, 1, 5, 9),
            Weekday::Fri,
            14,
            1,
            chrono_tz::Europe::Berlin,
        );
        assert_eq!(due, Some(utc(2026, 10, 9, 12)));
        // vor dem ersten Termin
        assert_eq!(
            due_slot(
                utc(2026, 1, 6, 9),
                utc(2026, 1, 5, 9),
                Weekday::Fri,
                14,
                1,
                chrono_tz::UTC
            ),
            None
        );
    }

    #[test]
    fn working_days_skip_weekends() {
        // Freitag + 1 Arbeitstag = Montag; + 3 = Mittwoch
        assert_eq!(
            add_working_days(utc(2026, 10, 9, 14), 1),
            utc(2026, 10, 12, 14)
        );
        assert_eq!(
            add_working_days(utc(2026, 10, 9, 14), 3),
            utc(2026, 10, 14, 14)
        );
        assert_eq!(
            add_working_days(utc(2026, 10, 9, 14), 0),
            utc(2026, 10, 9, 14)
        );
    }
}
