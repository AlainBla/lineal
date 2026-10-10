#![allow(clippy::unwrap_used, clippy::expect_used)]
//! GraphQL `users`: sichtbare User ohne E-Mail-Adressen (Assignee-Picker, Erwähnungen).

use async_graphql::Request;
use lineal_api::graphql::build_schema;
use lineal_app::ports::Store;
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use std::sync::Arc;

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn users_lists_visible_users_without_email(pool: sqlx::PgPool) {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let schema = build_schema(test_app(Arc::new(store)));

    let res = schema
        .execute(Request::new("{ users { id name displayName } }").data(ctx_for(SeedUser::Bob)))
        .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let v = res.data.into_json().unwrap();
    let names: Vec<_> = v["users"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["displayName"].as_str().unwrap().to_owned())
        .collect();
    assert!(names.contains(&"alice".to_owned()), "{names:?}");

    // Das Schema bietet kein E-Mail-Feld an.
    let res = schema
        .execute(Request::new("{ users { email } }").data(ctx_for(SeedUser::Bob)))
        .await;
    assert!(!res.errors.is_empty());

    // Ungültige Seitengröße
    let res = schema
        .execute(Request::new("{ users(first: 0) { id } }").data(ctx_for(SeedUser::Bob)))
        .await;
    assert!(!res.errors.is_empty());
}
