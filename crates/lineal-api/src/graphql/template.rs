//! GraphQL-Teilobjekte des Bereichs `template` (WP10): Verwaltung, Default-Templates und
//! Anlegen eines Issues aus einem Template (docs/06 §2).

use super::issue::IssueGql;
use super::org::gql_err;
use crate::context::{app, request_ctx};
use async_graphql::{Context, Enum, InputObject, Json, Object, SimpleObject};
use chrono::{DateTime, SecondsFormat, Utc};
use lineal_app::AppError;
use lineal_app::ports::repos::template::DefaultSlot;
use lineal_app::usecases::issue::CreateIssueInput;
use lineal_app::usecases::template::{SaveTemplateInput, TemplateView};
use lineal_domain::enums::TemplateType;

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Template-Typ.
#[derive(Enum, Clone, Copy, PartialEq, Eq)]
#[graphql(name = "TemplateKind", rename_items = "camelCase")]
pub enum TemplateKindGql {
    Issue,
    Project,
    Document,
}

impl From<TemplateKindGql> for TemplateType {
    fn from(k: TemplateKindGql) -> Self {
        match k {
            TemplateKindGql::Issue => Self::Issue,
            TemplateKindGql::Project => Self::Project,
            TemplateKindGql::Document => Self::Document,
        }
    }
}

/// Welches Default-Template eines Teams.
#[derive(Enum, Clone, Copy, PartialEq, Eq)]
#[graphql(name = "TemplateDefaultSlot", rename_items = "camelCase")]
pub enum DefaultSlotGql {
    ForMembers,
    ForNonMembers,
    Project,
}

#[derive(SimpleObject)]
#[graphql(name = "Template")]
pub struct TemplateGql {
    pub id: String,
    pub name: String,
    #[graphql(name = "type")]
    pub template_type: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    /// `null` = Workspace-Template.
    pub team_id: Option<String>,
    pub team_key: Option<String>,
    pub template_data: Json<serde_json::Value>,
    pub sort_order: f64,
    pub inherited_from_id: Option<String>,
    pub is_form: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<TemplateView> for TemplateGql {
    fn from(v: TemplateView) -> Self {
        let t = v.template;
        Self {
            id: t.id.to_string(),
            name: t.name,
            template_type: t.template_type.as_str().to_owned(),
            description: t.description,
            icon: t.icon,
            color: t.color,
            team_id: t.team_id.map(|x| x.to_string()),
            team_key: v.team.map(|(_, key, _)| key),
            template_data: Json(t.template_data),
            sort_order: t.sort_order,
            inherited_from_id: t.inherited_from_id.map(|x| x.to_string()),
            is_form: v.is_form,
            created_at: ts(t.created_at),
            updated_at: ts(t.updated_at),
        }
    }
}

#[derive(SimpleObject)]
#[graphql(name = "TemplatePayload")]
pub struct TemplatePayload {
    pub success: bool,
    pub template: Option<TemplateGql>,
}

#[derive(InputObject)]
#[graphql(name = "TemplateCreateInput")]
pub struct TemplateCreateInput {
    #[graphql(name = "type")]
    pub template_type: TemplateKindGql,
    pub name: String,
    pub description: Option<String>,
    /// Ohne Team: Workspace-Template.
    pub team_id: Option<String>,
    pub template_data: Json<serde_json::Value>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub sort_order: Option<f64>,
}

#[derive(InputObject)]
#[graphql(name = "TemplateUpdateInput")]
pub struct TemplateUpdateInput {
    pub name: Option<String>,
    pub description: Option<String>,
    pub template_data: Option<Json<serde_json::Value>>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub sort_order: Option<f64>,
    /// Sub-Team, das ein geerbtes Template anpasst (es entsteht eine Kopie mit `inheritedFrom`).
    pub as_team_id: Option<String>,
}

#[derive(InputObject)]
#[graphql(name = "TemplateIssueCreateInput")]
pub struct TemplateIssueCreateInput {
    pub team_id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub state: Option<String>,
    pub priority: Option<i32>,
    pub assignee_id: Option<String>,
    pub label_ids: Option<Vec<String>>,
    pub project_id: Option<String>,
    pub cycle_id: Option<String>,
    pub parent_id: Option<String>,
    /// Felder, die ausdrücklich nicht gesetzt werden sollen (überschreibt das Template).
    pub clear_fields: Option<Vec<String>>,
}

#[derive(SimpleObject)]
#[graphql(name = "TemplateIssuePayload")]
pub struct TemplateIssuePayload {
    pub success: bool,
    pub issue: IssueGql,
    pub sub_issues: Vec<IssueGql>,
    pub warnings: Vec<String>,
}

/// Default-Templates eines Teams (Ids; `null` = keines gesetzt).
#[derive(SimpleObject)]
#[graphql(name = "TeamDefaultTemplates")]
pub struct TeamDefaultTemplatesGql {
    pub for_members_id: Option<String>,
    pub for_non_members_id: Option<String>,
    pub project_id: Option<String>,
}

#[derive(Default)]
pub struct TemplateQuery;

#[Object]
impl TemplateQuery {
    /// Templates des Teams (eigene, geerbte, Workspace) bzw. alle sichtbaren.
    async fn templates(
        &self,
        ctx: &Context<'_>,
        team_id: Option<String>,
        #[graphql(name = "type")] template_type: Option<TemplateKindGql>,
    ) -> async_graphql::Result<Vec<TemplateGql>> {
        Ok(app(ctx)?
            .list_templates(
                request_ctx(ctx)?,
                team_id.as_deref(),
                template_type.map(Into::into),
            )
            .await
            .map_err(gql_err)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Gesetzte Default-Templates eines Teams.
    async fn team_default_templates(
        &self,
        ctx: &Context<'_>,
        team_id: String,
    ) -> async_graphql::Result<TeamDefaultTemplatesGql> {
        let d = app(ctx)?
            .get_team_default_templates(request_ctx(ctx)?, &team_id)
            .await
            .map_err(gql_err)?;
        Ok(TeamDefaultTemplatesGql {
            for_members_id: d.members.map(|x| x.to_string()),
            for_non_members_id: d.non_members.map(|x| x.to_string()),
            project_id: d.project.map(|x| x.to_string()),
        })
    }

    async fn template(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<TemplateGql> {
        Ok(app(ctx)?
            .get_template(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?
            .into())
    }
}

fn payload(v: TemplateView) -> TemplatePayload {
    TemplatePayload {
        success: true,
        template: Some(v.into()),
    }
}

#[derive(Default)]
pub struct TemplateMutation;

#[Object]
impl TemplateMutation {
    async fn template_create(
        &self,
        ctx: &Context<'_>,
        input: TemplateCreateInput,
    ) -> async_graphql::Result<TemplatePayload> {
        let v = app(ctx)?
            .save_template(
                request_ctx(ctx)?,
                SaveTemplateInput {
                    template_type: Some(input.template_type.into()),
                    name: Some(input.name),
                    description: input.description,
                    team: input.team_id,
                    template_data: Some(input.template_data.0),
                    icon: input.icon,
                    color: input.color,
                    sort_order: input.sort_order,
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(payload(v))
    }

    async fn template_update(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: TemplateUpdateInput,
    ) -> async_graphql::Result<TemplatePayload> {
        let v = app(ctx)?
            .save_template(
                request_ctx(ctx)?,
                SaveTemplateInput {
                    id: Some(id),
                    name: input.name,
                    description: input.description,
                    template_data: input.template_data.map(|d| d.0),
                    icon: input.icon,
                    color: input.color,
                    sort_order: input.sort_order,
                    as_team: input.as_team_id,
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(payload(v))
    }

    async fn template_delete(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<TemplatePayload> {
        Ok(payload(
            app(ctx)?
                .delete_template(request_ctx(ctx)?, &id)
                .await
                .map_err(gql_err)?,
        ))
    }

    /// Setzt (oder löscht mit `templateId: null`) ein Default-Template des Teams.
    async fn team_default_template_set(
        &self,
        ctx: &Context<'_>,
        team_id: String,
        slot: DefaultSlotGql,
        template_id: Option<String>,
    ) -> async_graphql::Result<bool> {
        let slot = match slot {
            DefaultSlotGql::ForMembers => DefaultSlot::Members,
            DefaultSlotGql::ForNonMembers => DefaultSlot::NonMembers,
            DefaultSlotGql::Project => DefaultSlot::Project,
        };
        app(ctx)?
            .set_team_default_template(request_ctx(ctx)?, &team_id, slot, template_id.as_deref())
            .await
            .map_err(gql_err)?;
        Ok(true)
    }

    /// Legt ein Issue (inkl. Sub-Issues) aus einem Template an – eine Transaktion.
    async fn template_issue_create(
        &self,
        ctx: &Context<'_>,
        template_id: String,
        input: TemplateIssueCreateInput,
    ) -> async_graphql::Result<TemplateIssuePayload> {
        let priority = input.priority.map(i64::from);
        if priority.is_some_and(|p| !(0..=4).contains(&p)) {
            return Err(gql_err(AppError::invalid_request("priority must be 0-4")));
        }
        let out = app(ctx)?
            .create_issue_from_template(
                request_ctx(ctx)?,
                &template_id,
                CreateIssueInput {
                    team: input.team_id,
                    title: input.title,
                    description: input.description,
                    state: input.state,
                    priority,
                    assignee: input.assignee_id,
                    labels: input.label_ids,
                    project: input.project_id,
                    cycle: input.cycle_id,
                    parent: input.parent_id,
                    ..Default::default()
                },
                &input.clear_fields.unwrap_or_default(),
            )
            .await
            .map_err(gql_err)?;
        Ok(TemplateIssuePayload {
            success: true,
            issue: out.root.into(),
            sub_issues: out.children.into_iter().map(Into::into).collect(),
            warnings: out.warnings,
        })
    }
}
