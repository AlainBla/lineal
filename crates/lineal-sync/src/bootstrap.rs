//! `GET /sync/bootstrap` (docs/09 §5.2): Erstbefüllung des Client-Caches in Paketen.
//!
//! Antwort: zeilenweises JSON (`application/x-ndjson`). Jede Zeile ist ein Objekt in der Form der
//! Sync-`data` plus `__class` (Modellname) und `syncGroups`; die letzte Zeile ist
//! `_metadata_={"lastSyncId":N,"subscribedSyncGroups":[…],"databaseVersion":1,"nextCursor":…}`.
//!
//! **Pakete.** `limit` (Default 1000, höchstens 2000) begrenzt die Zeilen je Antwort. Ist
//! `nextCursor` gesetzt, ruft der Client mit `?cursor=<nextCursor>` das nächste Paket ab (Modelle
//! und `lastSyncId` stecken im Cursor). `lastSyncId` ist der Stand des Logs **vor** dem ersten
//! Lesen: Der Client öffnet danach `GET /sync?since=<lastSyncId>` und spielt Änderungen, die
//! während des Ladens passierten, über den Strom nach (Aktionen sind vollständige Zustände, das
//! Nachspielen ist idempotent).
//!
//! **Rechte.** Es gilt derselbe Gruppenfilter wie im Strom ([`super::allowed_groups`], mit dem
//! Policy-Snapshot **dieser** Anfrage): Ein Objekt wird nur geliefert, wenn sich seine Sync-Gruppen
//! mit den Gruppen des Users überschneiden. Gäste erhalten damit nur ihre Teams, ihre Freigaben,
//! Workspace-Labels und die Personen ihrer Teams. Zurückgeliefert werden nur die Gruppen des
//! Empfängers.
//!
//! **Bestand.** Papierkorb und archivierte Issues gehören nicht dazu; Kommentare, Anhänge und
//! Verlauf werden wie bisher bei Bedarf über GraphQL geladen.

use super::{
    Shared, allowed_groups, app_error_response, authenticate, error_response, origin_allowed,
};
use axum::extract::{Query, State};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use lineal_app::Actor;
use lineal_app::policy::PolicySnapshot;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

/// Modelle des Bootstraps in Lieferreihenfolge (Stammdaten vor Issues).
pub const DEFAULT_MODELS: [&str; 11] = [
    "Organization",
    "Team",
    "User",
    "WorkflowState",
    "IssueLabel",
    "Cycle",
    "Project",
    "ProjectMilestone",
    "Initiative",
    "CustomView",
    "Issue",
];
/// Zusätzlich auf Anfrage lieferbar.
const OPTIONAL_MODELS: [&str; 2] = ["Template", "Document"];
/// Version der Datenform; ändert sich sie, verwirft der Client seinen lokalen Cache.
pub const DATABASE_VERSION: u32 = 1;
const DEFAULT_LIMIT: i64 = 1000;
const MAX_LIMIT: i64 = 2000;
const MAX_CURSOR_LEN: usize = 512;

#[derive(Deserialize)]
pub(crate) struct BootstrapQuery {
    #[serde(rename = "modelNames")]
    model_names: Option<String>,
    limit: Option<i64>,
    cursor: Option<String>,
}

/// Position innerhalb des Bootstraps.
#[derive(Debug, PartialEq)]
struct Cursor {
    head: i64,
    models: Vec<String>,
    /// Index in `models`.
    idx: usize,
    after: Option<Uuid>,
}

impl Cursor {
    /// `<head>;<idx>;<after|->;<Modell,Modell,…>`
    fn encode(&self) -> String {
        format!(
            "{};{};{};{}",
            self.head,
            self.idx,
            self.after.map_or_else(|| "-".to_owned(), |u| u.to_string()),
            self.models.join(",")
        )
    }

    fn parse(raw: &str) -> Option<Self> {
        if raw.len() > MAX_CURSOR_LEN {
            return None;
        }
        let mut parts = raw.splitn(4, ';');
        let head: i64 = parts.next()?.parse().ok()?;
        let idx: usize = parts.next()?.parse().ok()?;
        let after = match parts.next()? {
            "-" => None,
            s => Some(Uuid::parse_str(s).ok()?),
        };
        let models = parse_models(parts.next()?)?;
        (head >= 0 && idx < models.len()).then_some(Self {
            head,
            models,
            idx,
            after,
        })
    }
}

/// Prüft eine Modellliste (`A,B,C`): nur bekannte Modelle, ohne Duplikate.
fn parse_models(list: &str) -> Option<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        if !DEFAULT_MODELS.contains(&name) && !OPTIONAL_MODELS.contains(&name) {
            return None;
        }
        if out.iter().any(|m| m == name) {
            return None;
        }
        out.push(name.to_owned());
    }
    (!out.is_empty() && out.len() <= DEFAULT_MODELS.len() + OPTIONAL_MODELS.len()).then_some(out)
}

pub(crate) async fn bootstrap(
    State(shared): State<Arc<Shared>>,
    headers: HeaderMap,
    Query(query): Query<BootstrapQuery>,
) -> Response {
    let (ctx, _token_hash) = match authenticate(&shared.app, &headers).await {
        Ok(pair) => pair,
        Err(res) => return res,
    };
    if !origin_allowed(&headers, &shared) {
        return error_response(StatusCode::FORBIDDEN, "forbidden", "Origin not allowed.");
    }
    if !ctx.scopes.read {
        return error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Token has no read scope.",
        );
    }
    if matches!(ctx.actor, Actor::System) {
        return error_response(StatusCode::FORBIDDEN, "forbidden", "No user context.");
    }
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "`limit` must be between 1 and 2000.",
        );
    }
    let bad = |msg: &str| error_response(StatusCode::BAD_REQUEST, "invalid_request", msg);
    let cursor_in = match &query.cursor {
        Some(raw) => match Cursor::parse(raw) {
            Some(c) => Some(c),
            None => return bad("Invalid `cursor`."),
        },
        None => None,
    };
    let models = match (&cursor_in, &query.model_names) {
        (Some(c), _) => c.models.clone(),
        (None, Some(list)) => match parse_models(list) {
            Some(m) => m,
            None => return bad("`modelNames` contains an unknown or duplicate model."),
        },
        (None, None) => DEFAULT_MODELS.iter().map(|m| (*m).to_owned()).collect(),
    };

    match run(&shared, &ctx, models, cursor_in, limit).await {
        Ok(body) => {
            let mut res = (StatusCode::OK, body).into_response();
            res.headers_mut().insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/x-ndjson; charset=utf-8"),
            );
            res.headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            res
        }
        Err(e) => app_error_response(&e),
    }
}

async fn run(
    shared: &Shared,
    ctx: &lineal_app::RequestCtx,
    models: Vec<String>,
    cursor: Option<Cursor>,
    limit: i64,
) -> Result<String, lineal_app::AppError> {
    let mut tx = shared.app.store.begin(ctx.org_id).await?;
    // Stand des Logs vor dem Lesen der Daten (siehe Moduldoku); bei Folgepaketen aus dem Cursor.
    let current = tx.outbox().sync_head().await?.head;
    let head = match &cursor {
        // Ein manipulierter Cursor darf keinen Stand in der Zukunft melden (der Client würde
        // Änderungen überspringen).
        Some(c) => c.head.min(current),
        None => current,
    };
    let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
    let groups = allowed_groups(&snap, ctx);
    let (mut idx, mut after) = cursor.map_or((0, None), |c| (c.idx, c.after));

    let mut lines: Vec<String> = Vec::new();
    let mut budget = limit;
    while idx < models.len() && budget > 0 {
        let model = &models[idx];
        let requested = budget;
        let page = tx
            .outbox()
            .sync_bootstrap_page(model, after, requested)
            .await?;
        let scanned = i64::try_from(page.len()).unwrap_or(i64::MAX);
        let last = page.last().map(|r| r.id);
        for row in page {
            let visible: Vec<&String> = row.groups.iter().filter(|g| groups.contains(g)).collect();
            if visible.is_empty() {
                continue;
            }
            let mut data = row.data;
            super::redact(model, &mut data, &groups);
            if let Value::Object(map) = &mut data {
                map.insert("__class".into(), json!(model));
                map.insert("syncGroups".into(), json!(visible));
            }
            lines.push(data.to_string());
        }
        budget -= scanned;
        if scanned == 0 || scanned < requested {
            // Modell erschöpft: weiter mit dem nächsten.
            idx += 1;
            after = None;
        } else {
            after = last;
        }
    }
    let done = idx >= models.len();
    let next = (!done).then(|| {
        Cursor {
            head,
            models: models.clone(),
            idx,
            after,
        }
        .encode()
    });
    let meta = json!({
        "lastSyncId": head,
        "subscribedSyncGroups": groups,
        "databaseVersion": DATABASE_VERSION,
        "nextCursor": next,
        "count": lines.len(),
    });
    lines.push(format!("_metadata_={meta}"));
    let mut body = lines.join("\n");
    body.push('\n');
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_roundtrip_and_validation() {
        let c = Cursor {
            head: 42,
            models: vec!["User".into(), "Issue".into()],
            idx: 1,
            after: Some(Uuid::nil()),
        };
        assert_eq!(Cursor::parse(&c.encode()), Some(c));
        assert!(Cursor::parse("x;0;-;User").is_none());
        assert!(Cursor::parse("1;5;-;User").is_none(), "idx out of range");
        assert!(Cursor::parse("1;0;-;Nope").is_none());
        assert!(Cursor::parse("-1;0;-;User").is_none());
        assert!(Cursor::parse(&"a".repeat(600)).is_none());
    }

    #[test]
    fn model_lists_reject_unknown_and_duplicates() {
        assert_eq!(
            parse_models("User, Issue").unwrap(),
            vec!["User".to_owned(), "Issue".to_owned()]
        );
        assert!(parse_models("User,User").is_none());
        assert!(parse_models("Comment").is_none());
        assert!(parse_models("").is_none());
    }
}
