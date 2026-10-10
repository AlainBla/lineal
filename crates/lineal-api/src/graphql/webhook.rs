//! GraphQL-Teilobjekte des Pakets WP16: Webhooks, Einladungen, Team-Lebenszyklus und
//! `createWorkspace` (Konto-Session).
//!
//! Hinweis: Einladungen, Team-Lebenszyklus und `createWorkspace` haben keine eigene Datei in der
//! Eigentumsliste und stehen deshalb hier (der Root `graphql/mod.rs` bindet nur diese Datei ein).
//! Das Webhook-Secret erscheint nur in `webhookCreate`/`webhookRotateSecret`; Einladungs-Tokens
//! werden nie ausgegeben (nur per E-Mail).

use super::org::{OrganizationGql, gql_err};
use super::team::TeamGql;
use crate::context::{app, request_ctx};
use async_graphql::{Context, InputObject, Object, SimpleObject};
use lineal_app::AppError;
use lineal_app::ports::repos::webhook::{DeliveryRecord, InvitationRecord, WebhookRecord};
use lineal_app::usecases::invitation::InviteUsersInput;
use lineal_app::usecases::team_lifecycle::{OpenIssuesOnRetirement, SubTeamHandling};
use lineal_app::usecases::webhook::{CreateWebhookInput, UpdateWebhookInput};
use lineal_domain::enums::UserRole;
use lineal_domain::ids::WebhookId;
use uuid::Uuid;

fn not_found(kind: &str) -> async_graphql::Error {
    gql_err(
        AppError::not_found(format!("Could not find referenced {kind}."))
            .with_details(serde_json::json!({ "type": kind })),
    )
}

fn webhook_id(s: &str) -> async_graphql::Result<WebhookId> {
    Uuid::parse_str(s)
        .map(WebhookId)
        .map_err(|_| not_found("Webhook"))
}

fn uuid_of(s: &str, kind: &str) -> async_graphql::Result<Uuid> {
    Uuid::parse_str(s).map_err(|_| not_found(kind))
}

/// `Webhook` (ohne Secret).
#[derive(SimpleObject)]
#[graphql(name = "Webhook")]
pub struct WebhookGql {
    pub id: String,
    pub label: Option<String>,
    pub url: String,
    pub enabled: bool,
    pub team_id: Option<String>,
    pub all_public_teams: bool,
    pub resource_types: Vec<String>,
    pub creator_id: String,
    pub consecutive_failures: i32,
    pub disabled_reason: Option<String>,
    pub last_success_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<WebhookRecord> for WebhookGql {
    fn from(w: WebhookRecord) -> Self {
        Self {
            id: w.id.to_string(),
            label: w.label,
            url: w.url,
            enabled: w.enabled,
            team_id: w.team_id.map(|t| t.to_string()),
            all_public_teams: w.all_public_teams,
            resource_types: w.resource_types,
            creator_id: w.creator_id.to_string(),
            consecutive_failures: w.consecutive_failures,
            disabled_reason: w.disabled_reason,
            last_success_at: w.last_success_at.map(|d| d.to_rfc3339()),
            created_at: w.created_at.to_rfc3339(),
            updated_at: w.updated_at.to_rfc3339(),
        }
    }
}

/// `WebhookDelivery` (Zustellprotokoll, ohne Payload).
#[derive(SimpleObject)]
#[graphql(name = "WebhookDelivery")]
pub struct WebhookDeliveryGql {
    pub id: String,
    pub webhook_id: String,
    pub event_id: String,
    pub resource_type: String,
    pub action: String,
    pub attempt: i32,
    pub status: String,
    pub response_code: Option<i32>,
    pub duration_ms: Option<i32>,
    pub next_attempt_at: Option<String>,
    pub created_at: String,
}

impl From<DeliveryRecord> for WebhookDeliveryGql {
    fn from(d: DeliveryRecord) -> Self {
        Self {
            id: d.id.to_string(),
            webhook_id: d.webhook_id.to_string(),
            event_id: d.event_id.to_string(),
            resource_type: d.resource_type,
            action: d.action,
            attempt: d.attempt,
            status: d.status,
            response_code: d.response_code,
            duration_ms: d.duration_ms,
            next_attempt_at: d.next_attempt_at.map(|t| t.to_rfc3339()),
            created_at: d.created_at.to_rfc3339(),
        }
    }
}

#[derive(InputObject)]
pub struct WebhookCreateInput {
    pub label: Option<String>,
    pub url: String,
    /// Team-Referenz (UUID, Key oder Name); genau eines von `teamId`/`allPublicTeams`.
    pub team_id: Option<String>,
    #[graphql(default)]
    pub all_public_teams: bool,
    pub resource_types: Vec<String>,
}

#[derive(InputObject)]
pub struct WebhookUpdateInput {
    pub label: Option<String>,
    pub url: Option<String>,
    pub enabled: Option<bool>,
    pub resource_types: Option<Vec<String>>,
}

#[derive(SimpleObject)]
pub struct WebhookPayload {
    pub success: bool,
    pub webhook: Option<WebhookGql>,
}

/// Antwort mit dem Klartext-Secret (nur bei Anlage und Rotation, genau einmal).
#[derive(SimpleObject)]
pub struct WebhookSecretPayload {
    pub success: bool,
    pub webhook: Option<WebhookGql>,
    pub secret: Option<String>,
}

#[derive(SimpleObject)]
#[graphql(name = "WebhookSuccessPayload")]
pub struct SuccessPayload {
    pub success: bool,
}

/// `Invitation`.
#[derive(SimpleObject)]
#[graphql(name = "Invitation")]
pub struct InvitationGql {
    pub id: String,
    pub email: String,
    pub role: String,
    pub team_ids: Vec<String>,
    pub inviter_id: String,
    pub status: String,
    pub expires_at: String,
    pub last_sent_at: String,
    pub accepted_at: Option<String>,
    pub created_at: String,
}

impl From<InvitationRecord> for InvitationGql {
    fn from(i: InvitationRecord) -> Self {
        Self {
            id: i.id.to_string(),
            email: i.email,
            role: i.role.as_str().into(),
            team_ids: i.team_ids.iter().map(ToString::to_string).collect(),
            inviter_id: i.inviter_id.to_string(),
            status: i.status,
            expires_at: i.expires_at.to_rfc3339(),
            last_sent_at: i.last_sent_at.to_rfc3339(),
            accepted_at: i.accepted_at.map(|d| d.to_rfc3339()),
            created_at: i.created_at.to_rfc3339(),
        }
    }
}

#[derive(InputObject)]
pub struct InviteUsersGqlInput {
    pub emails: Vec<String>,
    /// `admin`, `user` (Default) oder `guest`.
    pub role: Option<String>,
    /// Team-Referenzen; Gäste brauchen mindestens eines.
    pub team_ids: Option<Vec<String>>,
}

#[derive(SimpleObject)]
pub struct InvitationBatchPayload {
    pub success: bool,
    pub invitations: Vec<InvitationGql>,
}

#[derive(SimpleObject)]
pub struct InvitationPayload {
    pub success: bool,
    pub invitation: Option<InvitationGql>,
}

#[derive(SimpleObject)]
pub struct AcceptInvitationPayload {
    pub success: bool,
    /// `urlKey` des Workspace, dem der User beigetreten ist.
    pub url_key: String,
}

#[derive(InputObject)]
pub struct CreateWorkspaceInput {
    pub name: String,
    pub url_key: Option<String>,
}

#[derive(SimpleObject)]
pub struct CreateWorkspacePayload {
    pub success: bool,
    pub workspace: Option<OrganizationGql>,
}

#[derive(SimpleObject)]
pub struct TeamLifecyclePayload {
    pub success: bool,
    pub team: Option<TeamGql>,
}

fn parse_role(s: Option<&str>) -> async_graphql::Result<UserRole> {
    match s.unwrap_or("user") {
        "admin" => Ok(UserRole::Admin),
        "user" => Ok(UserRole::User),
        "guest" => Ok(UserRole::Guest),
        other => Err(gql_err(AppError::invalid_request(format!(
            "role must be admin, user or guest, not `{other}`"
        )))),
    }
}

fn sub_team_handling(s: Option<&str>) -> async_graphql::Result<Option<SubTeamHandling>> {
    s.map(|v| SubTeamHandling::parse(v).map_err(gql_err))
        .transpose()
}

#[derive(Default)]
pub struct WebhookQuery;

#[Object]
impl WebhookQuery {
    /// Die für den Aufrufer verwaltbaren Webhooks (Admins; Team-Owner für ihre Teams).
    async fn webhooks(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<WebhookGql>> {
        let list = app(ctx)?
            .list_webhooks(request_ctx(ctx)?)
            .await
            .map_err(gql_err)?;
        Ok(list.into_iter().map(Into::into).collect())
    }

    async fn webhook(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<WebhookGql> {
        let w = app(ctx)?
            .get_webhook(request_ctx(ctx)?, webhook_id(&id)?)
            .await
            .map_err(gql_err)?;
        Ok(w.into())
    }

    /// Die letzten 100 Zustellungen eines Webhooks.
    async fn webhook_deliveries(
        &self,
        ctx: &Context<'_>,
        webhook_id_: String,
    ) -> async_graphql::Result<Vec<WebhookDeliveryGql>> {
        let list = app(ctx)?
            .list_webhook_deliveries(request_ctx(ctx)?, webhook_id(&webhook_id_)?)
            .await
            .map_err(gql_err)?;
        Ok(list.into_iter().map(Into::into).collect())
    }

    /// Einladungen des Workspace (Recht `invitationsRole`).
    async fn invitations(
        &self,
        ctx: &Context<'_>,
        #[graphql(default)] pending_only: bool,
    ) -> async_graphql::Result<Vec<InvitationGql>> {
        let list = app(ctx)?
            .list_invitations(request_ctx(ctx)?, pending_only)
            .await
            .map_err(gql_err)?;
        Ok(list.into_iter().map(Into::into).collect())
    }
}

#[derive(Default)]
pub struct WebhookMutation;

#[Object]
impl WebhookMutation {
    async fn webhook_create(
        &self,
        ctx: &Context<'_>,
        input: WebhookCreateInput,
    ) -> async_graphql::Result<WebhookSecretPayload> {
        let v = app(ctx)?
            .create_webhook(
                request_ctx(ctx)?,
                CreateWebhookInput {
                    label: input.label,
                    url: input.url,
                    team: input.team_id,
                    all_public_teams: input.all_public_teams,
                    resource_types: input.resource_types,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(WebhookSecretPayload {
            success: true,
            webhook: Some(v.webhook.into()),
            secret: v.secret,
        })
    }

    async fn webhook_update(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: WebhookUpdateInput,
    ) -> async_graphql::Result<WebhookPayload> {
        let w = app(ctx)?
            .update_webhook(
                request_ctx(ctx)?,
                webhook_id(&id)?,
                UpdateWebhookInput {
                    label: input.label.map(Some),
                    url: input.url,
                    enabled: input.enabled,
                    resource_types: input.resource_types,
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(WebhookPayload {
            success: true,
            webhook: Some(w.into()),
        })
    }

    async fn webhook_delete(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<SuccessPayload> {
        app(ctx)?
            .delete_webhook(request_ctx(ctx)?, webhook_id(&id)?)
            .await
            .map_err(gql_err)?;
        Ok(SuccessPayload { success: true })
    }

    /// Neues Signing-Secret; das alte gilt 24 h zusätzlich (`Lineal-Signature-Previous`).
    async fn webhook_rotate_secret(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<WebhookSecretPayload> {
        let v = app(ctx)?
            .rotate_webhook_secret(request_ctx(ctx)?, webhook_id(&id)?)
            .await
            .map_err(gql_err)?;
        Ok(WebhookSecretPayload {
            success: true,
            webhook: Some(v.webhook.into()),
            secret: v.secret,
        })
    }

    /// „Erneut senden“ einer Zustellung.
    async fn webhook_delivery_retry(
        &self,
        ctx: &Context<'_>,
        webhook_id_: String,
        delivery_id: String,
    ) -> async_graphql::Result<SuccessPayload> {
        app(ctx)?
            .resend_webhook_delivery(
                request_ctx(ctx)?,
                webhook_id(&webhook_id_)?,
                uuid_of(&delivery_id, "WebhookDelivery")?,
            )
            .await
            .map_err(gql_err)?;
        Ok(SuccessPayload { success: true })
    }

    // --- Einladungen ----------------------------------------------------------------------

    async fn invite_users(
        &self,
        ctx: &Context<'_>,
        input: InviteUsersGqlInput,
    ) -> async_graphql::Result<InvitationBatchPayload> {
        let created = app(ctx)?
            .invite_users(
                request_ctx(ctx)?,
                InviteUsersInput {
                    emails: input.emails,
                    role: Some(parse_role(input.role.as_deref())?),
                    teams: input.team_ids.unwrap_or_default(),
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(InvitationBatchPayload {
            success: true,
            invitations: created.into_iter().map(|c| c.invitation.into()).collect(),
        })
    }

    async fn invitation_resend(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<InvitationPayload> {
        let c = app(ctx)?
            .resend_invitation(request_ctx(ctx)?, uuid_of(&id, "Invitation")?)
            .await
            .map_err(gql_err)?;
        Ok(InvitationPayload {
            success: true,
            invitation: Some(c.invitation.into()),
        })
    }

    async fn invitation_revoke(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<InvitationPayload> {
        let i = app(ctx)?
            .revoke_invitation(request_ctx(ctx)?, uuid_of(&id, "Invitation")?)
            .await
            .map_err(gql_err)?;
        Ok(InvitationPayload {
            success: true,
            invitation: Some(i.into()),
        })
    }

    /// Nimmt eine Einladung mit der Konto-Session an: die E-Mail des angemeldeten Users muss zur
    /// Einladung passen (sonst `not_found`). Ungültiges, abgelaufenes oder fremdes Token ebenso.
    async fn accept_invitation(
        &self,
        ctx: &Context<'_>,
        token: String,
    ) -> async_graphql::Result<AcceptInvitationPayload> {
        let ok = app(ctx)?
            .accept_invitation_as_account(request_ctx(ctx)?, &token)
            .await
            .map_err(gql_err)?;
        Ok(AcceptInvitationPayload {
            success: true,
            url_key: ok.url_key,
        })
    }

    /// Legt mit der Konto-Session einen weiteren Workspace an (Owner = der Aufrufer).
    async fn create_workspace(
        &self,
        ctx: &Context<'_>,
        input: CreateWorkspaceInput,
    ) -> async_graphql::Result<CreateWorkspacePayload> {
        let created = app(ctx)?
            .create_workspace_for_account(request_ctx(ctx)?, &input.name, input.url_key.as_deref())
            .await
            .map_err(gql_err)?;
        Ok(CreateWorkspacePayload {
            success: true,
            workspace: Some(created.org.into()),
        })
    }

    // --- Team-Lebenszyklus (docs/02 §2.7) -------------------------------------------------

    /// Legt das Team still. `handleSubTeamsOnRetirement`: `retire` oder `unnest`.
    async fn team_retire(
        &self,
        ctx: &Context<'_>,
        id: String,
        handle_sub_teams_on_retirement: Option<String>,
    ) -> async_graphql::Result<TeamLifecyclePayload> {
        let t = app(ctx)?
            .retire_team(
                request_ctx(ctx)?,
                &id,
                sub_team_handling(handle_sub_teams_on_retirement.as_deref())?,
                OpenIssuesOnRetirement::Keep,
            )
            .await
            .map_err(gql_err)?;
        Ok(TeamLifecyclePayload {
            success: true,
            team: Some(t.into()),
        })
    }

    async fn team_unretire(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<TeamLifecyclePayload> {
        let t = app(ctx)?
            .restore_team(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?;
        Ok(TeamLifecyclePayload {
            success: true,
            team: Some(t.into()),
        })
    }

    /// Löscht das Team (30 Tage Papierkorb). `handleSubTeams`: `delete` oder `unnest`.
    async fn team_delete(
        &self,
        ctx: &Context<'_>,
        id: String,
        handle_sub_teams: Option<String>,
    ) -> async_graphql::Result<TeamLifecyclePayload> {
        let t = app(ctx)?
            .delete_team(
                request_ctx(ctx)?,
                &id,
                sub_team_handling(handle_sub_teams.as_deref())?,
            )
            .await
            .map_err(gql_err)?;
        Ok(TeamLifecyclePayload {
            success: true,
            team: Some(t.into()),
        })
    }

    /// Stellt ein gelöschtes Team wieder her (Admin/Owner, innerhalb der Frist).
    async fn team_unarchive(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<TeamLifecyclePayload> {
        let t = app(ctx)?
            .restore_deleted_team(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?;
        Ok(TeamLifecyclePayload {
            success: true,
            team: Some(t.into()),
        })
    }
}
