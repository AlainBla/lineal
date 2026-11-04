//! GraphQL: Bots (LIN-22) – User mit Rolle `app` samt API-Keys (`bots`, `botCreate`,
//! `botApiKeyCreate`, `botApiKeyRevoke`). Nur Owner/Admins per Cookie-Session; der Klartext eines
//! Keys erscheint genau einmal in der Antwort von `botApiKeyCreate`.

use super::api_key::{
    ApiKeyCreateInput, ApiKeyGql, parse_create_input, require_session, to_gql, visible_teams,
};
use super::org::gql_err;
use crate::context::{app, request_ctx};
use async_graphql::{Context, InputObject, Object, SimpleObject};
use lineal_app::AppError;
use lineal_app::ports::repos::user::User;
use lineal_app::usecases::bot::{BotView, CreateBotInput};
use lineal_domain::ids::{ApiKeyId, UserId};
use std::collections::HashMap;
use uuid::Uuid;

/// `Bot`: ein User mit Rolle `app`.
#[derive(SimpleObject)]
#[graphql(name = "Bot")]
pub struct BotGql {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub email: String,
    /// `false` = suspendiert (Keys wirken dann nicht).
    pub active: bool,
    pub created_at: String,
    /// Neueste zuerst, inkl. widerrufener.
    pub api_keys: Vec<ApiKeyGql>,
}

#[derive(InputObject)]
pub struct BotCreateInput {
    pub name: String,
    pub email: String,
}

#[derive(SimpleObject)]
pub struct BotCreatePayload {
    pub success: bool,
    pub bot: BotGql,
}

#[derive(SimpleObject)]
pub struct BotApiKeyCreatePayload {
    pub success: bool,
    pub api_key: ApiKeyGql,
    /// Klartext-Token, nur in dieser Antwort.
    pub token: String,
}

#[derive(SimpleObject)]
pub struct BotApiKeyRevokePayload {
    pub success: bool,
}

fn bot_gql(
    user: User,
    keys: Vec<lineal_app::ports::repos::auth::ApiKeyRecord>,
    teams: &HashMap<Uuid, super::api_key::ApiKeyTeamGql>,
) -> BotGql {
    BotGql {
        id: user.id.to_string(),
        name: user.name,
        display_name: user.display_name,
        email: user.email,
        active: user.active,
        created_at: user.created_at.to_rfc3339(),
        api_keys: keys.into_iter().map(|k| to_gql(k, teams)).collect(),
    }
}

fn parse_id(id: &str, kind: &str) -> async_graphql::Result<Uuid> {
    Uuid::parse_str(id).map_err(|_| {
        gql_err(
            AppError::not_found(format!("Could not find referenced {kind}."))
                .with_details(serde_json::json!({ "type": kind })),
        )
    })
}

#[derive(Default)]
pub struct BotQuery;

#[Object]
impl BotQuery {
    /// Alle Bots des Workspace (inkl. suspendierter), nur für Admins.
    async fn bots(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<BotGql>> {
        require_session(ctx)?;
        let bots = app(ctx)?
            .list_bots(request_ctx(ctx)?)
            .await
            .map_err(gql_err)?;
        let teams = visible_teams(ctx).await?;
        Ok(bots
            .into_iter()
            .map(|BotView { user, api_keys }| bot_gql(user, api_keys, &teams))
            .collect())
    }
}

#[derive(Default)]
pub struct BotMutation;

#[Object]
impl BotMutation {
    /// Legt einen Bot an (Rolle `app`, ohne Passwort und ohne E-Mail-Benachrichtigungen).
    async fn bot_create(
        &self,
        ctx: &Context<'_>,
        input: BotCreateInput,
    ) -> async_graphql::Result<BotCreatePayload> {
        require_session(ctx)?;
        let user = app(ctx)?
            .create_bot(
                request_ctx(ctx)?,
                CreateBotInput {
                    name: input.name,
                    email: input.email,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(BotCreatePayload {
            success: true,
            bot: bot_gql(user, Vec::new(), &HashMap::new()),
        })
    }

    /// Legt einen API-Key für den Bot an (Scopes `read`/`write`).
    async fn bot_api_key_create(
        &self,
        ctx: &Context<'_>,
        bot_id: String,
        input: ApiKeyCreateInput,
    ) -> async_graphql::Result<BotApiKeyCreatePayload> {
        require_session(ctx)?;
        let bot = UserId(parse_id(&bot_id, "Bot")?);
        let input = parse_create_input(input)?;
        let created = app(ctx)?
            .create_bot_api_key(request_ctx(ctx)?, bot, input)
            .await
            .map_err(gql_err)?;
        let teams = visible_teams(ctx).await?;
        Ok(BotApiKeyCreatePayload {
            success: true,
            api_key: to_gql(created.record, &teams),
            token: created.key,
        })
    }

    /// Widerruft einen Key des Bots (Keys anderer User → `not_found`).
    async fn bot_api_key_revoke(
        &self,
        ctx: &Context<'_>,
        bot_id: String,
        id: String,
    ) -> async_graphql::Result<BotApiKeyRevokePayload> {
        require_session(ctx)?;
        let bot = UserId(parse_id(&bot_id, "Bot")?);
        let key = ApiKeyId(parse_id(&id, "ApiKey")?);
        app(ctx)?
            .revoke_bot_api_key(request_ctx(ctx)?, bot, key)
            .await
            .map_err(gql_err)?;
        Ok(BotApiKeyRevokePayload { success: true })
    }
}
