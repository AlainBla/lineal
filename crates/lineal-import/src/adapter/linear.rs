//! Linear-GraphQL-Adapter (`https://api.linear.app/graphql`, nur lesende Abfragen).
//!
//! * Authorization: persönliche Keys (`lin_api_…`) **ohne** `Bearer`, OAuth-Tokens mit `Bearer`.
//! * Paginierung über `pageInfo.endCursor`; Seitengröße 50–100.
//! * Rate-Limit: Header `X-RateLimit-Requests-Remaining`/`-Complexity-Remaining` steuern
//!   vorausschauend (Wartezeit bis `…-Reset`, Unix-Millisekunden); bei `RATELIMITED` (HTTP 400/429
//!   oder GraphQL-Fehler) wird bis zum Reset gewartet und wiederholt.
//! * Fehlertexte enthalten nie das Token oder Antwortinhalte.

use crate::net::UrlPolicy;
use crate::port::{
    Downloaded, ImportSource, PageQuery, SourceEntity, SourceError, SourcePage, SourceResult,
    SourceWorkspace,
};
use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use lineal_app::usecases::import::Secret;
use serde_json::{Value, json};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Standard-Endpunkt.
pub const LINEAR_ENDPOINT: &str = "https://api.linear.app/graphql";

/// Abfrage von Token-Inhaber und Workspace.
pub const WORKSPACE_QUERY: &str = "query { viewer { id email } organization { id name urlKey } }";

/// Wie eine Entität abgefragt wird.
#[derive(Debug, Clone, Copy)]
enum Kind {
    /// `field(first, after) { nodes {...} pageInfo }`.
    Connection,
    /// Liste ohne Paginierung (`templates { ... }`).
    List,
    /// Kindobjekte der Issues (`issues { nodes { id <child_field> {...} } }`), flach als eigene Items.
    ViaIssues {
        child: &'static str,
        as_subscription: bool,
    },
}

struct QuerySpec {
    field: &'static str,
    selection: &'static str,
    kind: Kind,
    /// Zusätzliche Argumente der Verbindung (z. B. `includeArchived: true`).
    args: &'static str,
    /// Unterstützt `filter: { updatedAt: { gt: … } }`.
    filter: bool,
    /// Höchste Seitengröße: Linear lehnt Abfragen über 10 000 Komplexitätspunkte ab
    /// (Objekt 1, Feld 0,1, Verbindung × `first`), große Auswahlen brauchen kleinere Seiten.
    max_first: u32,
}

const ISSUE_FIELDS: &str = "id number identifier title description priority estimate sortOrder dueDate \
    createdAt updatedAt archivedAt startedAt completedAt canceledAt autoClosedAt autoArchivedAt triagedAt \
    snoozedUntilAt slaStartedAt slaMediumRiskAt slaHighRiskAt slaBreachesAt slaType trashed previousIdentifiers \
    url team { id key } state { id } assignee { id } creator { id } parent { id } project { id } \
    projectMilestone { id } cycle { id } labels(first: 50) { nodes { id } }";

fn spec(entity: SourceEntity) -> QuerySpec {
    use SourceEntity::*;
    let conn = |field, selection, args, filter| QuerySpec {
        field,
        selection,
        kind: Kind::Connection,
        args,
        filter,
        max_first: 100,
    };
    let small = |max_first: u32, mut q: QuerySpec| {
        q.max_first = max_first;
        q
    };
    match entity {
        Users => conn(
            "users",
            "id name displayName email admin guest active createdAt updatedAt",
            "includeDisabled: true",
            true,
        ),
        Teams => conn(
            "teams",
            "id key name description icon color visibility timezone createdAt updatedAt archivedAt \
             parent { id } cyclesEnabled cycleDuration cycleCooldownTime cycleStartDay upcomingCycleCount \
             issueEstimationType issueEstimationExtended issueEstimationAllowZero defaultIssueEstimate \
             triageEnabled requirePriorityToLeaveTriage autoClosePeriod autoArchivePeriod \
             groupIssueHistory setIssueSortOrderOnStateChange defaultIssueState { id } \
             triageIssueState { id } autoCloseStateId",
            "includeArchived: true",
            true,
        ),
        TeamMemberships => conn(
            "teamMemberships",
            "id owner createdAt updatedAt user { id } team { id }",
            "",
            false,
        ),
        WorkflowStates => conn(
            "workflowStates",
            "id name type color description position createdAt updatedAt archivedAt team { id }",
            "includeArchived: true",
            false,
        ),
        IssueLabels => conn(
            "issueLabels",
            "id name color description isGroup groupType retiredAt createdAt updatedAt team { id } parent { id } creator { id }",
            "includeArchived: true",
            true,
        ),
        ProjectStatuses => conn(
            "projectStatuses",
            "id name type color description position indefinite createdAt updatedAt archivedAt",
            "includeArchived: true",
            false,
        ),
        ProjectLabels => conn(
            "projectLabels",
            "id name color description isGroup retiredAt createdAt updatedAt parent { id } creator { id }",
            "includeArchived: true",
            false,
        ),
        Templates => QuerySpec {
            field: "templates",
            selection: "id name type description templateData createdAt updatedAt team { id } creator { id }",
            kind: Kind::List,
            args: "",
            filter: false,
            max_first: 100,
        },
        Cycles => conn(
            "cycles",
            "id number name description startsAt endsAt completedAt createdAt updatedAt archivedAt team { id } \
             scopeHistory completedScopeHistory inProgressScopeHistory issueCountHistory completedIssueCountHistory",
            "includeArchived: true",
            true,
        ),
        Initiatives => conn(
            "initiatives",
            "id name description content color icon status sortOrder slugId targetDate startedAt completedAt \
             createdAt updatedAt archivedAt owner { id } creator { id } parentInitiative { id }",
            "includeArchived: true",
            true,
        ),
        Projects => small(
            15,
            conn(
                "projects",
                "id name description content color icon priority sortOrder slugId startDate targetDate startedAt \
             completedAt canceledAt health createdAt updatedAt archivedAt status { id name } lead { id } creator { id } \
             teams(first: 20) { nodes { id } } members(first: 50) { nodes { id } } \
             labels(first: 20) { nodes { id } } initiatives(first: 10) { nodes { id } }",
                "includeArchived: true",
                true,
            ),
        ),
        ProjectMilestones => conn(
            "projectMilestones",
            "id name description targetDate sortOrder createdAt updatedAt archivedAt project { id }",
            "includeArchived: true",
            false,
        ),
        Issues => small(
            25,
            conn("issues", ISSUE_FIELDS, "includeArchived: true", true),
        ),
        IssueRelations => conn(
            "issueRelations",
            "id type createdAt updatedAt issue { id identifier url } relatedIssue { id identifier url }",
            "includeArchived: true",
            false,
        ),
        IssueHistory => QuerySpec {
            field: "issues",
            selection: "id history(first: 30) { nodes { id createdAt actor { id } \
                fromState { id } toState { id } fromAssignee { id } toAssignee { id } fromPriority toPriority \
                fromTeam { id } toTeam { id } fromProject { id } toProject { id } fromCycle { id } toCycle { id } } }",
            kind: Kind::ViaIssues {
                child: "history",
                as_subscription: false,
            },
            args: "includeArchived: true",
            filter: true,
            max_first: 10,
        },
        IssueSubscriptions => QuerySpec {
            field: "issues",
            selection: "id subscribers(first: 50) { nodes { id } }",
            kind: Kind::ViaIssues {
                child: "subscribers",
                as_subscription: true,
            },
            args: "includeArchived: true",
            filter: true,
            max_first: 20,
        },
        Comments => conn(
            "comments",
            "id body createdAt updatedAt editedAt quotedText resolvedAt user { id } issue { id } parent { id } \
             projectUpdate { id } initiativeUpdate { id } reactions { id emoji createdAt user { id } }",
            "includeArchived: true",
            true,
        ),
        Attachments => conn(
            "attachments",
            "id url title subtitle sourceType metadata createdAt updatedAt issue { id } creator { id }",
            "includeArchived: true",
            false,
        ),
        Documents => conn(
            "documents",
            "id title content icon color slugId createdAt updatedAt archivedAt creator { id } \
             project { id } initiative { id } issue { id }",
            "includeArchived: true",
            true,
        ),
        ProjectUpdates => conn(
            "projectUpdates",
            "id body health createdAt updatedAt editedAt user { id } project { id }",
            "includeArchived: true",
            false,
        ),
        InitiativeUpdates => conn(
            "initiativeUpdates",
            "id body health createdAt updatedAt editedAt user { id } initiative { id }",
            "includeArchived: true",
            false,
        ),
        CustomViews => conn(
            "customViews",
            "id name description icon color shared filterData createdAt updatedAt team { id } owner { id }",
            "",
            false,
        ),
        Favorites => conn(
            "favorites",
            "id type sortOrder createdAt updatedAt owner { id } issue { id } project { id } customView { id } \
             initiative { id }",
            "",
            false,
        ),
    }
}

/// Baut `Authorization`-Header-Wert: Personal Keys roh, alles andere als `Bearer`.
pub fn authorization_value(token: &Secret) -> String {
    let t = token.expose();
    if t.starts_with("lin_oauth_") || t.starts_with("Bearer ") {
        let raw = t.strip_prefix("Bearer ").unwrap_or(t);
        format!("Bearer {raw}")
    } else {
        t.to_owned()
    }
}

/// Löst `{ "nodes": [...] }` rekursiv zu Arrays auf (normalisiertes Format, siehe `port`).
pub fn flatten_connections(v: Value) -> Value {
    match v {
        Value::Object(mut m) => {
            if m.len() <= 2
                && m.contains_key("nodes")
                && m.keys().all(|k| k == "nodes" || k == "pageInfo")
            {
                if let Some(nodes) = m.remove("nodes") {
                    return flatten_connections(nodes);
                }
            }
            Value::Object(
                m.into_iter()
                    .map(|(k, v)| (k, flatten_connections(v)))
                    .collect(),
            )
        }
        Value::Array(a) => Value::Array(a.into_iter().map(flatten_connections).collect()),
        other => other,
    }
}

/// Der GraphQL-Request (Query-Text und Variablen) für eine Seite.
pub fn build_query(q: &PageQuery) -> Value {
    let s = spec(q.entity);
    let mut args = Vec::new();
    if !matches!(s.kind, Kind::List) {
        args.push("first: $first".to_owned());
        args.push("after: $after".to_owned());
    }
    if !s.args.is_empty() {
        args.push(s.args.to_owned());
    }
    if let (true, Some(after)) = (s.filter, q.updated_after) {
        args.push(format!(
            "filter: {{ updatedAt: {{ gt: \"{}\" }} }}",
            after.to_rfc3339_opts(SecondsFormat::Millis, true)
        ));
    }
    let arg_list = if args.is_empty() {
        String::new()
    } else {
        format!("({})", args.join(", "))
    };
    let query = match s.kind {
        Kind::List => format!("query {{ {}{arg_list} {{ {} }} }}", s.field, s.selection),
        _ => format!(
            "query($first: Int!, $after: String) {{ {}{arg_list} {{ nodes {{ {} }} pageInfo {{ hasNextPage endCursor }} }} }}",
            s.field, s.selection
        ),
    };
    json!({
        "query": query,
        "variables": { "first": q.limit.clamp(1, s.max_first), "after": q.cursor },
    })
}

/// Konfiguration des Adapters.
#[derive(Debug, Clone)]
pub struct LinearConfig {
    pub endpoint: String,
    pub policy: UrlPolicy,
    /// Wiederholungen bei Rate-Limit/Serverfehlern je Anfrage.
    pub max_retries: u32,
    /// Basis des exponentiellen Backoffs bei Serverfehlern.
    pub backoff_base: Duration,
    /// Längste Wartezeit auf einen Rate-Limit-Reset.
    pub max_wait: Duration,
    /// Ab so wenigen verbleibenden Requests wird vorausschauend bis zum Reset gewartet.
    pub low_requests_threshold: u64,
    /// Dito für die Abfragekomplexität.
    pub low_complexity_threshold: u64,
}

impl Default for LinearConfig {
    fn default() -> Self {
        Self {
            endpoint: LINEAR_ENDPOINT.to_owned(),
            policy: UrlPolicy::linear(),
            max_retries: 5,
            backoff_base: Duration::from_secs(1),
            max_wait: Duration::from_secs(15 * 60),
            low_requests_threshold: 5,
            low_complexity_threshold: 20_000,
        }
    }
}

/// Linear als Importquelle.
pub struct LinearSource {
    client: reqwest::Client,
    config: LinearConfig,
    auth: String,
    /// Frühester Zeitpunkt der nächsten Anfrage (Rate-Limit-Steuerung).
    not_before: Mutex<Option<Instant>>,
}

impl std::fmt::Debug for LinearSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinearSource")
            .field("endpoint", &self.config.endpoint)
            .field("auth", &"[REDACTED]")
            .finish()
    }
}

fn wait_from_reset_ms(reset_ms: u64, max: Duration) -> Duration {
    let now_ms = u64::try_from(Utc::now().timestamp_millis()).unwrap_or(0);
    Duration::from_millis(reset_ms.saturating_sub(now_ms)).min(max)
}

impl LinearSource {
    /// Quelle mit Standardkonfiguration.
    pub fn new(token: &Secret) -> SourceResult<Self> {
        Self::with_config(token, LinearConfig::default())
    }

    /// Quelle mit eigener Konfiguration (z. B. Mock-Server in Tests).
    pub fn with_config(token: &Secret, config: LinearConfig) -> SourceResult<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(60))
            .user_agent("lineal-import/0.1")
            .build()
            .map_err(|_| SourceError::Transport("http client setup failed".into()))?;
        Ok(Self {
            client,
            auth: authorization_value(token),
            config,
            not_before: Mutex::new(None),
        })
    }

    fn lock_not_before(&self) -> std::sync::MutexGuard<'_, Option<Instant>> {
        self.not_before.lock().unwrap_or_else(|p| p.into_inner())
    }

    async fn throttle(&self) {
        let until = *self.lock_not_before();
        if let Some(t) = until {
            let now = Instant::now();
            if t > now {
                tokio::time::sleep(t - now).await;
            }
        }
    }

    fn defer(&self, wait: Duration) {
        let until = Instant::now() + wait.min(self.config.max_wait);
        let mut g = self.lock_not_before();
        if g.is_none_or(|cur| cur < until) {
            *g = Some(until);
        }
    }

    /// Eine Anfrage mit Rate-Limit-Steuerung, Backoff und Wiederholung. Liefert `data`.
    async fn execute(&self, body: &Value) -> SourceResult<Value> {
        let mut attempt = 0u32;
        loop {
            self.throttle().await;
            match self.execute_once(body).await {
                Ok(v) => return Ok(v),
                Err(SourceError::RateLimited(wait)) if attempt < self.config.max_retries => {
                    attempt += 1;
                    let wait = wait
                        .unwrap_or(Duration::from_secs(60))
                        .min(self.config.max_wait);
                    tracing::warn!(
                        attempt,
                        wait_ms = wait.as_millis() as u64,
                        "import source rate limit, waiting"
                    );
                    self.defer(wait);
                }
                Err(SourceError::Transport(m)) if attempt < self.config.max_retries.min(3) => {
                    attempt += 1;
                    let wait = self.config.backoff_base * 2u32.saturating_pow(attempt - 1);
                    tracing::warn!(attempt, error = %m, "import source transport error, retrying");
                    tokio::time::sleep(wait.min(self.config.max_wait)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn execute_once(&self, body: &Value) -> SourceResult<Value> {
        let resp = self
            .client
            .post(&self.config.endpoint)
            .header(reqwest::header::AUTHORIZATION, &self.auth)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await
            .map_err(|_| SourceError::Transport("request failed".into()))?;
        let status = resp.status();
        let headers = resp.headers().clone();
        let num = |name: &str| -> Option<u64> {
            headers
                .get(name)?
                .to_str()
                .ok()?
                .trim()
                .parse::<f64>()
                .ok()
                .map(|f| f as u64)
        };
        let reset_wait =
            |name: &str| num(name).map(|ms| wait_from_reset_ms(ms, self.config.max_wait));
        // Vorausschauend drosseln.
        if num("x-ratelimit-requests-remaining")
            .is_some_and(|r| r <= self.config.low_requests_threshold)
        {
            if let Some(w) = reset_wait("x-ratelimit-requests-reset") {
                self.defer(w);
            }
        }
        if num("x-ratelimit-complexity-remaining")
            .is_some_and(|r| r <= self.config.low_complexity_threshold)
        {
            if let Some(w) = reset_wait("x-ratelimit-complexity-reset") {
                self.defer(w);
            }
        }
        let limited_wait = || {
            let a = reset_wait("x-ratelimit-requests-reset");
            let b = reset_wait("x-ratelimit-complexity-reset");
            match (a, b) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            }
        };
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(SourceError::Auth);
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(SourceError::RateLimited(limited_wait()));
        }
        if status.is_server_error() {
            return Err(SourceError::Transport(format!("HTTP {}", status.as_u16())));
        }
        let payload: Value = resp
            .json()
            .await
            .map_err(|_| SourceError::Invalid("response is not JSON".into()))?;
        if let Some(errors) = payload.get("errors").and_then(Value::as_array) {
            let code = |e: &Value| {
                e["extensions"]["code"]
                    .as_str()
                    .or_else(|| e["extensions"]["type"].as_str())
                    .unwrap_or("")
                    .to_ascii_uppercase()
            };
            if errors.iter().any(|e| code(e).contains("RATELIMITED")) {
                return Err(SourceError::RateLimited(limited_wait()));
            }
            if errors.iter().any(|e| {
                let c = code(e);
                c.contains("AUTHENTICATION") || c.contains("FORBIDDEN")
            }) {
                return Err(SourceError::Auth);
            }
            if payload.get("data").is_none_or(Value::is_null) {
                let c = errors.first().map(code).unwrap_or_default();
                let c: String = c
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == ' ')
                    .take(60)
                    .collect();
                return Err(SourceError::Invalid(format!(
                    "the source returned an error ({c})"
                )));
            }
        }
        if !status.is_success() {
            return Err(SourceError::Invalid(format!("HTTP {}", status.as_u16())));
        }
        payload
            .get("data")
            .cloned()
            .ok_or_else(|| SourceError::Invalid("response has no data".into()))
    }
}

#[async_trait]
impl ImportSource for LinearSource {
    fn kind(&self) -> &'static str {
        "linear"
    }

    async fn workspace(&self) -> SourceResult<SourceWorkspace> {
        let body = json!({ "query": WORKSPACE_QUERY });
        let data = self.execute(&body).await?;
        let org = &data["organization"];
        let text = |v: &Value| v.as_str().map(str::to_owned);
        Ok(SourceWorkspace {
            id: text(&org["id"])
                .ok_or_else(|| SourceError::Invalid("organization missing".into()))?,
            name: text(&org["name"]).unwrap_or_default(),
            url_key: text(&org["urlKey"]).unwrap_or_default(),
            viewer_email: text(&data["viewer"]["email"]),
        })
    }

    async fn page(&self, q: &PageQuery) -> SourceResult<SourcePage> {
        let s = spec(q.entity);
        let data = self.execute(&build_query(q)).await?;
        let root = &data[s.field];
        let next_cursor = match s.kind {
            Kind::List => None,
            _ => {
                let info = &root["pageInfo"];
                if info["hasNextPage"].as_bool().unwrap_or(false) {
                    info["endCursor"].as_str().map(str::to_owned)
                } else {
                    None
                }
            }
        };
        let nodes: Vec<Value> = match s.kind {
            Kind::List => root.as_array().cloned().unwrap_or_default(),
            _ => root["nodes"].as_array().cloned().unwrap_or_default(),
        };
        let nodes: Vec<Value> = nodes.into_iter().map(flatten_connections).collect();
        let items = match s.kind {
            Kind::ViaIssues { child, as_subscription } => nodes
                .into_iter()
                .flat_map(|issue| {
                    let issue_id = issue["id"].clone();
                    issue[child]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .map(move |c| {
                            if as_subscription {
                                json!({
                                    "id": format!("{}:{}", issue_id.as_str().unwrap_or(""), c["id"].as_str().unwrap_or("")),
                                    "issue": { "id": issue_id },
                                    "user": { "id": c["id"] },
                                })
                            } else {
                                let mut c = c;
                                c["issue"] = json!({ "id": issue_id });
                                c
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .collect(),
            _ => nodes,
        };
        Ok(SourcePage { items, next_cursor })
    }

    async fn download(&self, url: &str) -> SourceResult<Downloaded> {
        self.config.policy.fetch(url, Some(&self.auth)).await
    }
}

/// Hilfe für Tests/Plan: Zeitpunkt als Linear-ISO-String.
pub fn iso(ts: DateTime<Utc>) -> String {
    ts.to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_key_is_sent_without_bearer_and_oauth_with() {
        let key = Secret::new("lin_api_abcdefgh1234").unwrap();
        assert_eq!(authorization_value(&key), "lin_api_abcdefgh1234");
        let oauth = Secret::new("lin_oauth_abcdefgh1234").unwrap();
        assert_eq!(authorization_value(&oauth), "Bearer lin_oauth_abcdefgh1234");
    }

    #[test]
    fn issue_query_is_paginated_filtered_and_read_only() {
        let after: DateTime<Utc> = "2026-03-01T10:00:00Z".parse().unwrap();
        let body = build_query(&PageQuery {
            entity: SourceEntity::Issues,
            cursor: Some("abc".into()),
            updated_after: Some(after),
            limit: 50,
        });
        let q = body["query"].as_str().unwrap();
        assert!(
            q.starts_with("query("),
            "must be a query, never a mutation: {q}"
        );
        assert!(!q.contains("mutation"));
        assert!(q.contains("issues(first: $first, after: $after, includeArchived: true"));
        assert!(q.contains("filter: { updatedAt: { gt: \"2026-03-01T10:00:00.000Z\" } }"));
        assert!(q.contains("pageInfo { hasNextPage endCursor }"));
        assert_eq!(
            body["variables"]["first"], 25,
            "issues use smaller pages (complexity limit)"
        );
        assert_eq!(body["variables"]["after"], "abc");
    }

    #[test]
    fn every_entity_builds_a_read_only_query() {
        for e in SourceEntity::ALL {
            let body = build_query(&PageQuery {
                entity: e,
                cursor: None,
                updated_after: None,
                limit: 1000,
            });
            let q = body["query"].as_str().unwrap();
            assert!(q.starts_with("query"), "{e:?}");
            assert!(!q.to_lowercase().contains("mutation"), "{e:?}");
            let first = body["variables"]["first"].as_u64().unwrap();
            assert!((1..=100).contains(&first), "page size is clamped: {e:?}");
        }
    }

    #[test]
    fn connections_are_flattened() {
        let v = json!({"id":"1","labels":{"nodes":[{"id":"a"},{"id":"b"}]},"team":{"id":"t"}});
        let f = flatten_connections(v);
        assert_eq!(f["labels"], json!([{"id":"a"},{"id":"b"}]));
        assert_eq!(f["team"], json!({"id":"t"}));
    }

    #[test]
    fn debug_output_hides_the_token() {
        let key = Secret::new("lin_api_SUPERSECRET99").unwrap();
        let s = LinearSource::new(&key).unwrap();
        assert!(!format!("{s:?}").contains("SUPERSECRET"));
    }
}
