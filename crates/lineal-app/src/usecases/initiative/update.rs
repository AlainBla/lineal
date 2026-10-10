//! usecases – Status-Updates von Initiativen (docs/04 §5): analog zu Projekten, ohne Meilensteine
//! und Fortschritt im Diff; erstes Update nur durch den Owner.

use super::{initiative_status_ref, require_enabled, require_view};
use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::initiative::{
    Initiative, InitiativePatch, InitiativeUpdate, InitiativeUpdateFilter, NewInitiativeUpdate,
};
use crate::ports::repos::user::{Page, PageRequest, User};
use crate::resolve;
use crate::usecases::project::emit;
use crate::usecases::project::pure::{Staleness, compute_diff, staleness};
use crate::usecases::project::update::reminder_cycle_days;
use chrono::{DateTime, Utc};
use lineal_domain::enums::{Health, InitiativeStatus};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{InitiativeId, InitiativeUpdateId};
use serde_json::{Value, json};
use uuid::Uuid;

/// Ein Status-Update einer Initiative mit abgeleiteten Feldern.
#[derive(Debug, Clone, PartialEq)]
pub struct InitiativeUpdateView {
    pub update: InitiativeUpdate,
    pub initiative: Initiative,
    pub user: Option<User>,
    pub is_stale: bool,
    pub comment_count: i64,
}

/// Filter von `get_status_updates` (Initiative).
#[derive(Debug, Clone, Default)]
pub struct ListInitiativeUpdatesInput {
    pub initiative: Option<String>,
    pub user: Option<String>,
    pub include_archived: bool,
    pub created_after: Option<DateTime<Utc>>,
    pub updated_after: Option<DateTime<Utc>>,
    pub page: PageRequest,
}

async fn snapshot(tx: &mut dyn Tx, i: &Initiative) -> AppResult<Value> {
    let owner_name = match i.owner_id {
        Some(id) => tx.users().get(id).await?.map(|u| u.name),
        None => None,
    };
    let (id, name, kind) = initiative_status_ref(i.status);
    Ok(json!({
        "status": { "id": id, "name": name, "type": kind },
        "targetDate": i.target_date.map(|d| d.to_string()),
        "targetDateResolution": i.target_date_resolution.map(|r| r.as_str()),
        "ownerId": i.owner_id.map(|u| u.to_string()),
        "ownerName": owner_name,
    }))
}

async fn view_of(
    tx: &mut dyn Tx,
    u: InitiativeUpdate,
    now: DateTime<Utc>,
) -> AppResult<InitiativeUpdateView> {
    let initiative = tx
        .initiatives()
        .get(u.initiative_id)
        .await?
        .ok_or_else(|| AppError::internal("initiative of update missing"))?;
    let settings = tx.initiatives().reminder_settings().await?;
    let latest = tx.initiatives().latest_update(initiative.id, None).await?;
    let expected =
        initiative.status == InitiativeStatus::Active && initiative.owner_id.is_some();
    let days = reminder_cycle_days(
        &initiative.update_reminder_mode,
        initiative.update_reminder_frequency_in_weeks,
        settings.and_then(|s| s.initiative_frequency_in_weeks),
    );
    let st = staleness(
        expected,
        days,
        latest.as_ref().map(|l| (l.created_at, l.health)),
        now,
    );
    let is_stale = latest.as_ref().is_some_and(|l| l.id == u.id)
        && matches!(st, Staleness::Missing | Staleness::Grey);
    let user = tx.users().get(u.user_id).await?;
    let comment_count = tx.initiatives().update_comment_count(u.id).await?;
    Ok(InitiativeUpdateView {
        update: u,
        initiative,
        user,
        is_stale,
        comment_count,
    })
}

async fn refresh_parent(
    tx: &mut dyn Tx,
    initiative: InitiativeId,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let latest = tx.initiatives().latest_update(initiative, None).await?;
    let patch = match latest {
        Some(l) => InitiativePatch {
            last_update_id: Some(Some(l.id)),
            health: Some(Some(l.health)),
            health_updated_at: Some(Some(l.created_at)),
            ..Default::default()
        },
        None => InitiativePatch {
            last_update_id: Some(None),
            health: Some(None),
            health_updated_at: Some(None),
            ..Default::default()
        },
    };
    tx.initiatives().update(initiative, &patch, now).await?;
    Ok(())
}

async fn resolve_update(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    reference: &str,
) -> AppResult<(InitiativeUpdate, Initiative)> {
    let nf = || {
        AppError::not_found(format!("Could not find status update \"{reference}\""))
            .with_details(json!({ "reference": reference, "type": "InitiativeUpdate" }))
    };
    let id = Uuid::parse_str(reference.trim()).map_err(|_| nf())?;
    let u = tx
        .initiatives()
        .get_update(InitiativeUpdateId(id))
        .await?
        .ok_or_else(nf)?;
    let i = tx
        .initiatives()
        .get(u.initiative_id)
        .await?
        .filter(|i| !i.trashed && resolve::initiative::is_visible(snap, i))
        .ok_or_else(nf)?;
    Ok((u, i))
}

impl App {
    /// `get_status_updates` (Initiative): Liste mit Filtern; Gäste erhalten `forbidden`.
    pub async fn list_initiative_updates(
        &self,
        ctx: &RequestCtx,
        input: ListInitiativeUpdatesInput,
    ) -> AppResult<(Page<InitiativeUpdate>, Vec<InitiativeUpdateView>)> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        require_view(&snap, ctx)?;
        require_enabled(&mut *tx).await?;
        let now = self.clock.now();
        let mut filter = InitiativeUpdateFilter {
            visible_teams: snap.visible_team_ids().into_iter().collect(),
            include_archived: input.include_archived,
            created_after: input.created_after,
            updated_after: input.updated_after,
            ..Default::default()
        };
        if let Some(i) = &input.initiative {
            filter.initiative =
                Some(resolve::initiative::resolve_in(&mut *tx, &snap, i).await?.id);
        }
        if let Some(u) = &input.user {
            filter.user = Some(resolve::user::resolve_in(&mut *tx, &snap, ctx, u).await?.id);
        }
        let page = tx.initiatives().list_updates(&filter, &input.page).await?;
        let mut views = Vec::with_capacity(page.items.len());
        for u in page.items.clone() {
            views.push(view_of(&mut *tx, u, now).await?);
        }
        Ok((page, views))
    }

    /// Ein Status-Update einer Initiative (UUID).
    pub async fn get_initiative_update(
        &self,
        ctx: &RequestCtx,
        reference: &str,
    ) -> AppResult<InitiativeUpdateView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        require_view(&snap, ctx)?;
        let (u, _) = resolve_update(&mut *tx, &snap, reference).await?;
        view_of(&mut *tx, u, self.clock.now()).await
    }

    /// Veröffentlicht ein Update; das erste nur durch den Owner (ohne Owner jeder mit Schreibrecht).
    pub async fn post_initiative_update(
        &self,
        ctx: &RequestCtx,
        initiative: &str,
        body: &str,
        health: Option<Health>,
        is_diff_hidden: bool,
    ) -> AppResult<InitiativeUpdateView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::ManageInitiatives)?;
        require_enabled(&mut *tx).await?;
        let now = self.clock.now();
        let i = resolve::initiative::resolve_in(&mut *tx, &snap, initiative).await?;
        if i.archived_at.is_some() || i.trashed {
            return Err(AppError::invalid_request("The initiative is archived or deleted.")
                .with_details(json!({ "reason": "initiative_archived" })));
        }
        let body = lineal_domain::rules::markdown::sanitize(body.trim());
        if body.is_empty() {
            return Err(AppError::invalid_request("`body` must not be empty")
                .with_details(json!({ "field": "body" })));
        }
        let actor = resolve::user::actor_user_id(ctx)
            .ok_or_else(|| AppError::forbidden("Status updates need a user."))?;
        let previous = tx.initiatives().latest_update(i.id, None).await?;
        if previous.is_none()
            && let Some(owner) = i.owner_id
            && owner != actor
            && !snap.actor.is_system()
        {
            return Err(AppError::forbidden(
                "Only the initiative owner can publish the first status update.",
            )
            .with_details(json!({ "reason": "first_update_owner_only" })));
        }
        let health = health
            .or(previous.as_ref().map(|p| p.health))
            .unwrap_or(Health::OnTrack);
        let snap_json = snapshot(&mut *tx, &i).await?;
        let (diff, diff_markdown) = match &previous {
            Some(p) => match compute_diff(&p.snapshot, &snap_json, false) {
                Some((d, md)) => (Some(d), Some(md)),
                None => (None, None),
            },
            None => (None, None),
        };
        let id = InitiativeUpdateId(self.ids.new_uuid());
        tx.initiatives()
            .insert_update(&NewInitiativeUpdate {
                id,
                initiative_id: i.id,
                user_id: actor,
                body,
                health,
                snapshot: snap_json,
                diff,
                diff_markdown,
                is_diff_hidden,
                now,
            })
            .await?;
        refresh_parent(&mut *tx, i.id, now).await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::InitiativeUpdateCreated {
                update_id: id,
                initiative_id: i.id,
            },
        )
        .await?;
        let u = tx
            .initiatives()
            .get_update(id)
            .await?
            .ok_or_else(|| AppError::internal("update missing"))?;
        let v = view_of(&mut *tx, u, now).await?;
        tx.commit().await?;
        Ok(v)
    }

    /// Bearbeitet ein Update (nur Autor).
    pub async fn edit_initiative_update(
        &self,
        ctx: &RequestCtx,
        reference: &str,
        body: Option<&str>,
        health: Option<Health>,
    ) -> AppResult<InitiativeUpdateView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::ManageInitiatives)?;
        let now = self.clock.now();
        let (u, _) = resolve_update(&mut *tx, &snap, reference).await?;
        if resolve::user::actor_user_id(ctx) != Some(u.user_id) {
            return Err(AppError::forbidden("Only the author can edit a status update.")
                .with_details(json!({ "reason": "not_author" })));
        }
        if u.archived_at.is_some() {
            return Err(AppError::invalid_request("The status update is archived.")
                .with_details(json!({ "reason": "update_archived" })));
        }
        let clean = body.map(|b| lineal_domain::rules::markdown::sanitize(b.trim()));
        if clean.as_deref().is_some_and(str::is_empty) {
            return Err(AppError::invalid_request("`body` must not be empty")
                .with_details(json!({ "field": "body" })));
        }
        let edited = tx
            .initiatives()
            .edit_update(u.id, clean.as_deref(), health, now)
            .await?;
        if health.is_some() {
            refresh_parent(&mut *tx, u.initiative_id, now).await?;
        }
        emit(
            &mut *tx,
            ctx,
            DomainEvent::InitiativeUpdateUpdated {
                update_id: u.id,
                initiative_id: u.initiative_id,
            },
        )
        .await?;
        let v = view_of(&mut *tx, edited, now).await?;
        tx.commit().await?;
        Ok(v)
    }

    /// Löscht (archiviert) ein Update (Autor oder Admin).
    pub async fn delete_initiative_update(
        &self,
        ctx: &RequestCtx,
        reference: &str,
    ) -> AppResult<(InitiativeUpdateId, DateTime<Utc>)> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::ManageInitiatives)?;
        let now = self.clock.now();
        let (u, _) = resolve_update(&mut *tx, &snap, reference).await?;
        let is_author = resolve::user::actor_user_id(ctx) == Some(u.user_id);
        if !(is_author || snap.actor.is_admin() || snap.actor.is_system()) {
            return Err(
                AppError::forbidden("Only the author or an admin can delete a status update.")
                    .with_details(json!({ "reason": "not_author" })),
            );
        }
        if u.archived_at.is_some() {
            return Err(AppError::not_found("Could not find status update.")
                .with_details(json!({ "reference": reference, "type": "InitiativeUpdate" })));
        }
        tx.initiatives().archive_update(u.id, now).await?;
        refresh_parent(&mut *tx, u.initiative_id, now).await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::InitiativeUpdateRemoved {
                update_id: u.id,
                initiative_id: u.initiative_id,
            },
        )
        .await?;
        tx.commit().await?;
        Ok((u.id, now))
    }
}
