//! GraphQL-Teilobjekte des Bereichs `initiative` (WP07): Initiativen (DAG), Projektzuordnung,
//! Initiative-Updates. Gäste erhalten `forbidden`; Initiativen mit privatem Lead-Team sind für
//! Nicht-Mitglieder `not_found`.

use super::org::gql_err;
use super::project::{SimpleRefGql, parse_health};
use crate::context::{app, request_ctx};
use async_graphql::{Context, InputObject, MaybeUndefined, Object, SimpleObject};
use lineal_app::AppError;
use lineal_app::ports::repos::user::PageRequest;
use lineal_app::usecases::initiative::update::{InitiativeUpdateView, ListInitiativeUpdatesInput};
use lineal_app::usecases::initiative::{
    InitiativeView, ListInitiativesInput, SaveInitiativeInput, initiative_status_ref,
};
use lineal_app::usecases::project::pure::parse_date;
use lineal_domain::enums::DateResolution;

#[derive(SimpleObject)]
#[graphql(name = "InitiativeProject")]
pub struct InitiativeProjectGql {
    pub id: String,
    pub identifier: String,
    pub name: String,
    pub status: String,
    pub status_type: String,
    pub health: Option<String>,
}

#[derive(SimpleObject)]
#[graphql(name = "InitiativeHealthRollup")]
pub struct HealthRollupGql {
    /// `onTrack` | `atRisk` | `offTrack`; `null` = grau.
    pub color: Option<String>,
    pub on_track: i32,
    pub at_risk: i32,
    pub off_track: i32,
    pub none: i32,
}

#[derive(SimpleObject)]
#[graphql(name = "Initiative")]
pub struct InitiativeGql {
    pub id: String,
    pub identifier: String,
    pub slug_id: String,
    pub name: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    /// `Proposed` | `Planned` | `Active` | `Completed` | `Canceled`.
    pub status: String,
    pub status_type: String,
    pub priority: i32,
    pub health: Option<String>,
    pub target_date: Option<String>,
    pub target_date_resolution: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub owner: Option<SimpleRefGql>,
    pub lead_team: Option<SimpleRefGql>,
    /// `public` | `restricted` | `private`.
    pub visibility: String,
    pub parent_initiatives: Vec<SimpleRefGql>,
    pub sub_initiatives: Vec<SimpleRefGql>,
    pub labels: Vec<SimpleRefGql>,
    pub projects: Vec<InitiativeProjectGql>,
    pub health_rollup: HealthRollupGql,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

fn iref(i: &lineal_app::ports::repos::initiative::Initiative) -> SimpleRefGql {
    SimpleRefGql {
        id: i.id.to_string(),
        name: i.name.clone(),
        key: Some(i.identifier()),
    }
}

impl From<InitiativeView> for InitiativeGql {
    fn from(v: InitiativeView) -> Self {
        let (_, sname, stype) = initiative_status_ref(v.initiative.status);
        let i = &v.initiative;
        Self {
            id: i.id.to_string(),
            identifier: v.identifier.clone(),
            slug_id: i.slug_id.clone(),
            name: i.name.clone(),
            summary: i.description.clone(),
            description: i.content.clone(),
            color: i.color.clone(),
            icon: i.icon.clone(),
            status: sname.into(),
            status_type: stype.into(),
            priority: i.priority,
            health: i.health.map(|h| h.as_str().into()),
            target_date: i.target_date.map(|d| d.to_string()),
            target_date_resolution: i.target_date_resolution.map(|r| r.as_str().into()),
            started_at: i.started_at.map(|d| d.to_rfc3339()),
            completed_at: i.completed_at.map(|d| d.to_rfc3339()),
            owner: v.owner.as_ref().map(|u| SimpleRefGql {
                id: u.id.to_string(),
                name: u.name.clone(),
                key: None,
            }),
            lead_team: v.lead_team.as_ref().map(|t| SimpleRefGql {
                id: t.id.to_string(),
                name: t.name.clone(),
                key: Some(t.key.clone()),
            }),
            visibility: v.visibility.into(),
            parent_initiatives: v.parents.iter().map(iref).collect(),
            sub_initiatives: v.sub_initiatives.iter().map(iref).collect(),
            labels: v
                .labels
                .iter()
                .map(|l| SimpleRefGql {
                    id: l.id.to_string(),
                    name: l.name.clone(),
                    key: None,
                })
                .collect(),
            projects: v
                .projects
                .iter()
                .map(|p| InitiativeProjectGql {
                    id: p.project.id.to_string(),
                    identifier: p.identifier.clone(),
                    name: p.project.name.clone(),
                    status: p.status.name.clone(),
                    status_type: p.status.status_type.as_str().into(),
                    health: p.project.health.map(|h| h.as_str().into()),
                })
                .collect(),
            health_rollup: HealthRollupGql {
                color: v.health_rollup.color().map(|h| h.as_str().into()),
                on_track: v.health_rollup.on_track as i32,
                at_risk: v.health_rollup.at_risk as i32,
                off_track: v.health_rollup.off_track as i32,
                none: v.health_rollup.none as i32,
            },
            archived_at: i.archived_at.map(|d| d.to_rfc3339()),
            created_at: i.created_at.to_rfc3339(),
            updated_at: i.updated_at.to_rfc3339(),
        }
    }
}

#[derive(SimpleObject)]
#[graphql(name = "InitiativeUpdate")]
pub struct InitiativeUpdateGql {
    pub id: String,
    pub initiative: SimpleRefGql,
    pub user: Option<SimpleRefGql>,
    pub body: String,
    pub health: String,
    pub diff_markdown: Option<String>,
    pub is_diff_hidden: bool,
    pub is_stale: bool,
    pub comment_count: i64,
    pub edited_at: Option<String>,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<InitiativeUpdateView> for InitiativeUpdateGql {
    fn from(v: InitiativeUpdateView) -> Self {
        let u = v.update;
        Self {
            id: u.id.to_string(),
            initiative: iref(&v.initiative),
            user: v.user.as_ref().map(|x| SimpleRefGql {
                id: x.id.to_string(),
                name: x.name.clone(),
                key: None,
            }),
            body: u.body,
            health: u.health.as_str().into(),
            diff_markdown: if u.is_diff_hidden {
                None
            } else {
                u.diff_markdown
            },
            is_diff_hidden: u.is_diff_hidden,
            is_stale: v.is_stale,
            comment_count: v.comment_count,
            edited_at: u.edited_at.map(|d| d.to_rfc3339()),
            archived_at: u.archived_at.map(|d| d.to_rfc3339()),
            created_at: u.created_at.to_rfc3339(),
            updated_at: u.updated_at.to_rfc3339(),
        }
    }
}

#[derive(InputObject, Default)]
pub struct InitiativeInput {
    pub name: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub priority: Option<i32>,
    pub status: Option<String>,
    pub owner: MaybeUndefined<String>,
    pub lead_team: MaybeUndefined<String>,
    /// Fügt Eltern hinzu (DAG; Zyklus → `invalid_request` mit `details.reason = "cycle"`).
    pub parent_initiatives: Option<Vec<String>>,
    pub labels: Option<Vec<String>>,
    pub target_date: MaybeUndefined<String>,
    pub target_date_resolution: MaybeUndefined<String>,
}

#[derive(SimpleObject)]
pub struct InitiativePayload {
    pub success: bool,
    pub initiative: Option<InitiativeGql>,
}

#[derive(SimpleObject)]
pub struct InitiativeUpdatePayload {
    pub success: bool,
    pub initiative_update: Option<InitiativeUpdateGql>,
}

#[derive(SimpleObject)]
pub struct InitiativeSuccessPayload {
    pub success: bool,
}

fn tri(v: MaybeUndefined<String>) -> Option<Option<String>> {
    match v {
        MaybeUndefined::Undefined => None,
        MaybeUndefined::Null => Some(None),
        MaybeUndefined::Value(x) => Some(Some(x)),
    }
}

fn to_input(id: Option<String>, i: InitiativeInput) -> async_graphql::Result<SaveInitiativeInput> {
    let target_date = match i.target_date {
        MaybeUndefined::Undefined => None,
        MaybeUndefined::Null => Some(None),
        MaybeUndefined::Value(s) => Some(Some(parse_date("targetDate", &s).map_err(gql_err)?)),
    };
    let target_date_resolution = match i.target_date_resolution {
        MaybeUndefined::Undefined => None,
        MaybeUndefined::Null => Some(None),
        MaybeUndefined::Value(s) => Some(Some(s.parse::<DateResolution>().map_err(|_| {
            gql_err(AppError::invalid_request(
                "`targetDateResolution` is not valid",
            ))
        })?)),
    };
    Ok(SaveInitiativeInput {
        id,
        name: i.name,
        summary: i.summary,
        description: i.description,
        patch: None,
        icon: i.icon,
        color: i.color,
        priority: i.priority,
        status: i.status,
        owner: tri(i.owner),
        lead_team: tri(i.lead_team),
        parent_initiatives: i.parent_initiatives.unwrap_or_default(),
        labels: i.labels,
        target_date,
        target_date_resolution,
    })
}

#[derive(Default)]
pub struct InitiativeQuery;

async fn list_trashed_initiatives(
    ctx: &Context<'_>,
    first: Option<i32>,
) -> async_graphql::Result<Vec<InitiativeGql>> {
    let (_, views) = app(ctx)?
        .list_initiatives(
            request_ctx(ctx)?,
            ListInitiativesInput {
                trashed: true,
                page: PageRequest {
                    limit: i64::from(crate::error::GqlResultExt::gql(
                        crate::pagination::first_arg(first, 50, 250),
                        ctx,
                    )?),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .map_err(gql_err)?;
    Ok(views.into_iter().map(Into::into).collect())
}

#[Object]
impl InitiativeQuery {
    #[allow(clippy::too_many_arguments)]
    async fn initiatives(
        &self,
        ctx: &Context<'_>,
        status: Option<String>,
        owner: Option<String>,
        lead_team: Option<String>,
        parent_initiative: Option<String>,
        label: Option<String>,
        query: Option<String>,
        include_archived: Option<bool>,
        first: Option<i32>,
    ) -> async_graphql::Result<Vec<InitiativeGql>> {
        let (_, views) = app(ctx)?
            .list_initiatives(
                request_ctx(ctx)?,
                ListInitiativesInput {
                    status,
                    owner,
                    lead_team,
                    parent_initiative,
                    label,
                    query,
                    include_archived: include_archived.unwrap_or(false),
                    page: PageRequest {
                        limit: i64::from(crate::error::GqlResultExt::gql(
                            crate::pagination::first_arg(first, 50, 250),
                            ctx,
                        )?),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(views.into_iter().map(Into::into).collect())
    }

    /// „Recently deleted“: gelöschte Initiativen innerhalb der Aufbewahrungsfrist.
    /// Wiederherstellen: `initiativeDelete(restore: true)`.
    async fn trashed_initiatives(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
    ) -> async_graphql::Result<Vec<InitiativeGql>> {
        Box::pin(list_trashed_initiatives(ctx, first)).await
    }

    /// Eine Initiative per UUID, Kennung (`I-4`), Slug oder Name.
    async fn initiative(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<InitiativeGql> {
        Ok(app(ctx)?
            .get_initiative(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?
            .into())
    }

    /// Status-Updates von Initiativen (neueste zuerst).
    async fn initiative_updates(
        &self,
        ctx: &Context<'_>,
        initiative_id: Option<String>,
        include_archived: Option<bool>,
        first: Option<i32>,
    ) -> async_graphql::Result<Vec<InitiativeUpdateGql>> {
        let (_, views) = app(ctx)?
            .list_initiative_updates(
                request_ctx(ctx)?,
                ListInitiativeUpdatesInput {
                    initiative: initiative_id,
                    include_archived: include_archived.unwrap_or(false),
                    page: PageRequest {
                        limit: i64::from(crate::error::GqlResultExt::gql(
                            crate::pagination::first_arg(first, 50, 250),
                            ctx,
                        )?),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(views.into_iter().map(Into::into).collect())
    }
}

#[derive(Default)]
pub struct InitiativeMutation;

#[Object]
impl InitiativeMutation {
    async fn initiative_create(
        &self,
        ctx: &Context<'_>,
        input: InitiativeInput,
    ) -> async_graphql::Result<InitiativePayload> {
        let v = app(ctx)?
            .save_initiative(request_ctx(ctx)?, to_input(None, input)?)
            .await
            .map_err(gql_err)?;
        Ok(InitiativePayload {
            success: true,
            initiative: Some(v.into()),
        })
    }

    async fn initiative_update(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: InitiativeInput,
    ) -> async_graphql::Result<InitiativePayload> {
        let v = app(ctx)?
            .save_initiative(request_ctx(ctx)?, to_input(Some(id), input)?)
            .await
            .map_err(gql_err)?;
        Ok(InitiativePayload {
            success: true,
            initiative: Some(v.into()),
        })
    }

    /// Entfernt eine Eltern-Beziehung.
    async fn initiative_parent_remove(
        &self,
        ctx: &Context<'_>,
        initiative_id: String,
        parent_id: String,
    ) -> async_graphql::Result<InitiativeSuccessPayload> {
        let removed = app(ctx)?
            .remove_initiative_parent(request_ctx(ctx)?, &initiative_id, &parent_id)
            .await
            .map_err(gql_err)?;
        Ok(InitiativeSuccessPayload { success: removed })
    }

    /// Ordnet ein Projekt der Initiative zu.
    async fn initiative_to_project_create(
        &self,
        ctx: &Context<'_>,
        initiative_id: String,
        project_id: String,
    ) -> async_graphql::Result<InitiativeSuccessPayload> {
        app(ctx)?
            .set_initiative_project(request_ctx(ctx)?, &initiative_id, &project_id, true)
            .await
            .map_err(gql_err)?;
        Ok(InitiativeSuccessPayload { success: true })
    }

    /// Löst die Zuordnung eines Projekts.
    async fn initiative_to_project_delete(
        &self,
        ctx: &Context<'_>,
        initiative_id: String,
        project_id: String,
    ) -> async_graphql::Result<InitiativeSuccessPayload> {
        app(ctx)?
            .set_initiative_project(request_ctx(ctx)?, &initiative_id, &project_id, false)
            .await
            .map_err(gql_err)?;
        Ok(InitiativeSuccessPayload { success: true })
    }

    async fn initiative_archive(
        &self,
        ctx: &Context<'_>,
        id: String,
        archived: Option<bool>,
    ) -> async_graphql::Result<InitiativeSuccessPayload> {
        app(ctx)?
            .set_initiative_archived(request_ctx(ctx)?, &id, archived.unwrap_or(true))
            .await
            .map_err(gql_err)?;
        Ok(InitiativeSuccessPayload { success: true })
    }

    /// Papierkorb (30 Tage) bzw. Wiederherstellen (`restore: true`, nur per UUID).
    async fn initiative_delete(
        &self,
        ctx: &Context<'_>,
        id: String,
        restore: Option<bool>,
    ) -> async_graphql::Result<InitiativeSuccessPayload> {
        if restore.unwrap_or(false) {
            app(ctx)?
                .restore_initiative(request_ctx(ctx)?, &id)
                .await
                .map_err(gql_err)?;
        } else {
            app(ctx)?
                .trash_initiative(request_ctx(ctx)?, &id)
                .await
                .map_err(gql_err)?;
        }
        Ok(InitiativeSuccessPayload { success: true })
    }

    async fn initiative_update_create(
        &self,
        ctx: &Context<'_>,
        initiative_id: String,
        body: String,
        health: Option<String>,
        is_diff_hidden: Option<bool>,
    ) -> async_graphql::Result<InitiativeUpdatePayload> {
        let v = app(ctx)?
            .post_initiative_update(
                request_ctx(ctx)?,
                &initiative_id,
                &body,
                parse_health(health)?,
                is_diff_hidden.unwrap_or(false),
            )
            .await
            .map_err(gql_err)?;
        Ok(InitiativeUpdatePayload {
            success: true,
            initiative_update: Some(v.into()),
        })
    }

    async fn initiative_update_edit(
        &self,
        ctx: &Context<'_>,
        id: String,
        body: Option<String>,
        health: Option<String>,
    ) -> async_graphql::Result<InitiativeUpdatePayload> {
        let v = app(ctx)?
            .edit_initiative_update(
                request_ctx(ctx)?,
                &id,
                body.as_deref(),
                parse_health(health)?,
            )
            .await
            .map_err(gql_err)?;
        Ok(InitiativeUpdatePayload {
            success: true,
            initiative_update: Some(v.into()),
        })
    }

    async fn initiative_update_delete(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<InitiativeSuccessPayload> {
        app(ctx)?
            .delete_initiative_update(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?;
        Ok(InitiativeSuccessPayload { success: true })
    }
}
