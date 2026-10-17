//! `POST /graphql`-Pipeline (docs/09 §1.2, §3.2): Auth -> Parse -> Scope -> Komplexität ->
//! Rate-Limit -> Ausführung mit `RequestCtx` in den Context-Daten -> Fehler mit `extensions.code`.

use crate::error::graphql_error_response;
use crate::graphql::LinealSchema;
use crate::middleware::auth::{AuthState, CredentialClass};
use crate::rate_limit::{RateHandle, complexity_headers, denied_error};
use async_graphql::parser::types::{
    DocumentOperations, ExecutableDocument, OperationType, Selection, SelectionSet,
};
use async_graphql::{Name, Request, Response as GqlResponse, Value};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lineal_app::AppError;
use serde_json::json;

/// Default-Seitengröße einer Connection ohne `first`/`last` (09 §3.2).
const DEFAULT_CONNECTION_SIZE: u64 = 50;
const MAX_FRAGMENT_DEPTH: usize = 64;

/// Kosten (Zehntelpunkte) eines Freitext-Leaffelds (`description`, `body`, …) je Knoten. Diese
/// Felder können bis zu 500 000 Zeichen enthalten (F-09/DOS-01); als Skalar (1) gewogen ließen
/// sich per Alias-Vervielfachung hunderte MB je Anfrage bei winziger `x-complexity` abrufen.
/// Bei 10 000 Punkten Query-Budget bleiben so höchstens rund 100 Freitext-Knoten je Anfrage.
const HEAVY_TEXT_COST: u64 = 100;
/// Kosten (Zehntelpunkte) je Eintrag einer Id-Liste (`input.ids` der Batch-Mutationen, F-09/DOS-03):
/// jeder Eintrag ist eine volle Einzeländerung samt Events.
const BATCH_ID_COST: u64 = 100;
/// Obergrenze der serialisierten Antwort einer Query (Verteidigung in der Tiefe; Mutationen
/// sind ausgenommen, weil sie dann bereits ausgeführt wären).
pub const MAX_QUERY_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// Felder, die große Freitexte liefern können.
fn is_heavy_text(name: &str) -> bool {
    matches!(
        name,
        "description"
            | "body"
            | "content"
            | "diffMarkdown"
            | "markdown"
            | "summary"
            | "quotedText"
            | "templateData"
    )
}

/// Einfache Listen von Objekten ohne `first`/`last`/`nodes` (kein Connection-Muster): Faktor =
/// angenommene Zeilenzahl. Ohne diese Gewichtung zählte `cycles { description }` einmal statt
/// je Zeile. `first` (falls gesetzt) gilt über [`is_connection`]; hier nur die Default-Größen.
fn plain_list_size(name: &str) -> Option<u64> {
    match name {
        // Batch-Payloads: bis zu 250 Issues je Aufruf.
        "issues" | "subIssues" => Some(250),
        // Je Projekt eine Handvoll (Obergrenze 50 nur in Vorlagen); 5 hält die Web-Listen im Budget.
        "milestones" | "projectMilestones" => Some(5),
        "cycles" | "templates" | "workflowStates" | "projects" | "trashedProjects"
        | "initiatives" | "trashedInitiatives" | "projectUpdates" | "initiativeUpdates"
        | "trashedIssues" | "triageIssues" | "issueLabels" | "projectLabels" | "attachments"
        | "recurringIssues" => Some(DEFAULT_CONNECTION_SIZE),
        _ => None,
    }
}

/// Anzahl der Einträge in `ids`-Listen der Argumente (`ids: [...]` oder `input: {ids: [...]}`).
fn batch_ids(field: &async_graphql::parser::types::Field, request: &Request) -> u64 {
    fn count(name: Option<&str>, v: &Value) -> u64 {
        match v {
            Value::List(items) if name == Some("ids") => items.len() as u64,
            Value::Object(map) => map.iter().map(|(k, v)| count(Some(k.as_str()), v)).sum(),
            _ => 0,
        }
    }
    field
        .arguments
        .iter()
        .filter_map(|(n, v)| {
            let value = v
                .node
                .clone()
                .into_const_with(|var| request.variables.get(&var).cloned().ok_or(()))
                .ok()?;
            Some(count(Some(n.node.as_str()), &value))
        })
        .sum()
}

/// Statische Komplexität nach 09 §3.2 (aufgerundet). Intern in Zehntelpunkten gerechnet,
/// damit Gleitkomma-Rauschen das Aufrunden nicht verfälscht.
pub fn complexity(doc: &ExecutableDocument, operation: &SelectionSet, request: &Request) -> u64 {
    let mut walk = Walk::default();
    selection_cost(doc, operation, request, &mut walk).div_ceil(10)
}

/// Zustand der Kostenberechnung. Fragmentkosten sind kontextfrei (Multiplikatoren wendet der
/// Aufrufer an) und werden memoisiert: Ohne Cache wäre die Berechnung bei verschachtelten,
/// mehrfach referenzierten Fragmenten exponentiell (CPU-DoS vor jeder Begrenzung).
#[derive(Default)]
struct Walk {
    stack: Vec<Name>,
    memo: std::collections::HashMap<Name, u64>,
}

fn int_arg(
    value: &async_graphql::parser::types::Field,
    name: &str,
    request: &Request,
) -> Option<u64> {
    let (_, v) = value.arguments.iter().find(|(n, _)| n.node == name)?;
    let value = v
        .node
        .clone()
        .into_const_with(|var| request.variables.get(&var).cloned().ok_or(()))
        .ok()?;
    match value {
        Value::Number(n) => n.as_u64(),
        _ => None,
    }
}

/// Connection: Argument `first`/`last` oder direktes Kind `nodes`/`edges` (ohne Schema-Typen).
fn is_connection(field: &async_graphql::parser::types::Field) -> bool {
    // `Issue.children` ist eine Liste mit Default-Größe 50.
    field.name.node == "children"
        || field
            .arguments
            .iter()
            .any(|(n, _)| n.node == "first" || n.node == "last")
        || field.selection_set.node.items.iter().any(|s| {
            matches!(&s.node, Selection::Field(f)
                if matches!(f.node.name.node.as_str(), "nodes" | "edges"))
        })
}

fn selection_cost(
    doc: &ExecutableDocument,
    set: &SelectionSet,
    request: &Request,
    walk: &mut Walk,
) -> u64 {
    let mut total: u64 = 0;
    for sel in &set.items {
        match &sel.node {
            Selection::Field(f) => {
                let field = &f.node;
                let children = &field.selection_set.node;
                let name = field.name.node.as_str();
                total =
                    total.saturating_add(batch_ids(field, request).saturating_mul(BATCH_ID_COST));
                if children.items.is_empty() {
                    total += if is_heavy_text(name) {
                        HEAVY_TEXT_COST
                    } else {
                        1
                    };
                } else if matches!(name, "nodes" | "edges" | "node") {
                    total = total.saturating_add(selection_cost(doc, children, request, walk));
                } else if is_connection(field) {
                    let n = int_arg(field, "first", request)
                        .or_else(|| int_arg(field, "last", request))
                        .unwrap_or(DEFAULT_CONNECTION_SIZE);
                    let inner = selection_cost(doc, children, request, walk);
                    total = total.saturating_add(10u64.saturating_add(n.saturating_mul(inner)));
                } else if let Some(n) = plain_list_size(name) {
                    let inner = selection_cost(doc, children, request, walk);
                    total = total.saturating_add(10u64.saturating_add(n.saturating_mul(inner)));
                } else if name == "history" {
                    // Aktivitätsverlauf: bis zu 1000 Zeilen je Issue, pauschal mit Faktor 100.
                    let inner = selection_cost(doc, children, request, walk);
                    total = total.saturating_add(10u64.saturating_add(inner.saturating_mul(100)));
                } else {
                    total = total.saturating_add(10 + selection_cost(doc, children, request, walk));
                }
            }
            Selection::InlineFragment(f) => {
                total = total.saturating_add(selection_cost(
                    doc,
                    &f.node.selection_set.node,
                    request,
                    walk,
                ));
            }
            Selection::FragmentSpread(s) => {
                let name = &s.node.fragment_name.node;
                if let Some(cost) = walk.memo.get(name) {
                    total = total.saturating_add(*cost);
                    continue;
                }
                // Zyklen sind ungültig (die Validierung meldet sie); hier nur nicht rekursieren.
                if walk.stack.contains(name) || walk.stack.len() > MAX_FRAGMENT_DEPTH {
                    continue;
                }
                if let Some(frag) = doc.fragments.get(name) {
                    walk.stack.push(name.clone());
                    let cost = selection_cost(doc, &frag.node.selection_set.node, request, walk);
                    walk.stack.pop();
                    walk.memo.insert(name.clone(), cost);
                    total = total.saturating_add(cost);
                }
            }
        }
    }
    total
}

/// Wählt die auszuführende Operation (wie die Ausführung selbst).
fn operation<'a>(
    doc: &'a ExecutableDocument,
    name: Option<&str>,
) -> Option<&'a async_graphql::parser::types::OperationDefinition> {
    match &doc.operations {
        DocumentOperations::Single(op) => Some(&op.node),
        DocumentOperations::Multiple(ops) => match name {
            Some(n) => ops.get(n).map(|o| &o.node),
            None if ops.len() == 1 => ops.values().next().map(|o| &o.node),
            None => None,
        },
    }
}

/// Axum-Standardgrenze für JSON-Bodies (2 MiB), hier für die Fehlermeldung.
const MAX_BODY_BYTES: u64 = 2 * 1024 * 1024;

/// Antwort auf einen unlesbaren Request-Body (Fehlermodell). Zu groß: `413 limit_exceeded`
/// (B3.20); Daten-/Syntaxfehler (z. B. ein JSON-Array als Batch) sind `400 invalid_request`
/// ohne interne Serde-Meldung (B8.17).
pub fn bad_body(rejection: &JsonRejection) -> Response {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        // B3.20: Code, Status in Body und HTTP-Status stimmen überein, die Meldung nennt das Limit.
        let e = AppError::limit_exceeded(format!(
            "Request body exceeds the maximum of {MAX_BODY_BYTES} bytes."
        ))
        .with_details(serde_json::json!({ "maxBytes": MAX_BODY_BYTES }));
        let rid = crate::middleware::request_id::current();
        let mut err = crate::error::gql_error(&e, &rid);
        if let Some(ext) = err.extensions.as_mut() {
            ext.set("status", 413);
        }
        let mut server_error = err.into_server_error(Default::default());
        server_error.locations.clear();
        let body = async_graphql::Response::from_errors(vec![server_error]);
        return (StatusCode::PAYLOAD_TOO_LARGE, axum::Json(body)).into_response();
    }
    let status = rejection.status();
    let message = match rejection {
        JsonRejection::JsonDataError(_) | JsonRejection::JsonSyntaxError(_) => {
            "The request body must be a single GraphQL request object with a `query` string \
             (batching is not supported)."
                .to_owned()
        }
        other => other.body_text(),
    };
    let mut res = graphql_error_response(&AppError::invalid_request(message));
    *res.status_mut() = if status == StatusCode::UNPROCESSABLE_ENTITY {
        StatusCode::BAD_REQUEST
    } else {
        status
    };
    res
}

/// Ergänzt Pflicht-Extensions an Fehlern ohne `code` (Validierung, ungeprüfte Resolver).
fn finish_errors(response: &mut GqlResponse, request_id: &str) -> bool {
    let query_level_only = !response.errors.is_empty()
        && response
            .errors
            .iter()
            .all(|e| e.path.is_empty() && e.extensions.is_none());
    for e in &mut response.errors {
        let ext = e.extensions.get_or_insert_with(Default::default);
        if ext.get("code").is_none() {
            if e.path.is_empty() || crate::error::is_engine_input_error(&e.message) {
                ext.set("code", "invalid_request");
                ext.set("status", 400);
            } else {
                ext.set("code", "internal");
                ext.set("status", 500);
            }
        }
        if ext.get("requestId").is_none() {
            ext.set("requestId", request_id);
        }
    }
    query_level_only && matches!(response.data, Value::Null)
}

/// Führt einen GraphQL-Request aus (nach erfolgreichem JSON-Parsing).
pub async fn execute(
    schema: &LinealSchema,
    auth: &AuthState,
    rate: Option<RateHandle>,
    mut request: Request,
) -> Response {
    let ctx = match auth.require() {
        Ok(ctx) => ctx.clone(),
        Err(e) => return graphql_error_response(&e),
    };
    let (is_mutation, cost) = {
        let name = request.operation_name.clone();
        // Parse-Fehler: Ausführung liefert die ordentliche Fehlerantwort (400, invalid_request).
        let parsed = request.parsed_query().ok().cloned();
        let Some(doc) = parsed else {
            return run(schema, request, &ctx.request_id, None, false).await;
        };
        match operation(&doc, name.as_deref()) {
            Some(op) => (
                op.ty == OperationType::Mutation,
                complexity(&doc, &op.selection_set.node, &request),
            ),
            None => return run(schema, request, &ctx.request_id, None, false).await,
        }
    };
    if is_mutation && !ctx.scopes.write {
        return graphql_error_response(&AppError::forbidden("Token has no write scope."));
    }
    let max = rate.as_ref().map_or(10_000, |r| r.max_query_complexity);
    if cost > max {
        return graphql_error_response(
            &AppError::invalid_request(format!(
                "Query complexity {cost} exceeds the maximum {max}."
            ))
            .with_details(json!({ "complexity": cost, "max": max })),
        );
    }
    let mut decision = None;
    if let Some(r) = &rate {
        let d = r.take_complexity(cost);
        if !d.allowed {
            return graphql_error_response(&denied_error(&d, "complexity"));
        }
        decision = Some((d, cost));
    }
    // Art der Anmeldung für Resolver, die nur per Cookie-Session erlaubt sind (API-Keys).
    let class = match auth {
        AuthState::Authenticated { class, .. } => *class,
        _ => CredentialClass::Anonymous,
    };
    run(
        schema,
        request.data(ctx.clone()).data(class),
        &ctx.request_id,
        decision,
        !is_mutation,
    )
    .await
}

async fn run(
    schema: &LinealSchema,
    request: Request,
    request_id: &str,
    decision: Option<(crate::rate_limit::Decision, u64)>,
    cap_response: bool,
) -> Response {
    // Frisches Request-Budget für `Issue.children/relations/history` (nie wiederverwendet).
    let mut response = schema
        .execute(request.data(crate::context::DetailBudget::default()))
        .await;
    let bad_request = finish_errors(&mut response, request_id);
    let status = if bad_request {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };
    let mut res = match serde_json::to_vec(&response) {
        Ok(bytes) if cap_response && bytes.len() > MAX_QUERY_RESPONSE_BYTES => {
            return graphql_error_response(
                &AppError::limit_exceeded(format!(
                    "Response exceeds the maximum of {MAX_QUERY_RESPONSE_BYTES} bytes; \
                     request fewer items or fewer text fields."
                ))
                .with_details(json!({ "maxBytes": MAX_QUERY_RESPONSE_BYTES })),
            );
        }
        Ok(bytes) => (
            status,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            bytes,
        )
            .into_response(),
        Err(_) => (status, Json(response)).into_response(),
    };
    if let Some((d, cost)) = decision {
        complexity_headers(res.headers_mut(), &d, cost);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_graphql::parser::parse_query;

    fn cost_of(query: &str, variables: serde_json::Value) -> u64 {
        let request = Request::new(query).variables(async_graphql::Variables::from_json(variables));
        let doc = parse_query(query).unwrap();
        let op = operation(&doc, None).unwrap();
        complexity(&doc, &op.selection_set.node, &request)
    }

    #[test]
    fn history_and_children_are_expensive_fields() {
        let plain = cost_of("{ issues(first: 100) { nodes { identifier } } }", json!({}));
        // `history` zählt mit Faktor 100 je Issue, `children` ohne `first` wie eine Connection (50).
        let history = cost_of(
            "{ issues(first: 100) { nodes { identifier history { source } } } }",
            json!({}),
        );
        assert!(history >= plain + 1_000, "{history} vs {plain}");
        let children = cost_of(
            "{ issues(first: 100) { nodes { children { identifier } } } }",
            json!({}),
        );
        assert!(children >= 500, "{children}");
        // Verschachtelung multipliziert: Listen mit Kindern mit Kindern sprengen jedes Limit.
        let nested = cost_of(
            "{ issues(first: 100) { nodes { children { children { history { source } } } } } }",
            json!({}),
        );
        assert!(nested > 10_000, "{nested}");
    }

    #[test]
    fn nested_fragment_fanout_is_linear_not_exponential() {
        // 40 Ebenen, jede referenziert die nächste zweimal: ohne Memo 2^40 Schritte.
        let mut q = String::from("query { ...F0 }\n");
        for i in 0..40 {
            q.push_str(&format!(
                "fragment F{i} on Query {{ ...F{n} ...F{n} }}\n",
                n = i + 1
            ));
        }
        q.push_str("fragment F40 on Query { a }\n");
        let start = std::time::Instant::now();
        let cost = cost_of(&q, serde_json::json!({}));
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        assert!(cost > 0);
    }

    #[test]
    fn text_fields_and_batch_ids_are_weighted() {
        // F-09/DOS-01: 8 Aliase mit 250 Beschreibungen sprengen das Budget (vorher ~400 Punkte).
        let alias = "{ issues(first: 250) { nodes { description } } }";
        let one = cost_of(alias, json!({}));
        assert!(one > 2_000, "{one}");
        let eight = (0..8)
            .map(|i| format!("a{i}: issues(first: 250) {{ nodes {{ description }} }}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(cost_of(&format!("{{ {eight} }}"), json!({})) > 10_000);
        // Schlanke Felder bleiben billig.
        assert!(cost_of("{ issues(first: 250) { nodes { identifier } } }", json!({})) < 100);
        // DOS-03: Batch-Größe zählt (Literal und Variable); 4 Aliase x 250 Ids > Budget.
        let ids: Vec<String> = (0..250).map(|i| format!("{i}")).collect();
        let q = "mutation($i: IssueBatchUpdateInput!) { issueBatchUpdate(input: $i) { updated } }";
        let c = cost_of(q, json!({ "i": { "ids": ids, "priority": 1 } }));
        assert!(c >= 2_500, "{c}");
        let q4 = "mutation($i: IssueBatchUpdateInput!) { a: issueBatchUpdate(input: $i) { updated } \
                  b: issueBatchUpdate(input: $i) { updated } c: issueBatchUpdate(input: $i) { updated } \
                  d: issueBatchUpdate(input: $i) { updated } }";
        assert!(cost_of(q4, json!({ "i": { "ids": ids, "priority": 1 } })) > 10_000);
        let few = cost_of(q, json!({ "i": { "ids": ["a", "b"], "priority": 1 } }));
        assert!(few < 100, "{few}");
    }

    #[test]
    fn plain_lists_and_more_text_fields_are_weighted() {
        // Einfache Listen ohne first: Faktor je Zeile statt einmal.
        let cycles = cost_of("{ cycles(teamId: \"t\") { description } }", json!({}));
        assert!(cycles >= 500, "{cycles}");
        let ms = cost_of(
            "{ projectMilestones(projectId: \"p\") { description } }",
            json!({}),
        );
        assert!(ms >= 50, "{ms}");
        // Alias zählt nach Feldname, nicht nach Alias.
        let aliased = cost_of("{ x: issue(id: \"a\") { d: description } }", json!({}));
        assert!(aliased >= 10, "{aliased}");
        for f in ["markdown", "summary", "templateData", "quotedText", "body"] {
            let c = cost_of(
                &format!("{{ issues(first: 250) {{ nodes {{ {f} }} }} }}"),
                json!({}),
            );
            assert!(c > 2_000, "{f}: {c}");
        }
        // Die Web-Projektliste (PROJECT_FIELDS, 100 je Seite) bleibt klar im Budget.
        let q = "{ projectsPage(first: 100) { nodes { id identifier slugId name summary description icon \
                 status { id name type color } health progress leadTeam { id name key } \
                 teams { id name key } lead { id name key } members { id name key } \
                 labels { id name key } initiatives { id name key } \
                 milestones { id name description targetDate sortOrder status progress } \
                 createdAt } }";
        let ui = cost_of(&format!("{q} }}"), json!({}));
        assert!(ui < 10_000, "{ui}");
    }

    #[test]
    fn complexity_matches_spec_example() {
        // 1 + 50 * (0,1 + 0,1 + 1 + 0,1) = 66 (09 §3.2)
        let q = "{ issues(first: 50) { nodes { id title assignee { name } } } }";
        assert_eq!(cost_of(q, json!({})), 66);
    }

    #[test]
    fn complexity_uses_variables_default_page_and_fragments() {
        let q =
            "query($n: Int) { issues(first: $n) { nodes { ...F } } } fragment F on Issue { id }";
        assert_eq!(cost_of(q, json!({ "n": 10 })), 2); // 1 + 10 * 0,1
        let q = "{ issues { nodes { id } } }";
        assert_eq!(cost_of(q, json!({})), 6); // 1 + 50 * 0,1
        assert_eq!(cost_of("{ __typename }", json!({})), 1); // 0,1 aufgerundet
    }
}
