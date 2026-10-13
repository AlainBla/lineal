//! Paket 8 (B8.17): GraphQL-Body als JSON-Array (Batch) ist `400 invalid_request` ohne interne
//! Serde-Meldung. Keine Datenbank (FakeStore).

#![allow(clippy::unwrap_used)]

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use lineal_api::middleware::auth::hash_token;
use lineal_app::ports::repos::{CredentialKind, Principal};
use lineal_app::{Actor, Scopes};
use lineal_domain::ids::{OrgId, UserId};
use lineal_testkit::{FakeStore, test_app};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

const KEY: &str = "lin_api_alice_readwrite";

#[tokio::test]
async fn batch_array_body_is_400_invalid_request() {
    let store = FakeStore::new().with_principal(
        hash_token(KEY),
        Principal {
            kind: CredentialKind::ApiKey,
            org_id: OrgId::new_v7(),
            actor: Actor::App {
                id: UserId::new_v7(),
            },
            scopes: Scopes::read_write(),
            team_scope: None,
        },
    );
    let router = lineal_api::router(test_app(Arc::new(store)), false);
    let req = Request::builder()
        .method(Method::POST)
        .uri("/graphql")
        .header("content-type", "application/json")
        .header("authorization", KEY)
        .body(Body::from(
            json!([{ "query": "{ __typename }" }]).to_string(),
        ))
        .unwrap();
    let res = router.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["errors"][0]["extensions"]["code"], "invalid_request");
    let message = body["errors"][0]["message"].as_str().unwrap();
    assert!(!message.contains("invalid type"), "{message}");
}
