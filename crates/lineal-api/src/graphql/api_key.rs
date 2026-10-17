//! GraphQL: persönliche API-Keys (`apiKeys`, `apiKeyCreate`, `apiKeyRevoke`).
//!
//! Nie ausgegeben: Hash. Der Klartext erscheint genau einmal in der Antwort von `apiKeyCreate`.
//! Anlegen geht nur per Cookie-Session (kein Key aus einem Key); Widerrufen nur eigene Keys.

use super::org::gql_err;
use crate::context::{app, request_ctx};
use crate::middleware::auth::CredentialClass;
use async_graphql::{Context, Enum, InputObject, Object, SimpleObject};
use chrono::{DateTime, Utc};
use lineal_app::AppError;
use lineal_app::ports::repos::auth::ApiKeyRecord;
use lineal_app::ports::repos::user::PageRequest;
use lineal_app::usecases::auth::CreateApiKeyInput;
use lineal_app::usecases::team::ListTeamsInput;
use lineal_domain::ids::ApiKeyId;
use std::collections::HashMap;
use uuid::Uuid;

/// Höchstzahl Teams je Key (Eingabegrenze).
const MAX_KEY_TEAMS: usize = 50;

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
#[graphql(name = "ApiKeyScope")]
pub enum ApiKeyScopeGql {
    Read,
    Write,
}

/// Team-Beschränkung eines Keys (nur sichtbare Teams).
#[derive(SimpleObject, Clone)]
#[graphql(name = "ApiKeyTeam")]
pub struct ApiKeyTeamGql {
    pub id: String,
    pub key: String,
    pub name: String,
}

/// `ApiKey` (ohne Hash und Klartext).
#[derive(SimpleObject)]
#[graphql(name = "ApiKey")]
pub struct ApiKeyGql {
    pub id: String,
    pub label: String,
    pub key_prefix: String,
    pub scopes: Vec<String>,
    /// `null` = alle Teams des Users.
    pub teams: Option<Vec<ApiKeyTeamGql>>,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
}

#[derive(InputObject)]
pub struct ApiKeyCreateInput {
    pub label: String,
    pub scopes: Vec<ApiKeyScopeGql>,
    /// Team-Referenzen (UUID/Key/Name) – nur sichtbare Teams.
    pub team_ids: Option<Vec<String>>,
    /// ISO-8601-Zeitpunkt in der Zukunft.
    pub expires_at: Option<String>,
}

#[derive(SimpleObject)]
pub struct ApiKeyCreatePayload {
    pub success: bool,
    pub api_key: ApiKeyGql,
    /// Klartext-Token, nur in dieser Antwort.
    pub token: String,
}

#[derive(SimpleObject)]
pub struct ApiKeyRevokePayload {
    pub success: bool,
}

fn to_gql(rec: ApiKeyRecord, teams: &HashMap<Uuid, ApiKeyTeamGql>) -> ApiKeyGql {
    ApiKeyGql {
        id: rec.id.to_string(),
        label: rec.label,
        key_prefix: rec.key_prefix,
        scopes: rec.scopes,
        teams: rec.team_ids.map(|ids| {
            ids.iter()
                .filter_map(|t| teams.get(&t.0).cloned())
                .collect()
        }),
        created_at: rec.created_at.to_rfc3339(),
        last_used_at: rec.last_used_at.map(|d| d.to_rfc3339()),
        expires_at: rec.expires_at.map(|d| d.to_rfc3339()),
        revoked_at: rec.revoked_at.map(|d| d.to_rfc3339()),
    }
}

/// Für den Viewer sichtbare Teams (inkl. stillgelegter) nach ID.
async fn visible_teams(ctx: &Context<'_>) -> async_graphql::Result<HashMap<Uuid, ApiKeyTeamGql>> {
    let page = app(ctx)?
        .list_teams(
            request_ctx(ctx)?,
            ListTeamsInput {
                include_archived: true,
                page: PageRequest {
                    limit: 250,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .map_err(gql_err)?;
    Ok(page
        .items
        .into_iter()
        .map(|v| {
            (
                v.team.id.0,
                ApiKeyTeamGql {
                    id: v.team.id.to_string(),
                    key: v.team.key,
                    name: v.team.name,
                },
            )
        })
        .collect())
}

fn not_found() -> async_graphql::Error {
    gql_err(
        AppError::not_found("Could not find referenced ApiKey.")
            .with_details(serde_json::json!({ "type": "ApiKey" })),
    )
}

/// Key-Verwaltung (Anlegen, Auflisten, Widerrufen) nur per Cookie-Session, nie mit API-Key/OAuth.
fn require_session(ctx: &Context<'_>) -> async_graphql::Result<()> {
    if ctx.data_opt::<CredentialClass>() != Some(&CredentialClass::Session) {
        return Err(gql_err(AppError::forbidden(
            "API keys can only be managed from a browser session.",
        )));
    }
    Ok(())
}

#[derive(Default)]
pub struct ApiKeyQuery;

#[Object]
impl ApiKeyQuery {
    /// Eigene API-Keys des Viewers (neueste zuerst, inkl. widerrufener).
    async fn api_keys(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<ApiKeyGql>> {
        require_session(ctx)?;
        let keys = app(ctx)?
            .list_api_keys(request_ctx(ctx)?)
            .await
            .map_err(gql_err)?;
        let teams = visible_teams(ctx).await?;
        Ok(keys.into_iter().map(|k| to_gql(k, &teams)).collect())
    }
}

#[derive(Default)]
pub struct ApiKeyMutation;

#[Object]
impl ApiKeyMutation {
    /// Legt einen Key an. Nur per Cookie-Session (nie mit einem API-Key/OAuth-Token).
    async fn api_key_create(
        &self,
        ctx: &Context<'_>,
        input: ApiKeyCreateInput,
    ) -> async_graphql::Result<ApiKeyCreatePayload> {
        require_session(ctx)?;
        if input
            .team_ids
            .as_ref()
            .is_some_and(|t| t.len() > MAX_KEY_TEAMS)
        {
            return Err(gql_err(
                AppError::invalid_request("too many teams")
                    .with_details(serde_json::json!({ "field": "teamIds" })),
            ));
        }
        let expires_at = match &input.expires_at {
            Some(s) => Some(
                DateTime::parse_from_rfc3339(s)
                    .map_err(|_| {
                        gql_err(
                            AppError::invalid_request("expiresAt must be an ISO-8601 timestamp")
                                .with_details(serde_json::json!({ "field": "expiresAt" })),
                        )
                    })?
                    .with_timezone(&Utc),
            ),
            None => None,
        };
        let scopes = input
            .scopes
            .iter()
            .map(|s| match s {
                ApiKeyScopeGql::Read => "read".to_owned(),
                ApiKeyScopeGql::Write => "write".to_owned(),
            })
            .collect();
        let created = app(ctx)?
            .create_api_key(
                request_ctx(ctx)?,
                CreateApiKeyInput {
                    label: input.label,
                    scopes,
                    teams: input.team_ids,
                    expires_at,
                },
            )
            .await
            .map_err(gql_err)?;
        let teams = visible_teams(ctx).await?;
        Ok(ApiKeyCreatePayload {
            success: true,
            api_key: to_gql(created.record, &teams),
            token: created.key,
        })
    }

    /// Widerruft einen eigenen Key (fremde IDs → `not_found`, auch für Admins).
    async fn api_key_revoke(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<ApiKeyRevokePayload> {
        require_session(ctx)?;
        let id = Uuid::parse_str(&id)
            .map(ApiKeyId)
            .map_err(|_| not_found())?;
        let rctx = request_ctx(ctx)?;
        let a = app(ctx)?;
        let own = a.list_api_keys(rctx).await.map_err(gql_err)?;
        if !own.iter().any(|k| k.id == id) {
            return Err(not_found());
        }
        a.revoke_api_key(rctx, id).await.map_err(gql_err)?;
        Ok(ApiKeyRevokePayload { success: true })
    }
}
