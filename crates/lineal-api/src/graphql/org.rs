//! GraphQL-Teilobjekte des Bereichs `org` (WP02): Workspace lesen und Settings ändern.

//! `createWorkspace` braucht eine Konto-Session ohne Workspace-Kontext und folgt mit WP03/WP16.

use crate::context::{app, request_ctx};
use async_graphql::{Context, ErrorExtensions, InputObject, Object, SimpleObject};
use lineal_app::AppError;
use lineal_app::ports::repos::org::Org;
use lineal_app::ports::repos::user::PageRequest;
use lineal_app::usecases::org::UpdateWorkspaceInput;
use lineal_app::usecases::user::ListUsersInput;

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
}
