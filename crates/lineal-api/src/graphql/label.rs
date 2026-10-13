//! GraphQL-Teilobjekte des Bereichs `label` (WP06): `issueLabels`, `issueLabelCreate`,
//! `issueLabelUpdate`, `issueLabelRetire`, `issueLabelRestore`.

use super::org::gql_err;
use crate::context::{app, request_ctx};
use async_graphql::{Context, InputObject, MaybeUndefined, Object, SimpleObject};
use lineal_app::AppError;
use lineal_app::ports::repos::label::LabelKind;
use lineal_app::usecases::label::{LabelView, ListLabelsInput, SaveLabelInput};
use lineal_domain::enums::LabelGroupType;

/// `IssueLabel`.
#[derive(SimpleObject)]
#[graphql(name = "IssueLabel")]
pub struct IssueLabelGql {
    pub id: String,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
    pub team_id: Option<String>,
    pub parent_id: Option<String>,
    pub is_group: bool,
    pub group_type: Option<String>,
    pub retired_at: Option<String>,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<LabelView> for IssueLabelGql {
    fn from(v: LabelView) -> Self {
        let l = v.label;
        Self {
            id: l.id.to_string(),
            group_type: l
                .is_group
                .then(|| l.effective_group_type().as_str().to_owned()),
            name: l.name,
            color: l.color,
            description: l.description,
            team_id: l.team_id.map(|t| t.to_string()),
            parent_id: l.parent_id.map(|p| p.to_string()),
            is_group: l.is_group,
            retired_at: l.retired_at.map(|t| t.to_rfc3339()),
            archived_at: l.archived_at.map(|t| t.to_rfc3339()),
            created_at: l.created_at.to_rfc3339(),
            updated_at: l.updated_at.to_rfc3339(),
        }
    }
}

#[derive(SimpleObject)]
pub struct IssueLabelPayload {
    pub success: bool,
    pub issue_label: Option<IssueLabelGql>,
}

/// Ergebnis von `issueLabelMerge`/`issueLabelDelete`.
#[derive(SimpleObject)]
pub struct IssueLabelBulkPayload {
    pub success: bool,
    /// Zusammenführen: das Ziel-Label.
    pub issue_label: Option<IssueLabelGql>,
    /// Anzahl der betroffenen Issues.
    pub affected_issues: i32,
}

#[derive(InputObject)]
pub struct IssueLabelCreateInput {
    pub name: String,
    pub color: Option<String>,
    pub description: Option<String>,
    pub team_id: Option<String>,
    pub parent_id: Option<String>,
    pub is_group: Option<bool>,
    pub group_type: Option<String>,
}

#[derive(InputObject)]
pub struct IssueLabelUpdateInput {
    pub name: Option<String>,
    pub color: Option<String>,
    pub description: MaybeUndefined<String>,
    pub parent_id: MaybeUndefined<String>,
    pub is_group: Option<bool>,
    pub group_type: Option<String>,
}

fn group_type(s: Option<String>) -> async_graphql::Result<Option<LabelGroupType>> {
    s.map(|s| {
        s.parse::<LabelGroupType>().map_err(|_| {
            gql_err(AppError::invalid_request(
                "groupType must be singleSelect or multiSelect",
            ))
        })
    })
    .transpose()
}

fn tri<T>(m: MaybeUndefined<T>) -> Option<Option<T>> {
    match m {
        MaybeUndefined::Undefined => None,
        MaybeUndefined::Null => Some(None),
        MaybeUndefined::Value(v) => Some(Some(v)),
    }
}

fn payload(v: LabelView) -> IssueLabelPayload {
    IssueLabelPayload {
        success: true,
        issue_label: Some(v.into()),
    }
}

#[derive(Default)]
pub struct LabelQuery;

#[Object]
impl LabelQuery {
    /// Anzahl der (für den Aufrufer sichtbaren) Issues mit diesem Label bzw. dieser Gruppe;
    /// setzt das Recht zum Verwalten des Labels voraus (Bestätigung vor Merge/Delete).
    async fn issue_label_usage(&self, ctx: &Context<'_>, id: String) -> async_graphql::Result<i32> {
        let n = app(ctx)?
            .issue_label_usage(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?;
        Ok(i32::try_from(n).unwrap_or(i32::MAX))
    }

    /// Sichtbare Issue-Labels (Workspace und Teams, optional für ein Team samt Vorfahren).
    async fn issue_labels(
        &self,
        ctx: &Context<'_>,
        team_id: Option<String>,
        name: Option<String>,
        include_groups: Option<bool>,
        include_archived: Option<bool>,
        first: Option<i32>,
    ) -> async_graphql::Result<Vec<IssueLabelGql>> {
        let page = app(ctx)?
            .list_labels(
                request_ctx(ctx)?,
                LabelKind::Issue,
                ListLabelsInput {
                    team: team_id,
                    name,
                    include_groups: include_groups.unwrap_or(true),
                    include_archived: include_archived.unwrap_or(false),
                    limit: i64::from(crate::error::GqlResultExt::gql(
                        crate::pagination::first_arg(first, 50, 250),
                        ctx,
                    )?),
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(page.items.into_iter().map(Into::into).collect())
    }
}

#[derive(Default)]
pub struct LabelMutation;

#[Object]
impl LabelMutation {
    async fn issue_label_create(
        &self,
        ctx: &Context<'_>,
        input: IssueLabelCreateInput,
    ) -> async_graphql::Result<IssueLabelPayload> {
        let v = app(ctx)?
            .save_label(
                request_ctx(ctx)?,
                LabelKind::Issue,
                SaveLabelInput {
                    name: Some(input.name),
                    color: input.color,
                    description: input.description.map(Some),
                    team_id: input.team_id,
                    parent: input.parent_id.map(Some),
                    is_group: input.is_group,
                    group_type: group_type(input.group_type)?,
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(payload(v))
    }

    async fn issue_label_update(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: IssueLabelUpdateInput,
    ) -> async_graphql::Result<IssueLabelPayload> {
        let v = app(ctx)?
            .save_label(
                request_ctx(ctx)?,
                LabelKind::Issue,
                SaveLabelInput {
                    id: Some(id),
                    name: input.name,
                    color: input.color,
                    description: tri(input.description),
                    parent: tri(input.parent_id),
                    is_group: input.is_group,
                    group_type: group_type(input.group_type)?,
                    ..Default::default()
                },
            )
            .await
            .map_err(gql_err)?;
        Ok(payload(v))
    }

    async fn issue_label_retire(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<IssueLabelPayload> {
        Ok(payload(
            app(ctx)?
                .retire_label(request_ctx(ctx)?, LabelKind::Issue, &id)
                .await
                .map_err(gql_err)?,
        ))
    }

    async fn issue_label_restore(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<IssueLabelPayload> {
        Ok(payload(
            app(ctx)?
                .restore_label(request_ctx(ctx)?, LabelKind::Issue, &id)
                .await
                .map_err(gql_err)?,
        ))
    }

    /// Führt das Label `id` in `targetId` zusammen und löscht `id`.
    async fn issue_label_merge(
        &self,
        ctx: &Context<'_>,
        id: String,
        target_id: String,
    ) -> async_graphql::Result<IssueLabelBulkPayload> {
        let r = app(ctx)?
            .merge_issue_label(request_ctx(ctx)?, &id, &target_id)
            .await
            .map_err(gql_err)?;
        Ok(IssueLabelBulkPayload {
            success: true,
            issue_label: r.target.map(Into::into),
            affected_issues: i32::try_from(r.affected_issues).unwrap_or(i32::MAX),
        })
    }

    /// Löscht das Label (Gruppen samt Kindern) endgültig und entfernt es von allen Issues.
    async fn issue_label_delete(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<IssueLabelBulkPayload> {
        let r = app(ctx)?
            .delete_issue_label(request_ctx(ctx)?, &id)
            .await
            .map_err(gql_err)?;
        Ok(IssueLabelBulkPayload {
            success: true,
            issue_label: None,
            affected_issues: i32::try_from(r.affected_issues).unwrap_or(i32::MAX),
        })
    }
}
