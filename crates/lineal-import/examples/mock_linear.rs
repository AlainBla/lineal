#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Mock-Linear-Server für die E2E-Tests (`scripts/e2e.sh`): beantwortet die GraphQL-Abfragen des
//! Linear-Adapters aus den erfundenen Fixture-Daten (`fixtures::acceptance_workspace`) auf
//! `127.0.0.1:$MOCK_LINEAR_PORT`. Er kontaktiert nie ein Fremdsystem; der Worker bekommt den
//! Endpunkt über `LINEAL_IMPORT_LINEAR_ENDPOINT` (nur mit `LINEAL_DEV=1`).

#[path = "../tests/common/mock_linear.rs"]
mod mock_linear;

use lineal_import::{FakeSource, fixtures};
use mock_linear::MockLinear;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("MOCK_LINEAR_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(18950);
    let source = Arc::new(FakeSource::from_json(&fixtures::acceptance_workspace()).unwrap());
    let (_mock, endpoint) = MockLinear::start_on(source, &format!("127.0.0.1:{port}")).await;
    println!("mock linear listening on {endpoint}");
    std::future::pending::<()>().await;
}
