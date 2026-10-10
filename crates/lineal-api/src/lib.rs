//! HTTP-Adapter: axum-Router, GraphQL, REST- und OAuth-Endpunkte.
//! Die Pfade `/mcp` und `/sync` liefern `lineal-mcp` und `lineal-sync`; `lineal-server` fügt
//! alle Router zusammen (docs/11 §2: Adapter kennen einander nicht).
#![forbid(unsafe_code)]

pub mod blob;
pub mod client_ip;
pub mod context;
pub mod error;
pub mod graphql;
pub mod middleware;
pub mod oauth;
pub mod pagination;
pub mod rate_limit;
pub mod rest;
pub mod router;
pub mod security_headers;

pub use router::{router, routes};
