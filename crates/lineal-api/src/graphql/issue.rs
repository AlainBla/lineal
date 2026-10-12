//! GraphQL-Teilobjekte des Bereichs `issue` (WP06): `issue`, `issues`, `issueCreate`,
//! `issueUpdate`, `issueArchive`, `issueUnarchive`, `issueDelete`, `issueRelationCreate`.

use super::org::gql_err;
use crate::context::{app, charge_detail_field, request_ctx};
use async_graphql::{Context, InputObject, MaybeUndefined, Object, SimpleObject};
use chrono::{DateTime, NaiveDate, Utc};
use lineal_app::ports::repos::issue::HistorySource;
use lineal_app::ports::repos::user::{OrderBy, PageCursor};
use lineal_app::usecases::issue::{
    BatchPatch, BatchUpdateInput, CreateIssueInput, DescriptionEdit, IssueView, ListIssuesInput,
    RelationChanges, UpdateIssueInput,
};
use lineal_app::usecases::template::FormAnswer;
use lineal_app::{AppError, AppResult};
use lineal_domain::enums::RelationType;

/// `Issue`. `children`, `relations` und `history` sind eigene Felder (Detailansicht); sie laden
/// beim Zugriff mit den Rechten des Aufrufers nach, in Listen also bitte nicht anfordern.
#[derive(SimpleObject)]
#[graphql(name = "Issue", complex)]
pub struct IssueGql {
    pub id: String,
    pub identifier: String,
    pub previous_identifiers: Vec<String>,
    pub title: String,
    pub description: Option<String>,
    pub priority: i32,
    pub estimate: Option<i32>,
    pub team_id: String,
    pub state_id: String,
    pub state_name: String,
    pub state_type: String,
    pub assignee_id: Option<String>,
    pub creator_id: Option<String>,
    pub parent_id: Option<String>,
    pub project_id: Option<String>,
    pub project_milestone_id: Option<String>,
    pub cycle_id: Option<String>,
    pub label_ids: Vec<String>,
    pub due_date: Option<String>,
    pub sort_order: f64,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub canceled_at: Option<String>,
    pub archived_at: Option<String>,
    pub trashed: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<IssueView> for IssueGql {
    fn from(v: IssueView) -> Self {
        let i = v.issue;
        let ts = |t: Option<DateTime<Utc>>| t.map(|t| t.to_rfc3339());
        Self {
            id: i.id.to_string(),
            identifier: i.identifier,
            previous_identifiers: v.previous_identifiers,
            title: i.title,
            description: i.description,
            priority: i32::from(i.priority.value()),
            estimate: i.estimate,
            team_id: i.team_id.to_string(),
            state_id: i.state_id.to_string(),
            state_name: v.extras.state_name,
            state_type: i.state_type.as_str().into(),
            assignee_id: i.assignee_id.map(|u| u.to_string()),
            creator_id: i.creator_id.map(|u| u.to_string()),
            parent_id: i.parent_id.map(|u| u.to_string()),
            project_id: i.project_id.map(|u| u.to_string()),
            project_milestone_id: i.project_milestone_id.map(|u| u.to_string()),
            cycle_id: i.cycle_id.map(|u| u.to_string()),
            label_ids: v.extras.label_ids.iter().map(ToString::to_string).collect(),
            due_date: i.due_date.map(|d| d.to_string()),
            sort_order: i.sort_order,
            started_at: ts(i.started_at),
            completed_at: ts(i.completed_at),
            canceled_at: ts(i.canceled_at),
            archived_at: ts(i.archived_at),
            trashed: i.trashed,
            created_at: i.created_at.to_rfc3339(),
            updated_at: i.updated_at.to_rfc3339(),
        }
    }
}

/// Verwandtes Issue (Kennung und Titel).
#[derive(SimpleObject)]
pub struct RelatedIssue {
    pub id: String,
    pub identifier: String,
    pub title: String,
}

impl From<lineal_app::usecases::issue::RelatedIssue> for RelatedIssue {
    fn from(r: lineal_app::usecases::issue::RelatedIssue) -> Self {
        Self {
            id: r.uuid.to_string(),
            identifier: r.identifier,
            title: r.title,
        }
    }
}

/// Relationen eines Issues aus dessen Sicht (nur für den Aufrufer sichtbare Gegenstücke).
#[derive(SimpleObject)]
pub struct IssueRelations {
    pub blocks: Vec<RelatedIssue>,
    pub blocked_by: Vec<RelatedIssue>,
    pub related_to: Vec<RelatedIssue>,
    pub duplicate_of: Option<RelatedIssue>,
    pub duplicates: Vec<RelatedIssue>,
}

/// Ein Eintrag der Aktivitätshistorie.
#[derive(SimpleObject)]
pub struct IssueHistoryEntry {
    pub id: String,
    pub actor_id: Option<String>,
    pub source: String,
    /// `{ "<feld>": { "from": …, "to": … } }` (camelCase-Feldnamen).
    pub changes: async_graphql::types::Json<serde_json::Value>,
    pub added_label_ids: Vec<String>,
    pub removed_label_ids: Vec<String>,
    pub relation_changes: Option<async_graphql::types::Json<serde_json::Value>>,
    pub flags: Vec<String>,
    pub created_at: String,
}

#[async_graphql::ComplexObject]
impl IssueGql {
    /// Direkte Sub-Issues (teamübergreifend, nur sichtbare), höchstens `first` (Default 50, max 100).
    async fn children(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
    ) -> async_graphql::Result<Vec<IssueGql>> {
        charge_detail_field(ctx)?;
        let page = app(ctx)?
            .list_issues(
                request_ctx(ctx)?,
                ListIssuesInput {
                    parent: Some(self.id.clone()),
                    include_archived: false,
                    order_by: OrderBy::UpdatedAt,
                    limit: i64::from(first.unwrap_or(50).clamp(1, 100)),
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(page.items.into_iter().map(Into::into).collect())
    }

    /// Relationen dieses Issues.
    async fn relations(&self, ctx: &Context<'_>) -> async_graphql::Result<IssueRelations> {
        charge_detail_field(ctx)?;
        let r = app(ctx)?
            .issue_relations(request_ctx(ctx)?, &self.id)
            .await
            .map_err(gql_err)?;
        let list = |v: Vec<lineal_app::usecases::issue::RelatedIssue>| {
            v.into_iter().map(Into::into).collect::<Vec<_>>()
        };
        Ok(IssueRelations {
            blocks: list(r.blocks),
            blocked_by: list(r.blocked_by),
            related_to: list(r.related_to),
            duplicate_of: r.duplicate_of.map(Into::into),
            duplicates: list(r.duplicates),
        })
    }

    /// Ob der Aufrufer dieses Issue abonniert hat.
    async fn subscribed(&self, ctx: &Context<'_>) -> async_graphql::Result<bool> {
        charge_detail_field(ctx)?;
        app(ctx)?
            .issue_subscribed(request_ctx(ctx)?, &self.id)
            .await
            .map_err(gql_err)
    }

    /// Aktivitätsverlauf, älteste zuerst (höchstens 1000 Einträge).
    async fn history(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<IssueHistoryEntry>> {
        charge_detail_field(ctx)?;
        let entries = app(ctx)?
            .issue_history(request_ctx(ctx)?, &self.id)
            .await
            .map_err(gql_err)?;
        let ids = |v: Option<Vec<lineal_domain::ids::LabelId>>| {
            v.unwrap_or_default()
                .into_iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
        };
        Ok(entries
            .into_iter()
            .map(|e| IssueHistoryEntry {
                id: e.id.to_string(),
                actor_id: e.actor_id.map(|u| u.to_string()),
                source: e.source.as_str().to_owned(),
                changes: async_graphql::types::Json(e.changes),
                added_label_ids: ids(e.added_label_ids),
                removed_label_ids: ids(e.removed_label_ids),
                relation_changes: e.relation_changes.map(async_graphql::types::Json),
                flags: e.flags,
                created_at: e.created_at.to_rfc3339(),
            })
            .collect())
    }
}

#[derive(SimpleObject)]
pub struct IssuePayload {
    pub success: bool,
    pub issue: Option<IssueGql>,
}

#[derive(SimpleObject)]
pub struct IssueConnection {
    pub nodes: Vec<IssueGql>,
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

#[derive(InputObject)]
pub struct IssueCreateInput {
    pub id: Option<String>,
    pub team_id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub state_id: Option<String>,
    pub assignee_id: Option<String>,
    pub priority: Option<i32>,
    pub estimate: Option<f64>,
    pub label_ids: Option<Vec<String>>,
    pub parent_id: Option<String>,
    pub project_id: Option<String>,
    pub project_milestone_id: Option<String>,
    pub cycle_id: Option<String>,
    pub due_date: Option<String>,
    pub sort_order: Option<f64>,
    /// Template (UUID oder Name); hat Vorrang vor `useDefaultTemplate`.
    pub template_id: Option<String>,
    /// Wendet das Default-Template des Teams an (Mitglieder: `defaultTemplateForMembers`, sonst
    /// `defaultTemplateForNonMembers`, docs/06 §2.8).
    pub use_default_template: Option<bool>,
    /// Antworten eines Form-Templates.
    pub form_answers: Option<Vec<FormAnswerInput>>,
}

/// Antwort auf ein Feld eines Form-Templates.
#[derive(InputObject)]
pub struct FormAnswerInput {
    pub field_id: String,
    /// Text, Zahl oder Liste von Texten (JSON).
    pub value: async_graphql::Json<serde_json::Value>,
}

/// Tri-State: ein weggelassenes Feld bleibt unverändert, ein explizites `null` leert es.
#[derive(InputObject)]
pub struct IssueUpdateInput {
    pub title: Option<String>,
    pub description: Option<String>,
    pub state_id: Option<String>,
    pub assignee_id: MaybeUndefined<String>,
    pub priority: Option<i32>,
    pub estimate: MaybeUndefined<f64>,
    pub team_id: Option<String>,
    pub parent_id: MaybeUndefined<String>,
    pub label_ids: Option<Vec<String>>,
    pub added_label_ids: Option<Vec<String>>,
    pub removed_label_ids: Option<Vec<String>>,
    pub project_id: MaybeUndefined<String>,
    pub project_milestone_id: MaybeUndefined<String>,
    pub cycle_id: MaybeUndefined<String>,
    pub due_date: MaybeUndefined<String>,
    pub sort_order: Option<f64>,
    /// Bei einem Teamwechsel auch alle Sub-Issues verschieben (Default `false`).
    pub move_sub_issues: Option<bool>,
}

/// Mehrfachaktion (`issueBatchUpdate`): dieselbe Änderung für bis zu 250 Issues.
#[derive(InputObject)]
pub struct IssueBatchUpdateInput {
    pub ids: Vec<String>,
    pub state_id: Option<String>,
    pub priority: Option<i32>,
    pub assignee_id: MaybeUndefined<String>,
    pub added_label_ids: Option<Vec<String>>,
    pub removed_label_ids: Option<Vec<String>>,
    pub project_id: MaybeUndefined<String>,
    pub cycle_id: MaybeUndefined<String>,
    pub estimate: MaybeUndefined<f64>,
    /// Zielteam (Bulk-Move).
    pub team_id: Option<String>,
    pub move_sub_issues: Option<bool>,
    pub archive: Option<bool>,
    /// In den Papierkorb legen.
    pub delete: Option<bool>,
}

/// Fehler eines einzelnen Issues in `issueBatchUpdate`.
#[derive(SimpleObject)]
pub struct IssueBatchFailure {
    pub id: String,
    pub code: String,
    pub message: String,
}

#[derive(SimpleObject)]
pub struct IssueBatchPayload {
    /// `true`, wenn mindestens ein Issue geändert wurde.
    pub success: bool,
    pub issues: Vec<IssueGql>,
    pub failures: Vec<IssueBatchFailure>,
}

#[derive(InputObject)]
pub struct IssueRelationCreateInput {
    pub issue_id: String,
    pub related_issue_id: String,
    /// `blocks`, `related` oder `duplicate`.
    #[graphql(name = "type")]
    pub relation_type: String,
}

fn date(s: &str) -> AppResult<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| AppError::invalid_request("dueDate must be YYYY-MM-DD"))
}

fn tri<T>(m: MaybeUndefined<T>) -> Option<Option<T>> {
    match m {
        MaybeUndefined::Undefined => None,
        MaybeUndefined::Null => Some(None),
        MaybeUndefined::Value(v) => Some(Some(v)),
    }
}

fn tri_date(v: Option<Option<String>>) -> AppResult<Option<Option<NaiveDate>>> {
    match v {
        None => Ok(None),
        Some(None) => Ok(Some(None)),
        Some(Some(s)) => Ok(Some(Some(date(&s)?))),
    }
}

fn payload(v: IssueView) -> IssuePayload {
    IssuePayload {
        success: true,
        issue: Some(v.into()),
    }
}

#[derive(Default)]
pub struct IssueQuery;

#[Object]
impl IssueQuery {
    /// Ein Issue per UUID oder Kennung (auch frühere); nicht sichtbar → `not_found`.
    async fn issue(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<IssueGql> {
        let d = app(ctx)?
            .get_issue(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?;
        Ok(d.view.into())
    }

    /// Issues der sichtbaren Teams.
    #[allow(clippy::too_many_arguments)]
    async fn issues(
        &self,
        ctx: &Context<'_>,
        team_id: Option<String>,
        assignee_id: Option<String>,
        state: Option<String>,
        label: Option<String>,
        project_id: Option<String>,
        cycle_id: Option<String>,
        priority: Option<i32>,
        include_archived: Option<bool>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<IssueConnection> {
        let cursor = after
            .as_deref()
            .map(|c| -> AppResult<PageCursor> {
                let (k, i) = c
                    .split_once('|')
                    .ok_or_else(|| AppError::invalid_request("invalid cursor"))?;
                Ok(PageCursor {
                    key: DateTime::parse_from_rfc3339(k)
                        .map_err(|_| AppError::invalid_request("invalid cursor"))?
                        .with_timezone(&Utc),
                    id: i
                        .parse()
                        .map_err(|_| AppError::invalid_request("invalid cursor"))?,
                })
            })
            .transpose()
            .map_err(gql_err)?;
        let page = app(ctx)?
            .list_issues(
                request_ctx(ctx)?,
                ListIssuesInput {
                    team: team_id,
                    assignee: assignee_id.map(Some),
                    state,
                    label,
                    project: project_id,
                    cycle: cycle_id,
                    priority: priority.map(i64::from),
                    include_archived: include_archived.unwrap_or(false),
                    order_by: OrderBy::UpdatedAt,
                    limit: i64::from(first.unwrap_or(50)),
                    cursor,
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(IssueConnection {
            has_next_page: page.next.is_some(),
            end_cursor: page.next.map(|c| {
                format!(
                    "{}|{}",
                    c.key.to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
                    c.id
                )
            }),
            nodes: page.items.into_iter().map(Into::into).collect(),
        })
    }
}

#[derive(Default)]
pub struct IssueMutation;

#[Object]
impl IssueMutation {
    async fn issue_create(
        &self,
        ctx: &Context<'_>,
        input: IssueCreateInput,
    ) -> async_graphql::Result<IssuePayload> {
        let id = input
            .id
            .as_deref()
            .map(|s| s.parse())
            .transpose()
            .map_err(|_| gql_err(AppError::invalid_request("id must be a UUID")))?;
        let a = app(ctx)?;
        let rctx = request_ctx(ctx)?;
        let template = match (&input.template_id, input.use_default_template) {
            (Some(t), _) => Some(t.clone()),
            (None, Some(true)) => a
                .default_template_for_issue(rctx, &input.team_id)
                .await
                .map_err(gql_err)?,
            _ => None,
        };
        let answers: Vec<FormAnswer> = input
            .form_answers
            .unwrap_or_default()
            .into_iter()
            .map(|x| FormAnswer {
                field_id: x.field_id,
                value: x.value.0,
            })
            .collect();
        let create = CreateIssueInput {
            id,
            team: input.team_id,
            title: input.title,
            description: input.description,
            state: input.state_id,
            priority: input.priority.map(i64::from),
            assignee: input.assignee_id,
            estimate: input.estimate,
            labels: input.label_ids,
            due_date: input
                .due_date
                .as_deref()
                .map(date)
                .transpose()
                .map_err(gql_err)?,
            cycle: input.cycle_id,
            project: input.project_id,
            milestone: input.project_milestone_id,
            parent: input.parent_id,
            sort_order: input.sort_order,
            source: HistorySource::Api,
            ..Default::default()
        };
        let v = match template {
            Some(t) => {
                a.create_issue_with_form(rctx, &t, create, answers, &[])
                    .await
                    .map_err(gql_err)?
                    .root
            }
            None => {
                if !answers.is_empty() {
                    return Err(gql_err(AppError::invalid_request(
                        "formAnswers require a form template",
                    )));
                }
                a.create_issue(rctx, create).await.map_err(gql_err)?
            }
        };
        Ok(payload(v))
    }

    async fn issue_update(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: IssueUpdateInput,
    ) -> async_graphql::Result<IssuePayload> {
        let v = app(ctx)?
            .update_issue(
                request_ctx(ctx)?,
                UpdateIssueInput {
                    id,
                    team: input.team_id,
                    title: input.title,
                    description: input.description.map(DescriptionEdit::Set),
                    state: input.state_id,
                    priority: input.priority.map(i64::from),
                    assignee: tri(input.assignee_id),
                    estimate: tri(input.estimate),
                    labels: input.label_ids,
                    add_labels: input.added_label_ids.unwrap_or_default(),
                    remove_labels: input.removed_label_ids.unwrap_or_default(),
                    due_date: tri_date(tri(input.due_date)).map_err(gql_err)?,
                    cycle: tri(input.cycle_id),
                    project: tri(input.project_id),
                    milestone: tri(input.project_milestone_id),
                    parent: tri(input.parent_id),
                    sort_order: input.sort_order,
                    move_sub_issues: input.move_sub_issues.unwrap_or(false),
                    source: HistorySource::Api,
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(payload(v))
    }

    /// Wendet eine Änderung auf mehrere Issues an (je Issue transaktional, Teilerfolge möglich).
    async fn issue_batch_update(
        &self,
        ctx: &Context<'_>,
        input: IssueBatchUpdateInput,
    ) -> async_graphql::Result<IssueBatchPayload> {
        let result = app(ctx)?
            .issue_batch_update(
                request_ctx(ctx)?,
                BatchUpdateInput {
                    ids: input.ids,
                    patch: BatchPatch {
                        state: input.state_id,
                        priority: input.priority.map(i64::from),
                        assignee: tri(input.assignee_id),
                        add_labels: input.added_label_ids.unwrap_or_default(),
                        remove_labels: input.removed_label_ids.unwrap_or_default(),
                        project: tri(input.project_id),
                        cycle: tri(input.cycle_id),
                        estimate: tri(input.estimate),
                        team: input.team_id,
                        move_sub_issues: input.move_sub_issues.unwrap_or(false),
                        archive: input.archive.unwrap_or(false),
                        delete: input.delete.unwrap_or(false),
                    },
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(IssueBatchPayload {
            success: !result.updated.is_empty(),
            issues: result.updated.into_iter().map(Into::into).collect(),
            failures: result
                .failures
                .into_iter()
                .map(|f| IssueBatchFailure {
                    id: f.id,
                    code: f.code,
                    message: f.message,
                })
                .collect(),
        })
    }

    /// Legt das Issue in den Papierkorb (`trash`) bzw. stellt es mit `issueUnarchive` wieder her.
    async fn issue_archive(
        &self,
        ctx: &Context<'_>,
        id: String,
        trash: Option<bool>,
    ) -> async_graphql::Result<IssuePayload> {
        let _ = trash;
        let issue = app(ctx)?
            .delete_issue(request_ctx(ctx)?, &id, false)
            .await
            .map_err(gql_err)?;
        Ok(IssuePayload {
            success: true,
            issue: Some(
                IssueView {
                    extras: Default::default(),
                    previous_identifiers: vec![],
                    issue,
                }
                .into(),
            ),
        })
    }

    async fn issue_unarchive(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<IssuePayload> {
        let v = app(ctx)?
            .restore_issue(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?;
        Ok(payload(v))
    }

    async fn issue_delete(
        &self,
        ctx: &Context<'_>,
        id: String,
        permanently_delete: Option<bool>,
    ) -> async_graphql::Result<IssuePayload> {
        let issue = app(ctx)?
            .delete_issue(request_ctx(ctx)?, &id, permanently_delete.unwrap_or(false))
            .await
            .map_err(gql_err)?;
        Ok(IssuePayload {
            success: true,
            issue: Some(
                IssueView {
                    extras: Default::default(),
                    previous_identifiers: vec![],
                    issue,
                }
                .into(),
            ),
        })
    }

    async fn issue_relation_create(
        &self,
        ctx: &Context<'_>,
        input: IssueRelationCreateInput,
    ) -> async_graphql::Result<IssuePayload> {
        let kind: RelationType = input
            .relation_type
            .parse()
            .map_err(|_| gql_err(AppError::invalid_request("unknown relation type")))?;
        let r = input.related_issue_id;
        let relations = match kind {
            RelationType::Blocks => RelationChanges {
                blocks: vec![r],
                ..Default::default()
            },
            RelationType::Related => RelationChanges {
                related_to: vec![r],
                ..Default::default()
            },
            RelationType::Duplicate => RelationChanges {
                duplicate_of: Some(Some(r)),
                ..Default::default()
            },
            RelationType::Similar => {
                return Err(gql_err(AppError::invalid_request(
                    "similar relations cannot be created manually",
                )));
            }
        };
        let v = app(ctx)?
            .update_issue(
                request_ctx(ctx)?,
                UpdateIssueInput {
                    id: input.issue_id,
                    relations,
                    source: HistorySource::Api,
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(payload(v))
    }
}
