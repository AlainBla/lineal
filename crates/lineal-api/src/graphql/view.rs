//! GraphQL-Teilobjekte des Bereichs `view` (WP11, docs/05 §2–4): Custom Views, Favoriten,
//! Anzeigeoptionen und `viewIssues` (Issues über Custom View und/oder Filter-JSON).
//!
//! Filter reisen als `JSON` (der Filterbaum aus docs/05 §1.2) und werden serverseitig strikt
//! validiert; Sichtbarkeit und Rechte prüft der Use-Case (unsichtbare Views → `not_found`).

use super::issue::{IssueConnection, IssueGql};
use crate::context::{app, request_ctx};
use crate::error::GqlResultExt;
use async_graphql::types::Json;
use async_graphql::{Context, InputObject, Object, SimpleObject};
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::user::{OrderBy, PageCursor, PageRequest};
use lineal_app::ports::repos::view::{Favorite, ViewModel, ViewSubscription};
use lineal_app::usecases::issue::ListIssuesInput;
use lineal_app::usecases::view::{
    CreateFavoriteInput, CreateViewInput, ListViewsInput, UpdateViewInput, ViewInfo, ViewKeyInput,
    ViewPreferencesView,
};
use lineal_app::{AppError, AppResult};
use serde_json::Value;
use uuid::Uuid;

/// `CustomView`.
#[derive(SimpleObject)]
#[graphql(name = "CustomView")]
pub struct CustomViewGql {
    pub id: String,
    pub slug_id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub model_name: String,
    /// Normalisierter Filterbaum (`filterData`/`projectFilterData`/`initiativeFilterData`).
    pub filter_data: Json<Value>,
    pub owner_id: String,
    pub owner_name: Option<String>,
    pub shared: bool,
    pub team_id: Option<String>,
    pub team_key: Option<String>,
    pub project_id: Option<String>,
    pub initiative_id: Option<String>,
    pub sort_order: f64,
    pub version: i32,
    /// Darf der Betrachter bearbeiten oder löschen?
    pub can_edit: bool,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}

impl From<ViewInfo> for CustomViewGql {
    fn from(i: ViewInfo) -> Self {
        let v = i.view;
        Self {
            id: v.id.to_string(),
            slug_id: v.slug_id,
            name: v.name,
            description: v.description,
            icon: v.icon,
            color: v.color,
            model_name: v.model.as_str().into(),
            filter_data: Json(v.filter),
            owner_id: v.owner_id.to_string(),
            owner_name: i.owner_name,
            shared: v.shared,
            team_id: v.team_id.map(|t| t.to_string()),
            team_key: i.team.map(|(k, _)| k),
            project_id: v.project_id.map(|p| p.to_string()),
            initiative_id: v.initiative_id.map(|p| p.to_string()),
            sort_order: v.sort_order,
            version: v.version,
            can_edit: i.can_edit,
            created_at: v.created_at.to_rfc3339(),
            updated_at: v.updated_at.to_rfc3339(),
            archived_at: v.archived_at.map(|t| t.to_rfc3339()),
        }
    }
}

#[derive(SimpleObject)]
pub struct CustomViewConnection {
    pub nodes: Vec<CustomViewGql>,
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

#[derive(SimpleObject)]
pub struct CustomViewPayload {
    pub success: bool,
    pub custom_view: Option<CustomViewGql>,
}

#[derive(SimpleObject)]
#[graphql(name = "CustomViewDeletePayload")]
pub struct CustomViewDeletePayload {
    pub success: bool,
    pub entity_id: String,
}

#[derive(InputObject)]
pub struct CustomViewCreateInput {
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    /// `Issue`, `Project` oder `Initiative`.
    pub model_name: String,
    pub filter_data: Json<Value>,
    pub shared: Option<bool>,
    pub team_id: Option<String>,
    pub project_id: Option<String>,
    pub initiative_id: Option<String>,
    /// Anzeigeoptionen zum Zeitpunkt des Anlegens.
    pub preferences: Option<Json<Value>>,
}

#[derive(InputObject)]
pub struct CustomViewUpdateInput {
    pub expected_version: Option<i32>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub filter_data: Option<Json<Value>>,
    pub shared: Option<bool>,
    /// Team-Scope setzen (`null` lässt das Team unverändert; zum Entfernen `clearTeam`).
    pub team_id: Option<String>,
    pub clear_team: Option<bool>,
    pub owner_id: Option<String>,
    pub sort_order: Option<f64>,
    pub archived: Option<bool>,
}

/// `Favorite`.
#[derive(SimpleObject)]
#[graphql(name = "Favorite")]
pub struct FavoriteGql {
    pub id: String,
    #[graphql(name = "type")]
    pub kind: String,
    pub parent_id: Option<String>,
    pub folder_name: Option<String>,
    pub sort_order: f64,
    pub target_id: Option<String>,
    pub predefined_view_type: Option<String>,
    pub predefined_view_team_id: Option<String>,
}

impl From<Favorite> for FavoriteGql {
    fn from(f: Favorite) -> Self {
        Self {
            id: f.id.to_string(),
            kind: f.kind,
            parent_id: f.parent_id.map(|p| p.to_string()),
            folder_name: f.folder_name,
            sort_order: f.sort_order,
            target_id: f.target_id.map(|t| t.to_string()),
            predefined_view_type: f.predefined_view_type,
            predefined_view_team_id: f.predefined_view_team_id.map(|t| t.to_string()),
        }
    }
}

#[derive(SimpleObject)]
pub struct FavoritePayload {
    pub success: bool,
    pub favorite: Option<FavoriteGql>,
}

#[derive(SimpleObject)]
#[graphql(name = "FavoriteDeletePayload")]
pub struct FavoriteDeletePayload {
    pub success: bool,
    pub entity_id: String,
}

#[derive(InputObject)]
pub struct FavoriteCreateInput {
    #[graphql(name = "type")]
    pub kind: String,
    pub target_id: Option<String>,
    pub folder_name: Option<String>,
    pub parent_id: Option<String>,
    pub predefined_view_type: Option<String>,
    pub predefined_view_team_id: Option<String>,
}

/// Abo auf eine Custom View (docs/05 §4.7).
#[derive(SimpleObject)]
#[graphql(name = "ViewSubscription")]
pub struct ViewSubscriptionGql {
    pub id: String,
    pub view_id: String,
    /// `issueAddedToView`, `issueStatusChanged`, `issueStatusChangedAll`.
    pub notification_subscription_types: Vec<String>,
    pub active: bool,
}

impl From<ViewSubscription> for ViewSubscriptionGql {
    fn from(s: ViewSubscription) -> Self {
        Self {
            id: s.id.to_string(),
            view_id: s.view_id.0.to_string(),
            notification_subscription_types: s.types,
            active: s.active,
        }
    }
}

/// Wirksame Anzeigeoptionen.
#[derive(SimpleObject)]
#[graphql(name = "ViewPreferencesValues")]
pub struct ViewPreferencesGql {
    pub values: Json<Value>,
    pub has_personal: bool,
    pub has_default: bool,
}

impl From<ViewPreferencesView> for ViewPreferencesGql {
    fn from(v: ViewPreferencesView) -> Self {
        Self {
            values: Json(Value::Object(v.effective)),
            has_personal: v.has_personal,
            has_default: v.has_default,
        }
    }
}

fn parse_uuid(s: &str, field: &str) -> AppResult<Uuid> {
    Uuid::parse_str(s.trim()).map_err(|_| {
        AppError::invalid_request(format!("{field} must be a UUID"))
            .with_details(serde_json::json!({ "field": field }))
    })
}

/// Cursor `rfc3339|uuid` wie bei `issues`.
pub(crate) fn decode_cursor(after: Option<&str>) -> AppResult<Option<PageCursor>> {
    after
        .map(|c| -> AppResult<PageCursor> {
            let bad = || AppError::invalid_request("invalid cursor");
            let (k, i) = c.split_once('|').ok_or_else(bad)?;
            Ok(PageCursor {
                key: DateTime::parse_from_rfc3339(k)
                    .map_err(|_| bad())?
                    .with_timezone(&Utc),
                id: i.parse().map_err(|_| bad())?,
            })
        })
        .transpose()
}

pub(crate) fn encode_cursor(c: &PageCursor) -> String {
    format!(
        "{}|{}",
        c.key.to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        c.id
    )
}

fn model(s: &str) -> AppResult<ViewModel> {
    ViewModel::parse(s)
        .ok_or_else(|| AppError::invalid_request("modelName must be Issue, Project or Initiative"))
}

fn key(
    custom_view_id: Option<String>,
    view_type: Option<String>,
    team_id: Option<String>,
) -> ViewKeyInput {
    ViewKeyInput {
        custom_view: custom_view_id,
        view_type,
        team: team_id,
    }
}

#[derive(Default)]
pub struct ViewQuery;

#[Object]
impl ViewQuery {
    /// Sichtbare Custom Views (ohne archivierte und ohne Projekt-/Initiativ-Tabs).
    #[allow(clippy::too_many_arguments)]
    async fn custom_views(
        &self,
        ctx: &Context<'_>,
        model_name: Option<String>,
        team_id: Option<String>,
        shared: Option<bool>,
        query: Option<String>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<CustomViewConnection> {
        let cursor = decode_cursor(after.as_deref()).gql(ctx)?;
        let model = model_name.as_deref().map(model).transpose().gql(ctx)?;
        let page = app(ctx)?
            .list_views(
                request_ctx(ctx)?,
                ListViewsInput {
                    model,
                    team: team_id,
                    shared,
                    query,
                    page: PageRequest {
                        limit: i64::from(first.unwrap_or(50)),
                        cursor,
                        order_by: OrderBy::UpdatedAt,
                    },
                },
            )
            .await
            .gql(ctx)?;
        Ok(CustomViewConnection {
            has_next_page: page.next.is_some(),
            end_cursor: page.next.as_ref().map(encode_cursor),
            nodes: page.items.into_iter().map(Into::into).collect(),
        })
    }

    /// Eine sichtbare View (UUID, Slug, URL oder exakter Name); sonst `not_found`.
    async fn custom_view(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<CustomViewGql> {
        Ok(app(ctx)?
            .get_view(request_ctx(ctx)?, &id)
            .await
            .gql(ctx)?
            .into())
    }

    /// Eigenes Abo einer sichtbaren View (`null`, solange keines besteht).
    async fn view_subscription(
        &self,
        ctx: &Context<'_>,
        custom_view_id: String,
    ) -> async_graphql::Result<Option<ViewSubscriptionGql>> {
        Ok(app(ctx)?
            .view_subscription(request_ctx(ctx)?, &custom_view_id)
            .await
            .gql(ctx)?
            .map(Into::into))
    }

    /// Favoriten des Viewers (nur sichtbare Ziele).
    async fn favorites(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<FavoriteGql>> {
        Ok(app(ctx)?
            .list_favorites(request_ctx(ctx)?)
            .await
            .gql(ctx)?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Wirksame Anzeigeoptionen einer Custom View oder vordefinierten Ansicht.
    async fn view_preferences(
        &self,
        ctx: &Context<'_>,
        custom_view_id: Option<String>,
        view_type: Option<String>,
        team_id: Option<String>,
    ) -> async_graphql::Result<ViewPreferencesGql> {
        Ok(app(ctx)?
            .view_preferences(request_ctx(ctx)?, key(custom_view_id, view_type, team_id))
            .await
            .gql(ctx)?
            .into())
    }

    /// Issues über Custom View und/oder Filter: View UND Filter UND Suchbegriff-Regeln
    /// (docs/05 §4.3); Sortierung nach `updatedAt`, nicht aus der View.
    #[allow(clippy::too_many_arguments)]
    async fn view_issues(
        &self,
        ctx: &Context<'_>,
        custom_view_id: Option<String>,
        filter: Option<Json<Value>>,
        query: Option<String>,
        team_id: Option<String>,
        include_sub_teams: Option<bool>,
        include_archived: Option<bool>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<IssueConnection> {
        let cursor = decode_cursor(after.as_deref()).gql(ctx)?;
        let page = app(ctx)?
            .list_issues(
                request_ctx(ctx)?,
                ListIssuesInput {
                    custom_view: custom_view_id,
                    query,
                    filter: filter.map(|f| f.0),
                    team: team_id,
                    include_sub_teams: include_sub_teams.unwrap_or(false),
                    include_archived: include_archived.unwrap_or(false),
                    order_by: OrderBy::UpdatedAt,
                    limit: i64::from(first.unwrap_or(50)),
                    cursor,
                    ..Default::default()
                },
            )
            .await
            .gql(ctx)?;
        Ok(IssueConnection {
            has_next_page: page.next.is_some(),
            end_cursor: page.next.as_ref().map(encode_cursor),
            nodes: page.items.into_iter().map(IssueGql::from).collect(),
        })
    }
}

#[derive(Default)]
pub struct ViewMutation;

#[Object]
impl ViewMutation {
    async fn custom_view_create(
        &self,
        ctx: &Context<'_>,
        input: CustomViewCreateInput,
    ) -> async_graphql::Result<CustomViewPayload> {
        let m = model(&input.model_name).gql(ctx)?;
        let v = app(ctx)?
            .create_view(
                request_ctx(ctx)?,
                CreateViewInput {
                    name: input.name,
                    description: input.description,
                    icon: input.icon,
                    color: input.color,
                    model: m,
                    filter: input.filter_data.0,
                    shared: input.shared.unwrap_or(false),
                    team: input.team_id,
                    project: input.project_id,
                    initiative: input.initiative_id,
                    preferences: input.preferences.map(|p| p.0),
                },
            )
            .await
            .gql(ctx)?;
        Ok(CustomViewPayload {
            success: true,
            custom_view: Some(v.into()),
        })
    }

    async fn custom_view_update(
        &self,
        ctx: &Context<'_>,
        id: String,
        input: CustomViewUpdateInput,
    ) -> async_graphql::Result<CustomViewPayload> {
        let team = if input.clear_team == Some(true) {
            Some(None)
        } else {
            input.team_id.map(Some)
        };
        let v = app(ctx)?
            .update_view(
                request_ctx(ctx)?,
                UpdateViewInput {
                    reference: id,
                    expected_version: input.expected_version,
                    name: input.name,
                    description: input.description.map(Some),
                    icon: input.icon.map(Some),
                    color: input.color.map(Some),
                    filter: input.filter_data.map(|f| f.0),
                    shared: input.shared,
                    team,
                    owner: input.owner_id,
                    sort_order: input.sort_order,
                    archived: input.archived,
                },
            )
            .await
            .gql(ctx)?;
        Ok(CustomViewPayload {
            success: true,
            custom_view: Some(v.into()),
        })
    }

    async fn custom_view_delete(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<CustomViewDeletePayload> {
        let id = app(ctx)?
            .delete_view(request_ctx(ctx)?, &id)
            .await
            .gql(ctx)?;
        Ok(CustomViewDeletePayload {
            success: true,
            entity_id: id.to_string(),
        })
    }

    /// Kopie für den Aufrufer (`shared = false`, Name „… (copy)“).
    async fn custom_view_duplicate(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<CustomViewPayload> {
        let v = app(ctx)?
            .duplicate_view(request_ctx(ctx)?, &id)
            .await
            .gql(ctx)?;
        Ok(CustomViewPayload {
            success: true,
            custom_view: Some(v.into()),
        })
    }

    async fn favorite_create(
        &self,
        ctx: &Context<'_>,
        input: FavoriteCreateInput,
    ) -> async_graphql::Result<FavoritePayload> {
        let parent = input
            .parent_id
            .as_deref()
            .map(|p| parse_uuid(p, "parentId"))
            .transpose()
            .gql(ctx)?;
        let f = app(ctx)?
            .create_favorite(
                request_ctx(ctx)?,
                CreateFavoriteInput {
                    kind: input.kind,
                    target: input.target_id,
                    folder_name: input.folder_name,
                    parent,
                    predefined_view_type: input.predefined_view_type,
                    predefined_view_team: input.predefined_view_team_id,
                },
            )
            .await
            .gql(ctx)?;
        Ok(FavoritePayload {
            success: true,
            favorite: Some(f.into()),
        })
    }

    /// Verschiebt einen eigenen Favoriten (Reihenfolge per Drag & Drop): `sortOrder` ist die
    /// Mittelwert-Position, `parentId` der Ordner (leer = oberste Ebene).
    async fn favorite_update(
        &self,
        ctx: &Context<'_>,
        id: String,
        sort_order: f64,
        parent_id: Option<String>,
    ) -> async_graphql::Result<FavoritePayload> {
        let uuid = parse_uuid(&id, "id").gql(ctx)?;
        let parent = parent_id
            .as_deref()
            .map(|p| parse_uuid(p, "parentId"))
            .transpose()
            .gql(ctx)?;
        let f = app(ctx)?
            .update_favorite(request_ctx(ctx)?, uuid, parent, sort_order)
            .await
            .gql(ctx)?;
        Ok(FavoritePayload {
            success: true,
            favorite: Some(f.into()),
        })
    }

    /// Abonniert eine sichtbare Issue-View oder ändert das Abo (Default `issueAddedToView`).
    async fn view_subscription_update(
        &self,
        ctx: &Context<'_>,
        custom_view_id: String,
        active: bool,
        notification_subscription_types: Option<Vec<String>>,
    ) -> async_graphql::Result<ViewSubscriptionGql> {
        Ok(app(ctx)?
            .set_view_subscription(
                request_ctx(ctx)?,
                &custom_view_id,
                active,
                notification_subscription_types,
            )
            .await
            .gql(ctx)?
            .into())
    }

    async fn favorite_delete(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> async_graphql::Result<FavoriteDeletePayload> {
        let uuid = parse_uuid(&id, "id").gql(ctx)?;
        app(ctx)?
            .delete_favorite(request_ctx(ctx)?, uuid)
            .await
            .gql(ctx)?;
        Ok(FavoriteDeletePayload {
            success: true,
            entity_id: id,
        })
    }

    /// Persönliche Änderung der Anzeigeoptionen (nur die genannten Schlüssel).
    async fn view_preferences_update(
        &self,
        ctx: &Context<'_>,
        custom_view_id: Option<String>,
        view_type: Option<String>,
        team_id: Option<String>,
        preferences: Json<Value>,
    ) -> async_graphql::Result<ViewPreferencesGql> {
        Ok(app(ctx)?
            .update_view_preferences(
                request_ctx(ctx)?,
                key(custom_view_id, view_type, team_id),
                preferences.0,
            )
            .await
            .gql(ctx)?
            .into())
    }

    /// „Set default for everyone“ (Team-Owner, View-Owner bzw. Admins).
    async fn view_preferences_set_default(
        &self,
        ctx: &Context<'_>,
        custom_view_id: Option<String>,
        view_type: Option<String>,
        team_id: Option<String>,
    ) -> async_graphql::Result<ViewPreferencesGql> {
        Ok(app(ctx)?
            .set_view_preferences_default(
                request_ctx(ctx)?,
                key(custom_view_id, view_type, team_id),
            )
            .await
            .gql(ctx)?
            .into())
    }

    /// „Reset“: löscht die persönlichen Overrides.
    async fn view_preferences_reset(
        &self,
        ctx: &Context<'_>,
        custom_view_id: Option<String>,
        view_type: Option<String>,
        team_id: Option<String>,
    ) -> async_graphql::Result<ViewPreferencesGql> {
        Ok(app(ctx)?
            .reset_view_preferences(request_ctx(ctx)?, key(custom_view_id, view_type, team_id))
            .await
            .gql(ctx)?
            .into())
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
