#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]
//! Kleiner Mock-Linear-Server (axum, nur `127.0.0.1`) für Integrationstests des Linear-Pfads.
//!
//! Er beantwortet die GraphQL-Abfragen des `LinearSource`-Adapters (Verbindungen mit `first`/`after`,
//! Listen, `viewer`/`organization`, Kindobjekte der Issues) aus den Daten einer `FakeSource`
//! (normalisiertes Format). Filter (`updatedAt`, Team/Projekt) werden ignoriert. Optional lässt
//! sich das Verhalten skripten: Rate-Limit (HTTP 429) und Serverfehler vor der nächsten Antwort,
//! Verzögerung je Anfrage. Es wird nie ein Fremdsystem kontaktiert.

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use lineal_import::{FakeSource, ImportSource, SourceEntity};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Skriptbare Störung vor der nächsten regulären Antwort.
#[derive(Debug, Clone, Copy)]
pub enum Fault {
    /// HTTP 429 mit Reset in `ms` Millisekunden.
    RateLimited(u64),
    /// HTTP 502.
    ServerError,
    /// HTTP 503 mit `Retry-After` in Sekunden.
    RetryAfter(u64),
}

pub struct MockLinear {
    source: Arc<FakeSource>,
    pub faults: Mutex<VecDeque<Fault>>,
    pub delay: Mutex<Duration>,
    /// So viele der nächsten Antworten enthalten neben den Daten auch `errors` (Teilfehler).
    pub partial_errors: Mutex<usize>,
    /// (Wurzelfeld, Autorisierungs-Header) je Anfrage.
    pub calls: Mutex<Vec<(String, Option<String>)>>,
}

impl MockLinear {
    /// Startet den Server und liefert Handle und Endpunkt-URL (`http://127.0.0.1:<port>/graphql`).
    pub async fn start(source: Arc<FakeSource>) -> (Arc<MockLinear>, String) {
        let mock = Arc::new(MockLinear {
            source,
            faults: Mutex::default(),
            delay: Mutex::default(),
            partial_errors: Mutex::default(),
            calls: Mutex::default(),
        });
        let app = Router::new()
            .route("/graphql", post(handle))
            .with_state(mock.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (mock, format!("http://{addr}/graphql"))
    }

    pub fn calls_for(&self, field: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(f, _)| f == field)
            .count()
    }
}

fn entity_for(field: &str) -> Option<SourceEntity> {
    Some(match field {
        "users" => SourceEntity::Users,
        "teams" => SourceEntity::Teams,
        "teamMemberships" => SourceEntity::TeamMemberships,
        "workflowStates" => SourceEntity::WorkflowStates,
        "issueLabels" => SourceEntity::IssueLabels,
        "projectStatuses" => SourceEntity::ProjectStatuses,
        "projectLabels" => SourceEntity::ProjectLabels,
        "templates" => SourceEntity::Templates,
        "cycles" => SourceEntity::Cycles,
        "initiatives" => SourceEntity::Initiatives,
        "projects" => SourceEntity::Projects,
        "projectMilestones" => SourceEntity::ProjectMilestones,
        "issues" => SourceEntity::Issues,
        "issueRelations" => SourceEntity::IssueRelations,
        "comments" => SourceEntity::Comments,
        "attachments" => SourceEntity::Attachments,
        "documents" => SourceEntity::Documents,
        "projectUpdates" => SourceEntity::ProjectUpdates,
        "initiativeUpdates" => SourceEntity::InitiativeUpdates,
        "customViews" => SourceEntity::CustomViews,
        "favorites" => SourceEntity::Favorites,
        _ => return None,
    })
}

/// Erstes Feld des Abfragekörpers (`query($first: Int!) { issues(...) {` -> `issues`).
fn root_field(query: &str) -> Option<String> {
    let after = &query[query.find('{')? + 1..];
    let name: String = after
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

async fn handle(
    State(m): State<Arc<MockLinear>>,
    headers: axum::http::HeaderMap,
    body: axum::Json<Value>,
) -> Response {
    let query = body["query"].as_str().unwrap_or_default().to_owned();
    let field = root_field(&query).unwrap_or_default();
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    m.calls.lock().unwrap().push((field.clone(), auth));
    let delay = *m.delay.lock().unwrap();
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    let fault = m.faults.lock().unwrap().pop_front();
    match fault {
        Some(Fault::RateLimited(ms)) => {
            let reset = (chrono::Utc::now().timestamp_millis() + ms as i64).to_string();
            let mut r = StatusCode::TOO_MANY_REQUESTS.into_response();
            r.headers_mut().insert(
                "x-ratelimit-requests-reset",
                HeaderValue::from_str(&reset).unwrap(),
            );
            return r;
        }
        Some(Fault::ServerError) => return StatusCode::BAD_GATEWAY.into_response(),
        Some(Fault::RetryAfter(secs)) => {
            let mut r = StatusCode::SERVICE_UNAVAILABLE.into_response();
            r.headers_mut().insert(
                "retry-after",
                HeaderValue::from_str(&secs.to_string()).unwrap(),
            );
            return r;
        }
        None => {}
    }
    let data = answer(&m, &query, &field, &body["variables"]).await;
    let mut payload = json!({ "data": data });
    {
        let mut partial = m.partial_errors.lock().unwrap();
        if *partial > 0 && field != "viewer" {
            *partial -= 1;
            payload["errors"] =
                json!([{ "message": "boom", "extensions": { "code": "INTERNAL_SERVER_ERROR" } }]);
        }
    }
    let mut r = axum::Json(payload).into_response();
    r.headers_mut().insert(
        "x-ratelimit-requests-remaining",
        HeaderValue::from_static("1000"),
    );
    r
}

async fn answer(m: &MockLinear, query: &str, field: &str, vars: &Value) -> Value {
    if field == "viewer" {
        let ws = m.source.workspace().await.unwrap();
        return json!({
            "viewer": { "id": "u-viewer", "email": ws.viewer_email },
            "organization": { "id": ws.id, "name": ws.name, "urlKey": ws.url_key },
        });
    }
    let entity = entity_for(field).unwrap_or_else(|| panic!("mock: unknown root field {field}"));
    let items = m.source.entity(entity);
    if entity == SourceEntity::Templates {
        return json!({ field: items });
    }
    let first = vars["first"].as_u64().unwrap_or(50) as usize;
    let offset: usize = vars["after"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let page: Vec<Value> = items.iter().skip(offset).take(first).cloned().collect();
    let end = offset + page.len();
    let nodes: Vec<Value> = if field == "issues" && query.contains("history(first") {
        let history = m.source.entity(SourceEntity::IssueHistory);
        page.iter()
            .map(|i| {
                let mine: Vec<&Value> = history
                    .iter()
                    .filter(|h| h["issue"]["id"] == i["id"])
                    .collect();
                json!({ "id": i["id"], "history": { "nodes": mine } })
            })
            .collect()
    } else if field == "issues" && query.contains("subscribers(first") {
        let subs = m.source.entity(SourceEntity::IssueSubscriptions);
        page.iter()
            .map(|i| {
                let users: Vec<Value> = subs
                    .iter()
                    .filter(|s| s["issue"]["id"] == i["id"])
                    .map(|s| json!({ "id": s["user"]["id"] }))
                    .collect();
                json!({ "id": i["id"], "subscribers": { "nodes": users } })
            })
            .collect()
    } else {
        // Verbindungen wie bei Linear (`labels { nodes }`); der Adapter löst sie wieder auf.
        page.into_iter().map(connections).collect()
    };
    json!({ field: {
        "nodes": nodes,
        "pageInfo": { "hasNextPage": end < items.len(), "endCursor": end.to_string() },
    } })
}

/// Listenfelder, die Linear als Verbindung liefert (`labels`, `teams`, `members`, `initiatives`).
fn connections(mut v: Value) -> Value {
    if let Some(obj) = v.as_object_mut() {
        for key in ["labels", "teams", "members", "initiatives"] {
            if let Some(arr) = obj.get(key).filter(|x| x.is_array()).cloned() {
                obj.insert(key.to_owned(), json!({ "nodes": arr }));
            }
        }
    }
    v
}
