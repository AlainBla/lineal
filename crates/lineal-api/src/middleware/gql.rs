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
                if children.items.is_empty() {
                    total += 1;
                } else if matches!(name, "nodes" | "edges" | "node") {
                    total = total.saturating_add(selection_cost(doc, children, request, walk));
                } else if is_connection(field) {
                    let n = int_arg(field, "first", request)
                        .or_else(|| int_arg(field, "last", request))
                        .unwrap_or(DEFAULT_CONNECTION_SIZE);
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

/// Antwort auf einen unlesbaren Request-Body (Fehlermodell). Daten-/Syntaxfehler (z. B. ein
/// JSON-Array als Batch) sind `400 invalid_request` ohne interne Serde-Meldung (B8.17).
pub fn bad_body(rejection: &JsonRejection) -> Response {
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
            return run(schema, request, &ctx.request_id, None).await;
        };
        match operation(&doc, name.as_deref()) {
            Some(op) => (
                op.ty == OperationType::Mutation,
                complexity(&doc, &op.selection_set.node, &request),
            ),
            None => return run(schema, request, &ctx.request_id, None).await,
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
    )
    .await
}

async fn run(
    schema: &LinealSchema,
    request: Request,
    request_id: &str,
    decision: Option<(crate::rate_limit::Decision, u64)>,
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
    let mut res = (status, Json(response)).into_response();
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
