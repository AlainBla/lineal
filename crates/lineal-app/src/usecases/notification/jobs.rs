//! Periodische Aufgaben der Inbox: Wiedervorlage, Cap, Fälligkeits-Erinnerungen, E-Mail-Digest
//! (docs/05 §6.4, §6.8–6.10).

use super::fanout::Fan;
use super::kinds::{Urgency, category_of, channel_default, render, urgency_of};
use crate::app::App;
use crate::error::AppResult;
use crate::ports::Mail;
use crate::ports::repos::notification::{
    ChannelPreference, NotificationView, UserNotificationPrefs,
};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use lineal_domain::ids::{NotificationId, OrgId, UserId};
use std::collections::BTreeMap;
use std::str::FromStr;

/// Archivierte Notifications werden nach dieser Frist endgültig entfernt (docs/05 §6.4 Nr. 5).
pub const ARCHIVE_RETENTION_DAYS: i64 = 90;
/// Wie weit zurück der Digest ungelesene Notifications berücksichtigt.
const EMAIL_LOOKBACK_DAYS: i64 = 7;
const EMAIL_BATCH: i64 = 2000;

/// Zeitzone eines Users (Fallback UTC).
pub(crate) fn tz_of(tz: Option<&str>) -> Tz {
    tz.and_then(|t| Tz::from_str(t).ok()).unwrap_or(Tz::UTC)
}

/// Einstellungs-Schalter aus `User.notificationSettings`.
pub(crate) fn setting(prefs: &UserNotificationPrefs, key: &str, default: bool) -> bool {
    prefs
        .settings
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(default)
}

fn channel_enabled(prefs: &[ChannelPreference], category: &str, channel: &str) -> bool {
    prefs
        .iter()
        .find(|p| p.category == category && p.channel == channel)
        .map_or_else(|| channel_default(category, channel), |p| p.enabled)
}

/// Frühester Versandzeitpunkt einer Notification (docs/05 §6.9): Verzögerung nach Dringlichkeit;
/// niedrige Dringlichkeit, die außerhalb 08:00–18:00 bzw. am Wochenende fällig würde, geht am
/// nächsten Werktag um 08:00 (Zeitzone des Users) raus.
pub fn email_release_at(
    created_at: DateTime<Utc>,
    kind: &str,
    immediate_format: bool,
    hold_off_hours: bool,
    tz: Tz,
) -> DateTime<Utc> {
    let urgency = urgency_of(kind);
    let delay = if immediate_format {
        Duration::zero()
    } else {
        match urgency {
            Urgency::High => Duration::zero(),
            Urgency::Medium => Duration::minutes(5),
            Urgency::Low => Duration::minutes(30),
        }
    };
    let due = created_at + delay;
    if !hold_off_hours || urgency != Urgency::Low {
        return due;
    }
    let local = due.with_timezone(&tz);
    let weekend = matches!(local.weekday(), Weekday::Sat | Weekday::Sun);
    let hour = local.time();
    let open = NaiveTime::from_hms_opt(8, 0, 0).unwrap_or_default();
    let close = NaiveTime::from_hms_opt(18, 0, 0).unwrap_or_default();
    if !weekend && hour >= open && hour < close {
        return due;
    }
    let mut day = local.date_naive();
    if weekend || hour >= close {
        day = day.succ_opt().unwrap_or(day);
    }
    while matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
        day = day.succ_opt().unwrap_or(day);
    }
    tz.from_local_datetime(&day.and_time(open))
        .earliest()
        .map_or(due, |t| t.with_timezone(&Utc))
}

fn mail_for(prefs: &UserNotificationPrefs, items: &[&NotificationView], base: &str) -> Mail {
    let mut groups: BTreeMap<&str, Vec<&NotificationView>> = BTreeMap::new();
    for v in items {
        groups.entry(v.n.group_key.as_str()).or_default().push(v);
    }
    let mut body = String::from("You have new notifications in Lineal.\n");
    for views in groups.values() {
        let r = render(views[0], base);
        body.push_str(&format!("\n{}\n  {}\n", r.title, r.url));
        for v in views {
            body.push_str(&format!("  - {}\n", render(v, base).subtitle));
        }
    }
    let subject = if items.len() == 1 {
        render(items[0], base).subtitle
    } else {
        format!("{} new notifications in Lineal", items.len())
    };
    Mail {
        to: prefs.email.clone(),
        subject,
        body,
    }
}

impl App {
    /// Job `snooze`: setzt abgelaufene Snoozes wieder auf ungelesen.
    pub async fn unsnooze_due_notifications(&self, now: DateTime<Utc>) -> AppResult<u64> {
        let mut tx = self.store.begin_system().await?;
        let n = tx.notifications().unsnooze_due(now).await?;
        tx.commit().await?;
        Ok(n)
    }

    /// Job `notification_cap`: archiviert je User die ältesten Notifications über dem Limit
    /// und entfernt archivierte Einträge nach der Aufbewahrungsfrist.
    pub async fn enforce_notification_cap(&self, now: DateTime<Utc>) -> AppResult<u64> {
        let mut tx = self.store.begin_system().await?;
        let archived = tx.notifications().archive_over_cap(now).await?;
        tx.notifications()
            .purge_archived(now - Duration::days(ARCHIVE_RETENTION_DAYS))
            .await?;
        tx.commit().await?;
        Ok(archived)
    }

    async fn org_ids(&self) -> AppResult<Vec<OrgId>> {
        let mut tx = self.store.begin_system().await?;
        let ids = tx.notifications().list_org_ids().await?;
        Ok(ids.into_iter().map(OrgId).collect())
    }

    /// Job `due_reminders`: `issueDue` für offene Issues, die morgen fällig sind oder überfällig
    /// wurden, ab 09:00 in der Zeitzone des Assignees (nur mit aktivierten Erinnerungen).
    /// Idempotent über `(Issue, Fälligkeitsdatum, Stufe)`. Liefert die Zahl neuer Notifications.
    pub async fn create_due_reminders(&self, now: DateTime<Utc>) -> AppResult<usize> {
        let mut total = 0;
        for org in self.org_ids().await? {
            let mut tx = self.store.begin(org).await?;
            let horizon = now.date_naive() + Duration::days(2);
            let candidates = tx.notifications().list_due_candidates(horizon).await?;
            let mut fan = Fan::new(&mut *tx, now, None).await?;
            for c in candidates {
                let local = now.with_timezone(&tz_of(c.timezone.as_deref()));
                if local.time() < NaiveTime::from_hms_opt(9, 0, 0).unwrap_or_default() {
                    continue;
                }
                let today: NaiveDate = local.date_naive();
                let stage = if c.due_date == today + Duration::days(1) {
                    "soon"
                } else if c.due_date < today {
                    "overdue"
                } else {
                    continue;
                };
                fan.reset_for_issue_reminder(c.issue_id, c.team_id);
                fan.add(c.assignee_id, "issueDue");
                total += fan
                    .deliver(
                        self.ids.as_ref(),
                        &format!("due:{}:{}:{stage}", c.issue_id, c.due_date),
                    )
                    .await?;
            }
            drop(fan);
            tx.commit().await?;
        }
        Ok(total)
    }

    /// E-Mail-Digest (docs/05 §6.9): sendet fällige, ungelesene, noch nicht gemailte Notifications
    /// je User gebündelt über den `Mailer`. Liefert die Zahl versendeter E-Mails.
    pub async fn send_email_digests(&self, now: DateTime<Utc>) -> AppResult<usize> {
        let mut sent = 0;
        for org in self.org_ids().await? {
            let mut tx = self.store.begin(org).await?;
            let base = match tx.orgs().get().await? {
                Some(o) => format!(
                    "{}/{}",
                    self.config.base_url.trim_end_matches('/'),
                    o.url_key
                ),
                None => self.config.base_url.trim_end_matches('/').to_owned(),
            };
            let candidates = tx
                .notifications()
                .list_email_candidates(now - Duration::days(EMAIL_LOOKBACK_DAYS), now, EMAIL_BATCH)
                .await?;
            let mut by_user: BTreeMap<UserId, Vec<NotificationView>> = BTreeMap::new();
            for v in candidates {
                by_user.entry(v.n.user_id).or_default().push(v);
            }
            for (user, views) in by_user {
                let Some(prefs) = tx.notifications().get_prefs(user).await? else {
                    continue;
                };
                if !setting(&prefs, "emailEnabled", true) {
                    continue;
                }
                let channels = tx.notifications().list_channel_preferences(user).await?;
                let tz = tz_of(prefs.timezone.as_deref());
                let immediate = prefs.email_format == "immediate";
                let hold = setting(&prefs, "emailHoldOutsideWorkHours", true);
                let only_priority = setting(&prefs, "emailOnlyPriority", false);
                let due: Vec<&NotificationView> = views
                    .iter()
                    .filter(|v| channel_enabled(&channels, category_of(&v.n.kind), "email"))
                    .filter(|v| !only_priority || v.n.is_priority)
                    .filter(|v| {
                        email_release_at(v.n.created_at, &v.n.kind, immediate, hold, tz) <= now
                    })
                    .collect();
                if due.is_empty() {
                    continue;
                }
                match self.mailer.send(mail_for(&prefs, &due, &base)).await {
                    Ok(()) => {
                        let ids: Vec<NotificationId> = due.iter().map(|v| v.n.id).collect();
                        tx.notifications().mark_emailed(&ids, now).await?;
                        sent += 1;
                    }
                    Err(e) => {
                        tracing::error!(user = %user, error = %e, "notification email failed");
                    }
                }
            }
            tx.commit().await?;
        }
        Ok(sent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).single().unwrap()
    }

    #[test]
    fn delay_by_urgency() {
        let t = utc(2026, 10, 7, 10, 0); // Mittwoch
        let berlin = Tz::Europe__Berlin;
        assert_eq!(
            email_release_at(t, "issueSlaBreached", false, true, berlin),
            t
        );
        assert_eq!(
            email_release_at(t, "issueCommentMention", false, true, berlin),
            t + Duration::minutes(5)
        );
        assert_eq!(
            email_release_at(t, "issueNewComment", false, true, berlin),
            t + Duration::minutes(30)
        );
        assert_eq!(
            email_release_at(t, "issueNewComment", true, true, berlin),
            t
        );
    }

    #[test]
    fn low_urgency_outside_work_hours_waits_for_next_workday() {
        let berlin = Tz::Europe__Berlin;
        // Mittwoch 19:00 Berlin (= 17:00 UTC) -> Donnerstag 08:00 Berlin (= 06:00 UTC)
        let evening = utc(2026, 10, 7, 17, 0);
        assert_eq!(
            email_release_at(evening, "issueNewComment", false, true, berlin),
            utc(2026, 10, 8, 6, 0)
        );
        // Freitag Abend -> Montag 08:00 Berlin
        let friday = utc(2026, 10, 9, 17, 0);
        assert_eq!(
            email_release_at(friday, "issueNewComment", false, true, berlin),
            utc(2026, 10, 12, 6, 0)
        );
        // Montag 05:00 Berlin (vor 08:00) -> Montag 08:00
        let early = utc(2026, 10, 12, 3, 0);
        assert_eq!(
            email_release_at(early, "issueNewComment", false, true, berlin),
            utc(2026, 10, 12, 6, 0)
        );
        // Abschaltbar; mittlere Dringlichkeit ist nicht betroffen.
        assert_eq!(
            email_release_at(evening, "issueNewComment", false, false, berlin),
            evening + Duration::minutes(30)
        );
        assert_eq!(
            email_release_at(evening, "issueAssignedToYou", false, true, berlin),
            evening + Duration::minutes(5)
        );
    }
}
