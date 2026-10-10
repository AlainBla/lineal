//! `POST /graphql`-Pipeline (docs/09 §1.2, §3.2): Auth -> Parse -> Scope -> Komplexität ->
//! Rate-Limit -> Ausführung mit `RequestCtx` in den Context-Daten -> Fehler mit `extensions.code`.

use crate::error::graphql_error_response;
use crate::graphql::LinealSchema;
use crate::middleware::auth::AuthState;
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
    let mut stack = Vec::new();
    selection_cost(doc, operation, request, &mut stack).div_ceil(10)
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
    field
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
    stack: &mut Vec<Name>,
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
                    total = total.saturating_add(selection_cost(doc, children, request, stack));
                } else if is_connection(field) {
                    let n = int_arg(field, "first", request)
                        .or_else(|| int_arg(field, "last", request))
                        .unwrap_or(DEFAULT_CONNECTION_SIZE);
                    let inner = selection_cost(doc, children, request, stack);
                    total = total.saturating_add(10u64.saturating_add(n.saturating_mul(inner)));
                } else {
                    total =
                        total.saturating_add(10 + selection_cost(doc, children, request, stack));
                }
            }
            Selection::InlineFragment(f) => {
                total = total.saturating_add(selection_cost(
                    doc,
                    &f.node.selection_set.node,
                    request,
                    stack,
                ));
            }
            Selection::FragmentSpread(s) => {
                let name = &s.node.fragment_name.node;
                // Zyklen sind ungültig (die Validierung meldet sie); hier nur nicht rekursieren.
                if stack.contains(name) || stack.len() > MAX_FRAGMENT_DEPTH {
                    continue;
                }
                if let Some(frag) = doc.fragments.get(name) {
                    stack.push(name.clone());
                    total = total.saturating_add(selection_cost(
                        doc,
                        &frag.node.selection_set.node,
                        request,
                        stack,
                    ));
                    stack.pop();
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

/// Antwort auf einen unlesbaren Request-Body (Fehlermodell, Status der Rejection).
pub fn bad_body(rejection: &JsonRejection) -> Response {
    let mut res = graphql_error_response(&AppError::invalid_request(rejection.body_text()));
    *res.status_mut() = rejection.status();
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
            if e.path.is_empty() {
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
    run(schema, request.data(ctx.clone()), &ctx.request_id, decision).await
}

async fn run(
    schema: &LinealSchema,
    request: Request,
    request_id: &str,
    decision: Option<(crate::rate_limit::Decision, u64)>,
) -> Response {
    let mut response = schema.execute(request).await;
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
