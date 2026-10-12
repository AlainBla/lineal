//! GraphQL-Teilobjekte des Bereichs `org` (WP02): Workspace lesen und Settings ändern.

//! `createWorkspace` braucht eine Konto-Session ohne Workspace-Kontext und folgt mit WP03/WP16.

use crate::context::{app, request_ctx};
use async_graphql::{Context, ErrorExtensions, InputObject, Object, SimpleObject};
use lineal_app::AppError;
use lineal_app::policy::OrgPolicy;
use lineal_app::ports::repos::org::Org;
use lineal_app::ports::repos::user::PageRequest;
use lineal_app::usecases::org::{SecuritySettingsPatch, UpdateWorkspaceInput};
use lineal_app::usecases::user::{ListUsersInput, UserView};
use lineal_domain::enums::UserRole;

/// Fehler nach docs/09 §1.2: `extensions.code` = `error` aus docs/00 §7.
pub(crate) fn gql_err(e: AppError) -> async_graphql::Error {
    async_graphql::Error::new(e.message.clone()).extend_with(|_, ext| {
        ext.set("code", e.kind.code());
        ext.set("status", e.kind.http_status());
        ext.set("details", e.details.to_string());
    })
}

/// `Organization`.
#[derive(SimpleObject)]
#[graphql(name = "Organization")]
pub struct OrganizationGql {
    pub id: String,
    pub name: String,
    pub url_key: String,
    pub previous_url_keys: Vec<String>,
    pub logo_url: Option<String>,
    pub fiscal_year_start_month: i32,
    pub initiatives_enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<Org> for OrganizationGql {
    fn from(o: Org) -> Self {
        Self {
            id: o.id.to_string(),
            name: o.name,
            url_key: o.url_key,
            previous_url_keys: o.previous_url_keys,
            logo_url: o.logo_url,
            fiscal_year_start_month: o.fiscal_year_start_month,
            initiatives_enabled: o.initiatives_enabled,
            created_at: o.created_at.to_rfc3339(),
            updated_at: o.updated_at.to_rfc3339(),
        }
    }
}

#[derive(InputObject)]
pub struct WorkspaceUpdateInput {
    pub name: Option<String>,
    pub url_key: Option<String>,
    pub fiscal_year_start_month: Option<i32>,
    pub initiatives_enabled: Option<bool>,
}

#[derive(SimpleObject)]
pub struct WorkspacePayload {
    pub success: bool,
    pub workspace: Option<OrganizationGql>,
}

/// `WorkspaceSecuritySettings`: Mindestrollen (`user`, `admin`, `owner`) und erlaubte Domains.
#[derive(SimpleObject)]
#[graphql(name = "WorkspaceSecuritySettings")]
pub struct SecuritySettingsGql {
    pub team_creation_role: String,
    pub invitations_role: String,
    pub admin_management_role: String,
    pub label_management_role: String,
    pub template_management_role: String,
    pub personal_api_keys_role: String,
    pub api_settings_role: String,
    pub workspace_initiatives_role: String,
    pub pinned_view_management_role: String,
    pub import_role: String,
    /// Leer = Einladungen an alle Domains erlaubt.
    pub allowed_invite_domains: Vec<String>,
    /// Aufbewahrung des Audit-Logs in Tagen (Default 365, Minimum 30).
    pub audit_log_retention_days: i32,
}

impl From<OrgPolicy> for SecuritySettingsGql {
    fn from(p: OrgPolicy) -> Self {
        let r = |x: UserRole| x.as_str().to_owned();
        Self {
            team_creation_role: r(p.team_creation_role),
            invitations_role: r(p.invitations_role),
            admin_management_role: r(p.admin_management_role),
            label_management_role: r(p.label_management_role),
            template_management_role: r(p.template_management_role),
            personal_api_keys_role: r(p.personal_api_keys_role),
            api_settings_role: r(p.api_settings_role),
            workspace_initiatives_role: r(p.workspace_initiatives_role),
            pinned_view_management_role: r(p.pinned_view_management_role),
            import_role: r(p.import_role),
            allowed_invite_domains: p.allowed_invite_domains,
            audit_log_retention_days: i32::try_from(p.audit_log_retention_days).unwrap_or(365),
        }
    }
}

/// Nicht gesetzte Felder bleiben unverändert; Rollen als `user`, `admin` oder `owner`.
#[derive(InputObject)]
pub struct SecuritySettingsUpdateInput {
    pub team_creation_role: Option<String>,
    pub invitations_role: Option<String>,
    pub admin_management_role: Option<String>,
    pub label_management_role: Option<String>,
    pub template_management_role: Option<String>,
    pub personal_api_keys_role: Option<String>,
    pub api_settings_role: Option<String>,
    pub workspace_initiatives_role: Option<String>,
    pub pinned_view_management_role: Option<String>,
    pub import_role: Option<String>,
    pub allowed_invite_domains: Option<Vec<String>>,
    /// Aufbewahrung des Audit-Logs in Tagen (30 bis 3650).
    pub audit_log_retention_days: Option<i32>,
}

impl SecuritySettingsUpdateInput {
    fn into_patch(self) -> Result<SecuritySettingsPatch, AppError> {
        let mut roles = Vec::new();
        for (key, v) in [
            ("teamCreationRole", self.team_creation_role),
            ("invitationsRole", self.invitations_role),
            ("adminManagementRole", self.admin_management_role),
            ("labelManagementRole", self.label_management_role),
            ("templateManagementRole", self.template_management_role),
            ("personalApiKeysRole", self.personal_api_keys_role),
            ("apiSettingsRole", self.api_settings_role),
            ("workspaceInitiativesRole", self.workspace_initiatives_role),
            ("pinnedViewManagementRole", self.pinned_view_management_role),
            ("importRole", self.import_role),
        ] {
            if let Some(v) = v {
                let role: UserRole = v.parse().map_err(|_| {
                    AppError::invalid_request(format!("`{key}` must be user, admin or owner"))
                        .with_details(serde_json::json!({ "field": key }))
                })?;
                roles.push((key.to_owned(), role));
            }
        }
        Ok(SecuritySettingsPatch {
            roles,
            allowed_invite_domains: self.allowed_invite_domains,
            audit_log_retention_days: self.audit_log_retention_days.map(i64::from),
        })
    }
}

#[derive(SimpleObject)]
pub struct SecuritySettingsPayload {
    pub success: bool,
    pub security_settings: SecuritySettingsGql,
}

/// Team-Angabe einer Mitgliederzeile.
#[derive(SimpleObject)]
#[graphql(name = "WorkspaceMemberTeam")]
pub struct MemberTeamGql {
    pub id: String,
    pub key: String,
    pub name: String,
}

/// `WorkspaceMember`: Admin-Sicht mit E-Mail (nur Owner/Admin).
#[derive(SimpleObject)]
#[graphql(name = "WorkspaceMember")]
pub struct WorkspaceMemberGql {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub email: String,
    pub role: String,
    /// `active`, `suspended` oder `imported`.
    pub status: String,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub teams: Vec<MemberTeamGql>,
}

impl From<UserView> for WorkspaceMemberGql {
    fn from(v: UserView) -> Self {
        let u = v.user;
        let status = if !u.active {
            "suspended".to_owned()
        } else {
            u.status
        };
        Self {
            id: u.id.to_string(),
            name: u.name,
            display_name: u.display_name,
            email: u.email,
            role: u.role.as_str().to_owned(),
            status,
            last_seen_at: u.last_seen.map(|t| t.to_rfc3339()),
            created_at: u.created_at.to_rfc3339(),
            teams: v
                .teams
                .into_iter()
                .map(|t| MemberTeamGql {
                    id: t.team.id.to_string(),
                    key: t.team.key,
                    name: t.display_name,
                })
                .collect(),
        }
    }
}

#[derive(SimpleObject)]
pub struct WorkspaceMemberPayload {
    pub success: bool,
    pub member: WorkspaceMemberGql,
}

#[derive(Default)]
pub struct OrgQuery;

#[Object]
impl OrgQuery {
    /// Der Workspace des Aufrufers.
    async fn workspace(&self, ctx: &Context<'_>) -> async_graphql::Result<OrganizationGql> {
        let org = app(ctx)?
            .get_workspace(request_ctx(ctx)?)
            .await
            .map_err(gql_err)?;
        Ok(org.into())
    }

    /// Security-Settings (Mindestrollen, erlaubte Einladungs-Domains). Lesbar für alle Mitglieder
    /// außer Gästen und Apps.
    async fn workspace_security_settings(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<SecuritySettingsGql> {
        let p = app(ctx)?
            .get_security_settings(request_ctx(ctx)?)
            .await
            .map_err(gql_err)?;
        Ok(p.into())
    }

    /// Volltextkonfiguration der Suche (`simple`, `german`, `english`). Nur für Owner/Admin.
    async fn workspace_fulltext_config(&self, ctx: &Context<'_>) -> async_graphql::Result<String> {
        app(ctx)?
            .get_fulltext_config(request_ctx(ctx)?)
            .await
            .map_err(gql_err)
    }

    /// Mitglieder mit E-Mail, Rolle und Status. Nur für Owner/Admin.
    async fn workspace_members(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = false)] include_suspended: bool,
        #[graphql(default = 250)] first: i32,
    ) -> async_graphql::Result<Vec<WorkspaceMemberGql>> {
        if !(1..=250).contains(&first) {
            return Err(gql_err(AppError::invalid_request(
                "`first` must be between 1 and 250",
            )));
        }
        let rows = app(ctx)?
            .list_members(request_ctx(ctx)?, include_suspended, i64::from(first))
            .await
            .map_err(gql_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    /// Für den Aufrufer sichtbare User (Assignee-Picker, Erwähnungen). Ohne E-Mail-Adressen
    /// (Sicherheits-Checkliste Punkt 6); optional auf ein Team und einen Suchbegriff begrenzt.
    async fn users(
        &self,
        ctx: &Context<'_>,
        query: Option<String>,
        team: Option<String>,
        #[graphql(default = 50)] first: i32,
    ) -> async_graphql::Result<Vec<UserRefGql>> {
        if !(1..=250).contains(&first) {
            return Err(gql_err(AppError::invalid_request(
                "`first` must be between 1 and 250",
            )));
        }
        let page = app(ctx)?
            .list_users(
                request_ctx(ctx)?,
                ListUsersInput {
                    query,
                    team,
                    page: PageRequest {
                        limit: i64::from(first),
                        ..Default::default()
                    },
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(page
            .items
            .into_iter()
            .filter(|u| u.active)
            .map(|u| UserRefGql {
                id: u.id.to_string(),
                name: u.name,
                display_name: u.display_name,
                avatar_url: u.avatar_url,
            })
            .collect())
    }
}

/// Öffentliche User-Angaben (ohne E-Mail).
#[derive(SimpleObject)]
#[graphql(name = "UserRef")]
pub struct UserRefGql {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
}

#[derive(Default)]
pub struct OrgMutation;

#[Object]
impl OrgMutation {
    /// Ändert Workspace-Settings (Admin/Owner).
    async fn workspace_update(
        &self,
        ctx: &Context<'_>,
        input: WorkspaceUpdateInput,
    ) -> async_graphql::Result<WorkspacePayload> {
        let org = app(ctx)?
            .update_workspace(
                request_ctx(ctx)?,
                UpdateWorkspaceInput {
                    name: input.name,
                    url_key: input.url_key,
                    fiscal_year_start_month: input.fiscal_year_start_month,
                    initiatives_enabled: input.initiatives_enabled,
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(WorkspacePayload {
            success: true,
            workspace: Some(org.into()),
        })
    }

    /// Ändert die Security-Settings (Admin/Owner; `adminManagementRole` nur Owner).
    async fn workspace_security_settings_update(
        &self,
        ctx: &Context<'_>,
        input: SecuritySettingsUpdateInput,
    ) -> async_graphql::Result<SecuritySettingsPayload> {
        let patch = input.into_patch().map_err(gql_err)?;
        let p = app(ctx)?
            .update_security_settings(request_ctx(ctx)?, patch)
            .await
            .map_err(gql_err)?;
        Ok(SecuritySettingsPayload {
            success: true,
            security_settings: p.into(),
        })
    }

    /// Setzt die Volltextkonfiguration der Suche (`simple`, `german`, `english`; Admin/Owner).
    /// Die Suchvektoren des Workspace werden dabei neu aufgebaut.
    async fn workspace_fulltext_config_update(
        &self,
        ctx: &Context<'_>,
        config: String,
    ) -> async_graphql::Result<String> {
        app(ctx)?
            .set_fulltext_config(request_ctx(ctx)?, &config)
            .await
            .map_err(gql_err)
    }

    /// Ändert die Rolle eines Users (`admin`, `user`, `guest`, `owner`; Regeln siehe docs/02 §6.4).
    async fn user_role_update(
        &self,
        ctx: &Context<'_>,
        user_id: String,
        role: String,
    ) -> async_graphql::Result<WorkspaceMemberPayload> {
        let role: UserRole = role.parse().map_err(|_| {
            gql_err(
                AppError::invalid_request("role must be owner, admin, user or guest")
                    .with_details(serde_json::json!({ "field": "role" })),
            )
        })?;
        let a = app(ctx)?;
        let u = a
            .set_user_role(request_ctx(ctx)?, &user_id, role)
            .await
            .map_err(gql_err)?;
        member_payload(ctx, &u.id.to_string()).await
    }

    /// Sperrt einen User: Sessions, API-Keys und OAuth-Tokens werden sofort ungültig.
    async fn user_suspend(
        &self,
        ctx: &Context<'_>,
        user_id: String,
    ) -> async_graphql::Result<WorkspaceMemberPayload> {
        let u = app(ctx)?
            .set_user_active(request_ctx(ctx)?, &user_id, false)
            .await
            .map_err(gql_err)?;
        member_payload(ctx, &u.id.to_string()).await
    }

    /// Hebt die Sperre auf (der User muss sich neu anmelden).
    async fn user_unsuspend(
        &self,
        ctx: &Context<'_>,
        user_id: String,
    ) -> async_graphql::Result<WorkspaceMemberPayload> {
        let u = app(ctx)?
            .set_user_active(request_ctx(ctx)?, &user_id, true)
            .await
            .map_err(gql_err)?;
        member_payload(ctx, &u.id.to_string()).await
    }
}

async fn member_payload(
    ctx: &Context<'_>,
    id: &str,
) -> async_graphql::Result<WorkspaceMemberPayload> {
    let view = app(ctx)?
        .get_user(request_ctx(ctx)?, id)
        .await
        .map_err(gql_err)?;
    Ok(WorkspaceMemberPayload {
        success: true,
        member: view.into(),
    })
}
