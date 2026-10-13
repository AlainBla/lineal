//! „Remind me“ (docs/05 §6.10, F.10): persönliche Erinnerung an Issue, Projekt, Initiative oder
//! Dokument. Setzen/Entfernen wirkt nur auf die eigene Erinnerung und nur an sichtbaren
//! Objekten; der Job [`App::create_reminder_notifications`] erzeugt bei Fälligkeit
//! `issueReminder`/`projectReminder`/`initiativeReminder`/`documentReminder` (auch für die eigene
//! Aktion – Ausnahme von der Self-Suppression, deshalb ohne Akteur).

use super::fanout::Fan;
use super::{issue_not_found, me, require_write};
use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::notification::Reminder;
use chrono::{DateTime, Duration, Utc};
use lineal_domain::enums::UserRole;
use lineal_domain::ids::{OrgId, ProjectId};
use serde_json::json;
use uuid::Uuid;

/// Höchstzahl je Job-Lauf und Organisation.
const BATCH: i64 = 500;
/// Spätester Zeitpunkt (Tage ab jetzt).
const MAX_DAYS: i64 = 366 * 5;

fn kind_for(target_type: &str) -> Option<&'static str> {
    Some(match target_type {
        "issue" => "issueReminder",
        "project" => "projectReminder",
        "initiative" => "initiativeReminder",
        "document" => "documentReminder",
        _ => return None,
    })
}

fn target_not_found() -> AppError {
    AppError::not_found("Reminder target not found.")
}

impl App {
    /// Löst das Ziel auf und prüft die Sichtbarkeit (unsichtbar → `not_found`). Liefert die
    /// UUID des Objekts.
    async fn reminder_target(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        ctx: &RequestCtx,
        target_type: &str,
        target: &str,
    ) -> AppResult<Uuid> {
        let visible = snap.visible_team_ids();
        let guest = snap.actor.role == UserRole::Guest;
        let uuid = || Uuid::parse_str(target.trim()).map_err(|_| target_not_found());
        match target_type {
            "issue" => Ok(self
                .issue_for_subscription(tx, snap, ctx, target, Action::ViewTeam)
                .await
                .map_err(|e| {
                    if e.kind == crate::error::ErrorKind::NotFound {
                        issue_not_found(target)
                    } else {
                        e
                    }
                })?
                .id
                .0),
            "project" => {
                let id = uuid()?;
                if !tx
                    .notifications()
                    .reminder_target_alive("project", id)
                    .await?
                {
                    return Err(target_not_found());
                }
                let teams = tx.notifications().project_team_ids(ProjectId(id)).await?;
                let ok = if teams.is_empty() {
                    !guest
                } else {
                    teams.iter().any(|t| visible.contains(t))
                };
                if ok { Ok(id) } else { Err(target_not_found()) }
            }
            "initiative" => {
                let id = uuid()?;
                snap.require_workspace(ctx, Action::ViewInitiatives)
                    .map_err(|_| target_not_found())?;
                if !tx
                    .notifications()
                    .reminder_target_alive("initiative", id)
                    .await?
                {
                    return Err(target_not_found());
                }
                Ok(id)
            }
            "document" => self.document_for_subscription(tx, snap, uuid()?).await,
            _ => Err(AppError::invalid_request(
                "`targetType` must be one of issue, project, initiative, document.",
            )
            .with_details(json!({ "field": "targetType" }))),
        }
    }

    /// Setzt die Erinnerung (ersetzt eine vorhandene) bzw. entfernt sie (`remind_at = None`).
    /// Liefert die neue Erinnerung, bei Entfernen `None`.
    pub async fn set_reminder(
        &self,
        ctx: &RequestCtx,
        target_type: &str,
        target: &str,
        remind_at: Option<DateTime<Utc>>,
    ) -> AppResult<Option<Reminder>> {
        require_write(ctx)?;
        let user = me(ctx)?;
        let now = self.clock.now();
        if let Some(at) = remind_at {
            if at <= now {
                return Err(
                    AppError::invalid_request("`remindAt` must be in the future.")
                        .with_details(json!({ "field": "remindAt" })),
                );
            }
            if at > now + Duration::days(MAX_DAYS) {
                return Err(
                    AppError::invalid_request("`remindAt` is too far in the future.")
                        .with_details(json!({ "field": "remindAt" })),
                );
            }
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let id = self
            .reminder_target(&mut *tx, &snap, ctx, target_type, target)
            .await?;
        let out = match remind_at {
            Some(at) => Some(
                tx.notifications()
                    .set_reminder(self.ids.new_uuid(), user, target_type, id, at, now)
                    .await?,
            ),
            None => {
                tx.notifications()
                    .clear_reminder(user, target_type, id)
                    .await?;
                None
            }
        };
        tx.commit().await?;
        Ok(out)
    }

    /// Eigene offene Erinnerung am Objekt (für die Anzeige im Menü).
    pub async fn get_reminder(
        &self,
        ctx: &RequestCtx,
        target_type: &str,
        target: &str,
    ) -> AppResult<Option<Reminder>> {
        let user = me(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let id = self
            .reminder_target(&mut *tx, &snap, ctx, target_type, target)
            .await?;
        tx.notifications().get_reminder(user, target_type, id).await
    }

    /// Job `reminders`: löst fällige Erinnerungen aus und legt je eine Notification an
    /// (idempotent über `reminder:<id>:<Zeitpunkt>`). Verliert der User den Zugriff oder das
    /// Objekt, wird die Erinnerung ohne Notification als erledigt markiert. Liefert die Zahl der
    /// neuen Notifications.
    pub async fn create_reminder_notifications(&self, now: DateTime<Utc>) -> AppResult<usize> {
        let orgs: Vec<OrgId> = {
            let mut tx = self.store.begin_system().await?;
            tx.notifications()
                .list_org_ids()
                .await?
                .into_iter()
                .map(OrgId)
                .collect()
        };
        let mut total = 0;
        for org in orgs {
            let mut tx = self.store.begin(org).await?;
            let due = tx.notifications().list_due_reminders(now, BATCH).await?;
            if due.is_empty() {
                continue;
            }
            for r in &due {
                tx.notifications().mark_reminder_fired(r.id, now).await?;
            }
            let mut fan = Fan::new(&mut *tx, now, None).await?;
            for r in due {
                let Some(kind) = kind_for(&r.target_type) else {
                    continue;
                };
                if !fan.reset_for_reminder(&r.target_type, r.target_id).await? {
                    continue;
                }
                fan.add(r.user_id, kind);
                total += fan
                    .deliver(
                        self.ids.as_ref(),
                        &format!("reminder:{}:{}", r.id, r.remind_at.timestamp()),
                    )
                    .await?;
            }
            drop(fan);
            tx.commit().await?;
        }
        Ok(total)
    }
}
