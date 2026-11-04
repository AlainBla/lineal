//! Bots (LIN-22): User mit Rolle `app`, die Workspace-Admins anlegen und mit API-Keys ausstatten.
//!
//! Bots haben kein Passwort (kein Browser-Login) und keine E-Mail-Benachrichtigungen; sie handeln
//! ausschließlich per API-Key und erscheinen dann als `Actor::App`. Verwaltung nur durch Owner/Admins
//! per Browser-Session (wie die eigene Key-Verwaltung), Bot-Keys nie mit Scope `admin`.

use super::audit::AuditEvent;
use super::auth::{CreateApiKeyInput, CreatedApiKey};
use super::team::emit;
use super::user::{check_invite_domain, display_name_from_email, validate_email};
use crate::app::App;
use crate::context::{Actor, RequestCtx};
use crate::error::{AppError, AppResult};
use crate::policy::PolicySnapshot;
use crate::ports::Tx;
use crate::ports::repos::auth::ApiKeyRecord;
use crate::ports::repos::notification::UserNotificationPrefsPatch;
use crate::ports::repos::user::{NewUser, PageRequest, User, UserFilter};
use lineal_domain::enums::UserRole;
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{ApiKeyId, UserId};
use serde_json::json;

/// Eingabe für `create_bot`.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateBotInput {
    pub name: String,
    pub email: String,
}

/// Ein Bot mit seinen API-Keys (neueste zuerst, inkl. widerrufener).
#[derive(Debug, Clone, PartialEq)]
pub struct BotView {
    pub user: User,
    pub api_keys: Vec<ApiKeyRecord>,
}

/// Nur Owner/Admins mit `admin`-Scope und per Browser-Session.
fn require_bot_admin(ctx: &RequestCtx) -> AppResult<()> {
    let admin = matches!(
        ctx.actor,
        Actor::User {
            role: UserRole::Owner | UserRole::Admin,
            ..
        }
    );
    if !admin || !ctx.scopes.admin {
        return Err(AppError::forbidden(
            "Only workspace admins can manage bots.",
        ));
    }
    if !ctx.via_session {
        return Err(AppError::forbidden(
            "Bots can only be managed from a browser session.",
        ));
    }
    Ok(())
}

fn bot_not_found() -> AppError {
    AppError::not_found("Could not find referenced bot.").with_details(json!({ "type": "Bot" }))
}

/// Lädt den Bot (User mit Rolle `app`); andere User gelten als nicht gefunden.
async fn load_bot(tx: &mut dyn Tx, id: UserId) -> AppResult<User> {
    match tx.users().get(id).await? {
        Some(u) if u.role == UserRole::App => Ok(u),
        _ => Err(bot_not_found()),
    }
}

impl App {
    /// Alle Bots des Workspace (inkl. suspendierter) mit ihren Keys.
    pub async fn list_bots(&self, ctx: &RequestCtx) -> AppResult<Vec<BotView>> {
        require_bot_admin(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let users = tx
            .users()
            .list(
                &UserFilter {
                    include_suspended: true,
                    role: Some(UserRole::App),
                    ..Default::default()
                },
                &PageRequest {
                    limit: 250,
                    order_by: crate::ports::repos::user::OrderBy::CreatedAt,
                    ..Default::default()
                },
            )
            .await?;
        let mut out = Vec::new();
        for user in users.items {
            let api_keys = tx.auth().list_api_keys(user.id).await?;
            out.push(BotView { user, api_keys });
        }
        Ok(out)
    }

    /// Legt einen Bot an: Rolle `app`, kein Passwort, E-Mail-Benachrichtigungen aus.
    pub async fn create_bot(&self, ctx: &RequestCtx, input: CreateBotInput) -> AppResult<User> {
        require_bot_admin(ctx)?;
        if !ctx.scopes.write {
            return Err(AppError::forbidden(
                "Token scope does not allow write operations.",
            ));
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let name = lineal_domain::rules::text::validate_single_line_name("name", &input.name, 255)?;
        let email = input.email.trim().to_string();
        validate_email(&email)?;
        check_invite_domain(&snap.org, &email, "email")?;
        if tx.users().find_by_email(&email).await?.is_some() {
            return Err(AppError::conflict("A user with this email already exists")
                .with_details(json!({ "field": "email" })));
        }
        let now = self.clock.now();
        let id = UserId(self.ids.new_uuid());
        let display_name = display_name_from_email(&mut *tx, &email).await?;
        tx.users()
            .insert(&NewUser {
                id,
                name,
                display_name,
                email,
                role: UserRole::App,
                password_hash: None,
                now,
            })
            .await?;
        // Bots lesen keine Mails: keine Digests an die (oft fiktive) Adresse.
        tx.notifications()
            .update_prefs(
                id,
                &UserNotificationPrefsPatch {
                    settings: Some(json!({ "emailEnabled": false })),
                    ..Default::default()
                },
                now,
            )
            .await?;
        emit(&mut *tx, ctx, DomainEvent::UserCreated { user_id: id }).await?;
        let user = tx
            .users()
            .get(id)
            .await?
            .ok_or_else(|| AppError::internal("user vanished after insert"))?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new(
                "bot.created",
                "user",
                Some(id.0),
                json!({ "name": user.name, "display_name": user.display_name }),
            ),
        )
        .await?;
        tx.commit().await?;
        Ok(user)
    }

    /// Legt einen API-Key für einen Bot an (Scopes nur `read`/`write`).
    pub async fn create_bot_api_key(
        &self,
        ctx: &RequestCtx,
        bot: UserId,
        input: CreateApiKeyInput,
    ) -> AppResult<CreatedApiKey> {
        require_bot_admin(ctx)?;
        if !ctx.scopes.write {
            return Err(AppError::forbidden(
                "Token scope does not allow write operations.",
            ));
        }
        if input.scopes.iter().any(|s| s == "admin") {
            return Err(
                AppError::invalid_request("bot keys cannot have the admin scope")
                    .with_details(json!({ "field": "scopes" })),
            );
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let bot = load_bot(&mut *tx, bot).await?;
        if !bot.active {
            return Err(AppError::invalid_request("the bot is suspended")
                .with_details(json!({ "field": "botId" })));
        }
        let created = self
            .mint_api_key(&mut *tx, &snap, ctx, bot.id, input, false)
            .await?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new(
                "bot.api_key_created",
                "user",
                Some(bot.id.0),
                json!({ "key_prefix": created.record.key_prefix, "scopes": created.record.scopes }),
            ),
        )
        .await?;
        tx.commit().await?;
        Ok(created)
    }

    /// Widerruft einen Key des Bots (Keys anderer User → `not_found`).
    pub async fn revoke_bot_api_key(
        &self,
        ctx: &RequestCtx,
        bot: UserId,
        key: ApiKeyId,
    ) -> AppResult<()> {
        require_bot_admin(ctx)?;
        {
            let mut tx = self.store.begin(ctx.org_id).await?;
            let bot = load_bot(&mut *tx, bot).await?;
            let owned = tx
                .auth()
                .get_api_key(key)
                .await?
                .is_some_and(|k| k.user_id == bot.id);
            if !owned {
                return Err(AppError::not_found("Could not find referenced ApiKey.")
                    .with_details(json!({ "type": "ApiKey" })));
            }
        }
        // Widerruf samt Audit `api_key.revoked` (Admins dürfen fremde Keys widerrufen).
        self.revoke_api_key(ctx, key).await
    }
}
