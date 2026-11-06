//! Kalender-Feed der Meilensteine (docs/04 §3.7): Kalender-Apps können sich nicht anmelden, deshalb
//! ein geheimer Link je Person. Er zeigt nur, was die Person gerade sehen darf, nur lesend, und
//! enthält bewusst keine Beschreibungen: Kalender liegen oft bei Dritten.

use super::audit::AuditEvent;
use super::auth::{hash_token, random_hex};
use super::project::ListProjectsInput;
use crate::app::App;
use crate::context::{Actor, RequestCtx, Scopes};
use crate::error::{AppError, AppResult};
use crate::ports::repos::user::PageRequest;
use lineal_domain::ids::UserId;
use lineal_domain::rules::ics::{IcsEvent, calendar};
use serde_json::json;

const TOKEN_PREFIX: &str = "lin_cal_";

/// Nur für die eigene Web-Session: Der Link ist ein Zugang.
fn owner(ctx: &RequestCtx) -> AppResult<UserId> {
    if !ctx.via_session {
        return Err(AppError::forbidden(
            "Calendar feeds can only be managed from a browser session.",
        ));
    }
    match ctx.actor {
        Actor::User { id, .. } => Ok(id),
        _ => Err(AppError::forbidden("Calendar feeds need a user.")),
    }
}

impl App {
    fn calendar_url(&self, token: &str) -> String {
        format!(
            "{}/calendar/{token}.ics",
            self.config.base_url.trim_end_matches('/')
        )
    }

    /// Ob die Person einen Feed hat. Den Link selbst kennt der Server nicht mehr.
    pub async fn calendar_feed_enabled(&self, ctx: &RequestCtx) -> AppResult<bool> {
        let user = owner(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        tx.auth().has_calendar_feed(user).await
    }

    /// Legt den Link an oder ersetzt ihn; der alte funktioniert danach nicht mehr. Der Klartext
    /// erscheint nur hier.
    pub async fn rotate_calendar_feed(&self, ctx: &RequestCtx) -> AppResult<String> {
        let user = owner(ctx)?;
        let token = format!("{TOKEN_PREFIX}{}", random_hex());
        let mut tx = self.store.begin(ctx.org_id).await?;
        tx.auth()
            .set_calendar_feed(user, &hash_token(&token), self.clock.now())
            .await?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new("calendar_feed.created", "user", Some(user.0), json!({})),
        )
        .await?;
        tx.commit().await?;
        Ok(self.calendar_url(&token))
    }

    /// Schaltet den Feed ab.
    pub async fn revoke_calendar_feed(&self, ctx: &RequestCtx) -> AppResult<()> {
        let user = owner(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        tx.auth().delete_calendar_feed(user).await?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new("calendar_feed.revoked", "user", Some(user.0), json!({})),
        )
        .await?;
        tx.commit().await
    }

    /// Der Kalender zu einem Token; `None` bei unbekanntem Token oder gesperrter Person.
    pub async fn calendar_feed_ics(&self, token: &str) -> AppResult<Option<String>> {
        let mut sys = self.store.begin_system().await?;
        let principal = sys
            .auth()
            .find_calendar_feed_principal(&hash_token(token))
            .await?;
        drop(sys);
        let Some(p) = principal else {
            return Ok(None);
        };
        let ctx = RequestCtx {
            scopes: Scopes {
                read: true,
                write: false,
                admin: false,
            },
            actor: p.actor,
            ..RequestCtx::system(p.org_id, "calendar-feed")
        };
        let mut tx = self.store.begin(ctx.org_id).await?;
        let url_key = tx
            .orgs()
            .get()
            .await?
            .ok_or_else(|| AppError::internal("workspace vanished"))?
            .url_key;
        drop(tx);
        let base = self.config.base_url.trim_end_matches('/').to_owned();
        let host = base.split("://").last().unwrap_or("lineal").to_owned();
        let mut events = Vec::new();
        let mut page = PageRequest {
            limit: 250,
            ..Default::default()
        };
        loop {
            let (result, views) = self
                .list_projects(
                    &ctx,
                    ListProjectsInput {
                        page,
                        ..Default::default()
                    },
                )
                .await?;
            for project in views {
                let (_, milestones) = self
                    .list_milestones(&ctx, &project.project.id.0.to_string())
                    .await?;
                for view in milestones {
                    let m = view.milestone;
                    let Some(date) = m.target_date else { continue };
                    let done = if m.completed_at.is_some() { "✓ " } else { "" };
                    events.push(IcsEvent {
                        uid: format!("milestone-{}@{host}", m.id.0),
                        date,
                        summary: format!("{done}{} ({})", m.name, project.project.name),
                        url: format!(
                            "{base}/{url_key}/project/{}/milestone/{}",
                            project.identifier, m.id.0
                        ),
                        stamp: m.updated_at,
                        sequence: m.version,
                    });
                }
            }
            match result.next {
                Some(cursor) => page.cursor = Some(cursor),
                None => break,
            }
        }
        events.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.uid.cmp(&b.uid)));
        Ok(Some(calendar("Lineal milestones", &events)))
    }
}
