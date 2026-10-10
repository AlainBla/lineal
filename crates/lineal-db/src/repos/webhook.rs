//! `WebhookRepo` für `PgTx` (WP16): Webhooks, Zustellprotokoll, Entitäten für Payloads.
//! OAuth, Einladungen und Team-Lebenszyklus stehen als inhärente Methoden in `oauth.rs`,
//! `invitation.rs` und `team_lifecycle.rs`; dieser Block leitet nur weiter.

use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use lineal_app::AppResult;
use lineal_app::ports::repos::WebhookRepo;
use lineal_app::ports::repos::webhook::*;
use lineal_domain::enums::UserRole;
use lineal_domain::ids::{OrgId, TeamId, UserId, WebhookId};
use serde_json::{Map, Value, json};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct WebhookRow {
    id: Uuid,
    organization_id: Uuid,
    label: Option<String>,
    url: String,
    secret: String,
    previous_secret: Option<String>,
    secret_rotated_at: Option<DateTime<Utc>>,
    enabled: bool,
    team_id: Option<Uuid>,
    all_public_teams: bool,
    resource_types: Vec<String>,
    creator_id: Uuid,
    oauth_application_id: Option<Uuid>,
    failure_count: i32,
    disabled_reason: Option<String>,
    last_success_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

pub(crate) const WEBHOOK_COLS: &str = "id, organization_id, label, url, secret, previous_secret, \
    secret_rotated_at, enabled, team_id, all_public_teams, resource_types, creator_id, \
    oauth_application_id, failure_count, disabled_reason, last_success_at, created_at, updated_at";

impl From<WebhookRow> for WebhookRecord {
    fn from(r: WebhookRow) -> Self {
        WebhookRecord {
            id: WebhookId(r.id),
            org_id: OrgId(r.organization_id),
            label: r.label,
            url: r.url,
            secret_enc: r.secret,
            previous_secret_enc: r.previous_secret,
            secret_rotated_at: r.secret_rotated_at,
            enabled: r.enabled,
            team_id: r.team_id.map(TeamId),
            all_public_teams: r.all_public_teams,
            resource_types: r.resource_types,
            creator_id: UserId(r.creator_id),
            oauth_application_id: r.oauth_application_id,
            consecutive_failures: r.failure_count,
            disabled_reason: r.disabled_reason,
            last_success_at: r.last_success_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

fn snake_to_camel(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut up = false;
    for c in s.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Spaltennamen (snake_case) → camelCase; entfernt interne/sensible Spalten.
fn camelize(v: Value, drop: &[&str]) -> Value {
    let Value::Object(o) = v else {
        return v;
    };
    let mut out = Map::new();
    for (k, v) in o {
        if k == "organization_id" || k == "search_vector" || drop.contains(&k.as_str()) {
            continue;
        }
        out.insert(snake_to_camel(&k), v);
    }
    Value::Object(out)
}

fn priority_name(p: i64) -> &'static str {
    match p {
        1 => "Urgent",
        2 => "High",
        3 => "Medium",
        4 => "Low",
        _ => "No priority",
    }
}

type Pool<'a> = &'a mut sqlx::PgConnection;

async fn issue_teams(c: Pool<'_>, org: Uuid, issue: Uuid) -> AppResult<Vec<Uuid>> {
    sqlx::query_scalar("SELECT team_id FROM issue WHERE organization_id = $1 AND id = $2")
        .bind(org)
        .bind(issue)
        .fetch_all(c)
        .await
        .map_err(db_err)
}

async fn project_teams(c: Pool<'_>, org: Uuid, project: Uuid) -> AppResult<Vec<Uuid>> {
    sqlx::query_scalar(
        "SELECT team_id FROM project_team WHERE organization_id = $1 AND project_id = $2 ORDER BY sort_order, team_id",
    )
    .bind(org)
    .bind(project)
    .fetch_all(c)
    .await
    .map_err(db_err)
}

/// Eine Zeile als JSON (`to_jsonb`) aus `table` (nur feste, im Code stehende Tabellennamen).
async fn row_json(c: Pool<'_>, table: &str, org: Uuid, id: Uuid) -> AppResult<Option<Value>> {
    sqlx::query_scalar(&format!(
        "SELECT to_jsonb(t) FROM {table} t WHERE t.organization_id = $1 AND t.id = $2"
    ))
    .bind(org)
    .bind(id)
    .fetch_optional(c)
    .await
    .map_err(db_err)
}

async fn issue_entity(c: Pool<'_>, org: Uuid, id: Uuid) -> AppResult<Option<(Vec<Uuid>, Value, String)>> {
    let Some(base) = row_json(&mut *c, "issue", org, id).await? else {
        return Ok(None);
    };
    let team = base["team_id"].as_str().and_then(|s| Uuid::parse_str(s).ok());
    let state = base["state_id"].as_str().and_then(|s| Uuid::parse_str(s).ok());
    let team_obj: Option<Value> = match team {
        Some(t) => sqlx::query_as::<_, (Uuid, String, String)>(
            "SELECT id, key, name FROM team WHERE organization_id = $1 AND id = $2",
        )
        .bind(org)
        .bind(t)
        .fetch_optional(&mut *c)
        .await
        .map_err(db_err)?
        .map(|(id, key, name)| json!({ "id": id.to_string(), "key": key, "name": name })),
        None => None,
    };
    let state_obj: Option<Value> = match state {
        Some(s) => sqlx::query_as::<_, (Uuid, String, String)>(
            "SELECT id, name, type FROM workflow_state WHERE organization_id = $1 AND id = $2",
        )
        .bind(org)
        .bind(s)
        .fetch_optional(&mut *c)
        .await
        .map_err(db_err)?
        .map(|(id, name, t)| json!({ "id": id.to_string(), "name": name, "type": t })),
        None => None,
    };
    let labels: Vec<Uuid> = sqlx::query_scalar(
        "SELECT label_id FROM issue_to_label WHERE organization_id = $1 AND issue_id = $2 ORDER BY label_id",
    )
    .bind(org)
    .bind(id)
    .fetch_all(&mut *c)
    .await
    .map_err(db_err)?;
    let identifier = base["identifier"].as_str().unwrap_or_default().to_string();
    let priority = base["priority"].as_i64().unwrap_or(0);
    let mut data = camelize(
        base,
        &[
            "sort_order",
            "priority_sort_order",
            "sub_issue_sort_order",
            "source_comment_id",
            "last_applied_template_id",
            "snoozed_by_id",
        ],
    );
    if let Some(o) = data.as_object_mut() {
        o.insert(
            "priority".into(),
            json!({ "value": priority, "name": priority_name(priority) }),
        );
        o.insert("team".into(), team_obj.unwrap_or(Value::Null));
        o.insert("state".into(), state_obj.unwrap_or(Value::Null));
        o.insert(
            "labelIds".into(),
            Value::Array(labels.iter().map(|l| json!(l.to_string())).collect()),
        );
    }
    Ok(Some((
        team.into_iter().collect(),
        data,
        format!("issue/{identifier}"),
    )))
}

async fn comment_scope(c: Pool<'_>, org: Uuid, row: &Value) -> AppResult<(Vec<Uuid>, String)> {
    let str_id = |v: &Value, k: &str| v[k].as_str().and_then(|s| Uuid::parse_str(s).ok());
            let mut path = String::new();
            let teams = if let Some(i) = str_id(row, "issue_id") {
                if let Some(ident) = sqlx::query_scalar::<_, String>(
                    "SELECT identifier FROM issue WHERE organization_id = $1 AND id = $2",
                )
                .bind(org)
                .bind(i)
                .fetch_optional(&mut *c)
                .await
                .map_err(db_err)?
                {
                    path = format!("issue/{ident}");
                }
                issue_teams(&mut *c, org, i).await?
            } else {
                let project = match str_id(row, "project_id") {
                    Some(p) => Some(p),
                    None => match str_id(row, "project_update_id") {
                        Some(u) => sqlx::query_scalar(
                            "SELECT project_id FROM project_update WHERE organization_id = $1 AND id = $2",
                        )
                        .bind(org)
                        .bind(u)
                        .fetch_optional(&mut *c)
                        .await
                        .map_err(db_err)?,
                        None => match str_id(row, "project_milestone_id") {
                            Some(m) => sqlx::query_scalar(
                                "SELECT project_id FROM project_milestone WHERE organization_id = $1 AND id = $2",
                            )
                            .bind(org)
                            .bind(m)
                            .fetch_optional(&mut *c)
                            .await
                            .map_err(db_err)?,
                            None => None,
                        },
                    },
                };
                match project {
                    Some(p) => project_teams(&mut *c, org, p).await?,
                    None => match str_id(row, "document_id") {
                        Some(d) => document_teams(&mut *c, org, d).await?,
                        None => Vec::new(),
                    },
                }
            };
    Ok((teams, path))
}

async fn load_entity(
    c: Pool<'_>,
    org: Uuid,
    kind: &str,
    id: Uuid,
) -> AppResult<Option<(Vec<Uuid>, Value, String)>> {
    let str_id = |v: &Value, k: &str| v[k].as_str().and_then(|s| Uuid::parse_str(s).ok());
    Ok(match kind {
        "Issue" => issue_entity(c, org, id).await?,
        "IssueSLA" => {
            let Some((teams, issue, path)) = issue_entity(c, org, id).await? else {
                return Ok(None);
            };
            let pick = |k: &str| issue.get(k).cloned().unwrap_or(Value::Null);
            let data = json!({
                "id": id.to_string(),
                "issueId": id.to_string(),
                "slaType": pick("slaType"),
                "slaStartedAt": pick("slaStartedAt"),
                "slaMediumRiskAt": pick("slaMediumRiskAt"),
                "slaHighRiskAt": pick("slaHighRiskAt"),
                "slaBreachesAt": pick("slaBreachesAt"),
                "issueData": issue,
            });
            Some((teams, data, path))
        }
        "Comment" => {
            let Some(row) = row_json(&mut *c, "comment", org, id).await? else {
                return Ok(None);
            };
            let (teams, path) = comment_scope(&mut *c, org, &row).await?;
            Some((teams, camelize(row, &["body_data"]), path))
        }
        "IssueLabel" => {
            let Some(row) = row_json(&mut *c, "issue_label", org, id).await? else {
                return Ok(None);
            };
            let teams = str_id(&row, "team_id").into_iter().collect();
            Some((teams, camelize(row, &[]), String::new()))
        }
        "Reaction" => {
            let Some(row) = row_json(&mut *c, "reaction", org, id).await? else {
                return Ok(None);
            };
            let teams = if let Some(i) = str_id(&row, "issue_id") {
                issue_teams(&mut *c, org, i).await?
            } else if let Some(cm) = str_id(&row, "comment_id") {
                match row_json(&mut *c, "comment", org, cm).await? {
                    Some(crow) => comment_scope(&mut *c, org, &crow).await?.0,
                    None => Vec::new(),
                }
            } else if let Some(u) = str_id(&row, "project_update_id") {
                let p: Option<Uuid> = sqlx::query_scalar(
                    "SELECT project_id FROM project_update WHERE organization_id = $1 AND id = $2",
                )
                .bind(org)
                .bind(u)
                .fetch_optional(&mut *c)
                .await
                .map_err(db_err)?;
                match p {
                    Some(p) => project_teams(&mut *c, org, p).await?,
                    None => Vec::new(),
                }
            } else {
                Vec::new()
            };
            Some((teams, camelize(row, &[]), String::new()))
        }
        "Attachment" => {
            let Some(row) = row_json(&mut *c, "attachment", org, id).await? else {
                return Ok(None);
            };
            let issue = str_id(&row, "issue_id");
            let teams = match issue {
                Some(i) => issue_teams(&mut *c, org, i).await?,
                None => Vec::new(),
            };
            Some((teams, camelize(row, &["storage_key", "metadata"]), String::new()))
        }
        "Project" => {
            let Some(row) = row_json(&mut *c, "project", org, id).await? else {
                return Ok(None);
            };
            let slug = row["slug_id"].as_str().unwrap_or_default().to_string();
            let teams = project_teams(&mut *c, org, id).await?;
            let team_rows: Vec<(Uuid, String, String)> = sqlx::query_as(
                "SELECT t.id, t.key, t.name FROM project_team pt JOIN team t \
                 ON t.organization_id = pt.organization_id AND t.id = pt.team_id \
                 WHERE pt.organization_id = $1 AND pt.project_id = $2 ORDER BY pt.sort_order, t.id",
            )
            .bind(org)
            .bind(id)
            .fetch_all(&mut *c)
            .await
            .map_err(db_err)?;
            let mut data = camelize(
                row,
                &[
                    "scope_history",
                    "completed_scope_history",
                    "in_progress_scope_history",
                    "issue_count_history",
                    "completed_issue_count_history",
                    "sort_order",
                    "priority_sort_order",
                ],
            );
            if let Some(o) = data.as_object_mut() {
                o.insert(
                    "teamIds".into(),
                    Value::Array(teams.iter().map(|t| json!(t.to_string())).collect()),
                );
                o.insert(
                    "teams".into(),
                    Value::Array(
                        team_rows
                            .into_iter()
                            .map(|(id, key, name)| json!({ "id": id.to_string(), "key": key, "name": name }))
                            .collect(),
                    ),
                );
            }
            Some((teams, data, format!("project/{slug}")))
        }
        "ProjectUpdate" => {
            let Some(row) = row_json(&mut *c, "project_update", org, id).await? else {
                return Ok(None);
            };
            let teams = match str_id(&row, "project_id") {
                Some(p) => project_teams(&mut *c, org, p).await?,
                None => Vec::new(),
            };
            Some((teams, camelize(row, &["progress_snapshot"]), String::new()))
        }
        "Cycle" => {
            let Some(row) = row_json(&mut *c, "cycle", org, id).await? else {
                return Ok(None);
            };
            let teams = str_id(&row, "team_id").into_iter().collect();
            Some((
                teams,
                camelize(
                    row,
                    &[
                        "scope_history",
                        "completed_scope_history",
                        "in_progress_scope_history",
                        "issue_count_history",
                        "completed_issue_count_history",
                        "uncompleted_issue_ids_upon_close",
                    ],
                ),
                String::new(),
            ))
        }
        "Document" => {
            let Some(row) = row_json(&mut *c, "document", org, id).await? else {
                return Ok(None);
            };
            let slug = row["slug_id"].as_str().unwrap_or_default().to_string();
            let teams = document_teams(&mut *c, org, id).await?;
            Some((teams, camelize(row, &[]), format!("document/{slug}")))
        }
        "Initiative" => {
            let Some(row) = row_json(&mut *c, "initiative", org, id).await? else {
                return Ok(None);
            };
            let slug = row["slug_id"].as_str().unwrap_or_default().to_string();
            Some((
                Vec::new(),
                camelize(
                    row,
                    &[
                        "sort_order",
                        "priority_sort_order",
                        "scope_history",
                        "completed_scope_history",
                    ],
                ),
                format!("initiative/{slug}"),
            ))
        }
        "InitiativeUpdate" => {
            let Some(row) = row_json(&mut *c, "initiative_update", org, id).await? else {
                return Ok(None);
            };
            Some((Vec::new(), camelize(row, &["progress_snapshot"]), String::new()))
        }
        // Whitelist: nie E-Mail, Passwort-Hash oder persönliche Einstellungen.
        "User" => {
            let row: Option<Value> = sqlx::query_scalar(
                "SELECT jsonb_build_object('id', id, 'name', name, 'displayName', display_name, \
                 'role', role, 'active', active, 'status', status, 'avatarUrl', avatar_url, \
                 'createdAt', created_at, 'updatedAt', updated_at) \
                 FROM \"user\" WHERE organization_id = $1 AND id = $2",
            )
            .bind(org)
            .bind(id)
            .fetch_optional(&mut *c)
            .await
            .map_err(db_err)?;
            row.map(|d| (Vec::new(), d, String::new()))
        }
        _ => None,
    })
}

async fn document_teams(c: Pool<'_>, org: Uuid, doc: Uuid) -> AppResult<Vec<Uuid>> {
    let row: Option<(Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
        "SELECT team_id, issue_id, project_id, cycle_id FROM document WHERE organization_id = $1 AND id = $2",
    )
    .bind(org)
    .bind(doc)
    .fetch_optional(&mut *c)
    .await
    .map_err(db_err)?;
    let Some((team, issue, project, cycle)) = row else {
        return Ok(Vec::new());
    };
    if let Some(t) = team {
        return Ok(vec![t]);
    }
    if let Some(i) = issue {
        return issue_teams(&mut *c, org, i).await;
    }
    if let Some(p) = project {
        return project_teams(&mut *c, org, p).await;
    }
    if let Some(cy) = cycle {
        return sqlx::query_scalar("SELECT team_id FROM cycle WHERE organization_id = $1 AND id = $2")
            .bind(org)
            .bind(cy)
            .fetch_all(&mut *c)
            .await
            .map_err(db_err);
    }
    Ok(Vec::new())
}

#[async_trait]
impl WebhookRepo for PgTx {
    // --- Webhook-CRUD -----------------------------------------------------
    async fn insert_webhook(&mut self, w: &NewWebhook) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO webhook (id, organization_id, label, url, secret, enabled, team_id, all_public_teams, \
             resource_types, creator_id, oauth_application_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, true, $6, $7, $8, $9, $10, $11, $11)",
        )
        .bind(w.id.0)
        .bind(self.org.0)
        .bind(&w.label)
        .bind(&w.url)
        .bind(&w.secret_enc)
        .bind(w.team_id.map(|t| t.0))
        .bind(w.all_public_teams)
        .bind(&w.resource_types)
        .bind(w.creator_id.0)
        .bind(w.oauth_application_id)
        .bind(w.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_webhook(&mut self, id: WebhookId) -> AppResult<Option<WebhookRecord>> {
        let row = sqlx::query_as::<_, WebhookRow>(&format!(
            "SELECT {WEBHOOK_COLS} FROM webhook WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(WebhookRecord::from))
    }

    async fn list_webhooks(&mut self) -> AppResult<Vec<WebhookRecord>> {
        let rows = sqlx::query_as::<_, WebhookRow>(&format!(
            "SELECT {WEBHOOK_COLS} FROM webhook WHERE organization_id = $1 AND archived_at IS NULL \
             ORDER BY created_at DESC, id DESC"
        ))
        .bind(self.org.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(WebhookRecord::from).collect())
    }

    async fn update_webhook(
        &mut self,
        id: WebhookId,
        p: &WebhookPatch,
        now: DateTime<Utc>,
    ) -> AppResult<WebhookRecord> {
        let (new_secret, prev_secret) = match &p.rotate_secret {
            Some((n, o)) => (Some(n.clone()), Some(o.clone())),
            None => (None, None),
        };
        let (set_label, label) = match &p.label {
            Some(l) => (true, l.clone()),
            None => (false, None),
        };
        // Reaktivieren (enabled = true) setzt Fehlerzähler und Deaktivierungsgrund zurück.
        let row = sqlx::query_as::<_, WebhookRow>(&format!(
            "UPDATE webhook SET \
               label = CASE WHEN $3 THEN $4 ELSE label END, \
               url = COALESCE($5, url), \
               enabled = COALESCE($6, enabled), \
               resource_types = COALESCE($7, resource_types), \
               secret = COALESCE($8, secret), \
               previous_secret = CASE WHEN $8::text IS NOT NULL THEN $9 ELSE previous_secret END, \
               secret_rotated_at = CASE WHEN $8::text IS NOT NULL THEN $10 ELSE secret_rotated_at END, \
               failure_count = CASE WHEN $6 IS TRUE THEN 0 ELSE failure_count END, \
               disabled_reason = CASE WHEN $6 IS TRUE THEN NULL WHEN $6 IS FALSE THEN COALESCE(disabled_reason, 'manual') ELSE disabled_reason END, \
               version = version + 1, updated_at = $10 \
             WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL \
             RETURNING {WEBHOOK_COLS}"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .bind(set_label)
        .bind(label)
        .bind(&p.url)
        .bind(p.enabled)
        .bind(&p.resource_types)
        .bind(new_secret)
        .bind(prev_secret)
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(WebhookRecord::from).ok_or_else(|| {
            lineal_app::AppError::not_found("Could not find referenced Webhook.")
                .with_details(json!({ "type": "Webhook" }))
        })
    }

    async fn archive_webhook(&mut self, id: WebhookId, now: DateTime<Utc>) -> AppResult<bool> {
        let res = sqlx::query(
            "UPDATE webhook SET archived_at = $3, enabled = false, version = version + 1, updated_at = $3 \
             WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    // --- Zustellung -------------------------------------------------------
    async fn list_dispatch_webhooks(&mut self, resource_type: &str) -> AppResult<Vec<WebhookRecord>> {
        let rows = sqlx::query_as::<_, WebhookRow>(&format!(
            "SELECT {WEBHOOK_COLS} FROM webhook WHERE organization_id = $1 AND enabled AND archived_at IS NULL \
             AND $2 = ANY (resource_types) ORDER BY created_at, id"
        ))
        .bind(self.org.0)
        .bind(resource_type)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(WebhookRecord::from).collect())
    }

    async fn load_webhook_entity(
        &mut self,
        kind: &str,
        id: Uuid,
        hint_fields: &[String],
    ) -> AppResult<Option<WebhookEntity>> {
        let org = self.org.0;
        let Some((teams, data, url_path)) = load_entity(&mut self.tx, org, kind, id).await? else {
            return Ok(None);
        };
        let updated_from = if kind == "Issue" && !hint_fields.is_empty() {
            let changes: Option<Value> = sqlx::query_scalar(
                "SELECT changes FROM issue_history WHERE organization_id = $1 AND issue_id = $2 \
                 ORDER BY created_at DESC, id DESC LIMIT 1",
            )
            .bind(org)
            .bind(id)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?;
            let mut from = Map::new();
            for f in hint_fields {
                let old = changes
                    .as_ref()
                    .and_then(|c| c.get(f))
                    .and_then(|c| c.get("from"))
                    .cloned()
                    .unwrap_or(Value::Null);
                from.insert(f.clone(), old);
            }
            Some(Value::Object(from))
        } else {
            None
        };
        Ok(Some(WebhookEntity {
            team_ids: teams.into_iter().map(TeamId).collect(),
            data,
            updated_from,
            url_path,
        }))
    }

    async fn insert_delivery(&mut self, d: &NewDelivery) -> AppResult<bool> {
        let res = sqlx::query(
            "INSERT INTO webhook_delivery (id, organization_id, webhook_id, event_id, resource_type, action, \
             payload, next_attempt_at, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8, $8) \
             ON CONFLICT (webhook_id, event_id) DO NOTHING",
        )
        .bind(d.id)
        .bind(self.org.0)
        .bind(d.webhook_id.0)
        .bind(d.event_id)
        .bind(&d.resource_type)
        .bind(&d.action)
        .bind(&d.payload)
        .bind(d.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn claim_due_deliveries(
        &mut self,
        now: DateTime<Utc>,
        limit: i64,
        lease: Duration,
    ) -> AppResult<Vec<DueDelivery>> {
        // Zustellungen deaktivierter/gelöschter Webhooks werden nicht mehr versucht.
        sqlx::query(
            "UPDATE webhook_delivery d SET status = 'given_up', next_attempt_at = NULL, updated_at = $1 \
             FROM webhook w WHERE w.organization_id = d.organization_id AND w.id = d.webhook_id \
             AND d.status = 'pending' AND d.next_attempt_at <= $1 AND (NOT w.enabled OR w.archived_at IS NOT NULL)",
        )
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let rows: Vec<(Uuid, Uuid, Uuid, Uuid, String, String, i32, Value)> = sqlx::query_as(
            "WITH due AS ( \
               SELECT id FROM webhook_delivery WHERE status = 'pending' AND next_attempt_at <= $1 \
               ORDER BY next_attempt_at LIMIT $2 FOR UPDATE SKIP LOCKED) \
             UPDATE webhook_delivery d SET next_attempt_at = $3, updated_at = $1 \
             FROM due WHERE d.id = due.id \
             RETURNING d.id, d.organization_id, d.webhook_id, d.event_id, d.resource_type, d.action, d.attempt, d.payload",
        )
        .bind(now)
        .bind(limit)
        .bind(now + lease)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let mut out = Vec::with_capacity(rows.len());
        for (id, org, webhook, event, resource_type, action, attempt, payload) in rows {
            let w = sqlx::query_as::<_, WebhookRow>(&format!(
                "SELECT {WEBHOOK_COLS} FROM webhook WHERE organization_id = $1 AND id = $2"
            ))
            .bind(org)
            .bind(webhook)
            .fetch_optional(&mut *self.tx)
            .await
            .map_err(db_err)?;
            let Some(w) = w else { continue };
            out.push(DueDelivery {
                id,
                org_id: OrgId(org),
                event_id: event,
                resource_type,
                action,
                attempt,
                payload,
                webhook: w.into(),
            });
        }
        Ok(out)
    }

    async fn record_delivery_attempt(
        &mut self,
        delivery: Uuid,
        webhook: WebhookId,
        o: &DeliveryOutcome,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let org = self.org.0;
        let status = if o.success {
            "succeeded"
        } else if o.next_attempt_at.is_some() {
            "pending"
        } else {
            "given_up"
        };
        sqlx::query(
            "UPDATE webhook_delivery SET attempt = attempt + 1, status = $3, response_code = $4, duration_ms = $5, \
             error = $6, next_attempt_at = $7, updated_at = $8 WHERE organization_id = $1 AND id = $2",
        )
        .bind(org)
        .bind(delivery)
        .bind(status)
        .bind(o.response_code)
        .bind(o.duration_ms)
        .bind(o.error.as_deref().map(|e| e.chars().take(500).collect::<String>()))
        .bind(o.next_attempt_at)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if o.success {
            sqlx::query(
                "UPDATE webhook SET failure_count = 0, last_success_at = $3 \
                 WHERE organization_id = $1 AND id = $2",
            )
            .bind(org)
            .bind(webhook.0)
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
            return Ok(false);
        }
        if status == "pending" {
            return Ok(false);
        }
        // Aufgegeben: Fehlerzähler erhöhen; ab 5 in Folge ohne Erfolg in 24 h deaktivieren.
        let disabled: Option<bool> = sqlx::query_scalar(
            "UPDATE webhook SET failure_count = failure_count + 1, \
               enabled = CASE WHEN failure_count + 1 >= 5 AND (last_success_at IS NULL OR last_success_at < $3 - interval '24 hours') THEN false ELSE enabled END, \
               disabled_reason = CASE WHEN failure_count + 1 >= 5 AND (last_success_at IS NULL OR last_success_at < $3 - interval '24 hours') THEN 'delivery_failures' ELSE disabled_reason END, \
               updated_at = $3 \
             WHERE organization_id = $1 AND id = $2 \
             RETURNING (NOT enabled AND disabled_reason = 'delivery_failures')",
        )
        .bind(org)
        .bind(webhook.0)
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(disabled.unwrap_or(false))
    }

    async fn list_deliveries(&mut self, webhook: WebhookId, limit: i64) -> AppResult<Vec<DeliveryRecord>> {
        let rows: Vec<(Uuid, Uuid, Uuid, String, String, i32, String, Option<i32>, Option<i32>, Option<DateTime<Utc>>, DateTime<Utc>)> =
            sqlx::query_as(
                "SELECT id, webhook_id, event_id, resource_type, action, attempt, status, response_code, duration_ms, \
                 next_attempt_at, created_at FROM webhook_delivery \
                 WHERE organization_id = $1 AND webhook_id = $2 ORDER BY created_at DESC, id DESC LIMIT $3",
            )
            .bind(self.org.0)
            .bind(webhook.0)
            .bind(limit.clamp(1, 100))
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(id, w, e, rt, a, attempt, status, code, dur, next, created)| DeliveryRecord {
                id,
                webhook_id: WebhookId(w),
                event_id: e,
                resource_type: rt,
                action: a,
                attempt,
                status,
                response_code: code,
                duration_ms: dur,
                next_attempt_at: next,
                created_at: created,
            })
            .collect())
    }

    async fn requeue_delivery(&mut self, id: Uuid, now: DateTime<Utc>) -> AppResult<bool> {
        let res = sqlx::query(
            "UPDATE webhook_delivery SET status = 'pending', attempt = 0, next_attempt_at = $3, updated_at = $3 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn purge_deliveries(&mut self, before: DateTime<Utc>) -> AppResult<u64> {
        let res = sqlx::query("DELETE FROM webhook_delivery WHERE created_at < $1 AND status <> 'pending'")
            .bind(before)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(res.rows_affected())
    }

    async fn disable_webhooks_of_application(&mut self, application: Uuid, now: DateTime<Utc>) -> AppResult<u64> {
        let res = sqlx::query(
            "UPDATE webhook SET enabled = false, disabled_reason = 'oauth_revoked', updated_at = $3 \
             WHERE organization_id = $1 AND oauth_application_id = $2 AND enabled",
        )
        .bind(self.org.0)
        .bind(application)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected())
    }

    // --- OAuth ------------------------------------------------------------
    async fn oauth_register_client(&mut self, c: &ClientRegistration, now: DateTime<Utc>) -> AppResult<()> {
        self.oauth_register_client_impl(c, now).await
    }
    async fn oauth_find_client(&mut self, client_id: &str) -> AppResult<Option<ClientRegistration>> {
        self.oauth_find_client_impl(client_id).await
    }
    async fn oauth_ensure_application(
        &mut self,
        client: &ClientRegistration,
        creator: UserId,
        now: DateTime<Utc>,
    ) -> AppResult<Uuid> {
        self.oauth_ensure_application_impl(client, creator, now).await
    }
    async fn oauth_insert_code(&mut self, code: &NewAuthCode) -> AppResult<()> {
        self.oauth_insert_code_impl(code).await
    }
    async fn oauth_find_code(&mut self, code_hash: &str) -> AppResult<Option<AuthCodeRecord>> {
        self.oauth_find_code_impl(code_hash).await
    }
    async fn oauth_consume_code(&mut self, code_hash: &str, now: DateTime<Utc>) -> AppResult<bool> {
        self.oauth_consume_code_impl(code_hash, now).await
    }
    async fn oauth_insert_token(&mut self, t: &NewOauthToken) -> AppResult<()> {
        self.oauth_insert_token_impl(t).await
    }
    async fn oauth_revoke_family(&mut self, family: Uuid, now: DateTime<Utc>) -> AppResult<u64> {
        self.oauth_revoke_family_impl(family, now).await
    }
    async fn oauth_find_by_refresh_hash(&mut self, hash: &str) -> AppResult<Option<OauthTokenRecord>> {
        self.oauth_find_by_refresh_hash_impl(hash).await
    }
    async fn oauth_rotate_token(&mut self, id: Uuid, expected_current: &str, r: &TokenRotation) -> AppResult<bool> {
        self.oauth_rotate_token_impl(id, expected_current, r).await
    }
    async fn oauth_revoke_by_token_hash(
        &mut self,
        hash: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<(OrgId, String)>> {
        self.oauth_revoke_by_token_hash_impl(hash, now).await
    }

    // --- Einladungen ------------------------------------------------------
    async fn insert_invitation(&mut self, i: &NewInvitation) -> AppResult<()> {
        self.insert_invitation_impl(i).await
    }
    async fn get_invitation(&mut self, id: Uuid) -> AppResult<Option<InvitationRecord>> {
        self.get_invitation_impl(id).await
    }
    async fn list_invitations(&mut self, pending_only: bool) -> AppResult<Vec<InvitationRecord>> {
        self.list_invitations_impl(pending_only).await
    }
    async fn find_pending_invitation(&mut self, email: &str, now: DateTime<Utc>) -> AppResult<Option<InvitationRecord>> {
        self.find_pending_invitation_impl(email, now).await
    }
    async fn find_invitation_by_token_hash(&mut self, hash: &str) -> AppResult<Option<InvitationRecord>> {
        self.find_invitation_by_token_hash_impl(hash).await
    }
    async fn renew_invitation(
        &mut self,
        id: Uuid,
        token_hash: &str,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        self.renew_invitation_impl(id, token_hash, expires_at, now).await
    }
    async fn set_invitation_status(
        &mut self,
        id: Uuid,
        status: &str,
        accepted_user: Option<UserId>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        self.set_invitation_status_impl(id, status, accepted_user, now).await
    }
    async fn activate_imported_user(
        &mut self,
        user: UserId,
        name: &str,
        password_hash: Option<&str>,
        role: UserRole,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        self.activate_imported_user_impl(user, name, password_hash, role, now).await
    }
    async fn password_hash_of_account(&mut self, user: UserId) -> AppResult<Option<String>> {
        self.password_hash_of_account_impl(user).await
    }

    // --- Team-Lebenszyklus ------------------------------------------------
    async fn set_team_retired(
        &mut self,
        team: TeamId,
        retired_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        self.set_team_retired_impl(team, retired_at, now).await
    }
    async fn set_team_trashed(&mut self, team: TeamId, trashed: bool, now: DateTime<Utc>) -> AppResult<()> {
        self.set_team_trashed_impl(team, trashed, now).await
    }
    async fn trash_sole_projects(&mut self, team: TeamId, now: DateTime<Utc>) -> AppResult<Vec<Uuid>> {
        self.trash_sole_projects_impl(team, now).await
    }
    async fn restore_sole_projects(
        &mut self,
        team: TeamId,
        trashed_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<Uuid>> {
        self.restore_sole_projects_impl(team, trashed_at, now).await
    }
    async fn shared_team_projects(&mut self, team: TeamId) -> AppResult<Vec<(Uuid, TeamId)>> {
        self.shared_team_projects_impl(team).await
    }
    async fn project_issue_counts(&mut self, project: Uuid) -> AppResult<Vec<(TeamId, i64)>> {
        self.project_issue_counts_impl(project).await
    }
    async fn list_teams_due_for_purge(&mut self, before: DateTime<Utc>) -> AppResult<Vec<(OrgId, TeamId)>> {
        self.list_teams_due_for_purge_impl(before).await
    }
    async fn purge_team(&mut self, team: TeamId) -> AppResult<()> {
        self.purge_team_impl(team).await
    }
}
