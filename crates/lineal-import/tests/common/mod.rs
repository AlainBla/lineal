#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]
//! Gemeinsame Test-Helfer: frischer Workspace `acme` (ohne Seed-Teams), Admin Alice, Bob, Uhr.

use chrono::{DateTime, TimeZone, Utc};
use lineal_app::ports::Clock;
use lineal_app::ports::repos::import::ImportRun;
use lineal_app::usecases::import::{ImportScanInput, ImportStartInput};
use lineal_app::{Actor, App, RequestCtx, Scopes};
use lineal_db::PgStore;
use lineal_domain::enums::UserRole;
use lineal_domain::ids::{OrgId, UserId};
use lineal_import::blob::MemoryBlobSink;
use lineal_import::engine::{EngineConfig, ImportEngine, RunOutcome};
use lineal_import::fixtures;
use lineal_import::{FakeSource, ImportSource};
use lineal_testkit::test_app;
use sqlx::PgPool;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use uuid::Uuid;

pub struct V7Ids;

impl lineal_app::ports::IdGen for V7Ids {
    fn new_uuid(&self) -> Uuid {
        Uuid::now_v7()
    }
}

pub const TOKEN: &str = "lin_api_FAKETOKENabcdef0123456789";

pub struct TestClock(AtomicI64);

impl TestClock {
    pub fn at(t: DateTime<Utc>) -> Self {
        Self(AtomicI64::new(t.timestamp()))
    }
    pub fn set(&self, t: DateTime<Utc>) {
        self.0.store(t.timestamp(), Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_opt(self.0.load(Ordering::SeqCst), 0)
            .single()
            .unwrap()
    }
}

pub fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 10, 8, 0, 0)
        .single()
        .unwrap()
}

pub struct Env {
    pub app: Arc<App>,
    pub pool: PgPool,
    pub org: OrgId,
    pub alice: UserId,
    pub bob: UserId,
    pub clock: Arc<TestClock>,
    pub sink: Arc<MemoryBlobSink>,
}

impl Env {
    pub fn ctx(&self, user: UserId, role: UserRole) -> RequestCtx {
        RequestCtx {
            org_id: self.org,
            actor: Actor::User { id: user, role },
            request_id: "req_import_test".into(),
            scopes: Scopes::all(),
            team_scope: None,
        }
    }

    pub fn alice_ctx(&self) -> RequestCtx {
        self.ctx(self.alice, UserRole::Owner)
    }

    pub fn bob_ctx(&self) -> RequestCtx {
        self.ctx(self.bob, UserRole::User)
    }

    pub async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(&format!("SELECT ({sql})::bigint"))
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    /// Zeilenzahlen der wichtigsten Tabellen dieses Workspace.
    pub async fn snapshot(&self) -> Vec<(String, i64)> {
        let mut out = Vec::new();
        for t in [
            "team",
            "workflow_state",
            "issue_label",
            "issue",
            "issue_relation",
            "issue_history",
            "comment",
            "reaction",
            "attachment",
            "project",
            "project_milestone",
            "initiative",
            "cycle",
            "document",
            "project_update",
            "initiative_update",
            "issue_subscription",
            "issue_to_label",
            "issue_identifier_alias",
            "project_team",
            "team_membership",
            "\"user\"",
            "import_mapping",
        ] {
            let n: i64 = sqlx::query_scalar(&format!(
                "SELECT count(*) FROM {t} WHERE organization_id = $1"
            ))
            .bind(self.org.0)
            .fetch_one(&self.pool)
            .await
            .unwrap();
            out.push((t.to_owned(), n));
        }
        out
    }

    pub fn engine<'a>(
        &'a self,
        source: &'a dyn ImportSource,
        cfg: EngineConfig,
    ) -> ImportEngine<'a> {
        ImportEngine::new(&self.app, source, &*self.sink).with_config(cfg)
    }

    /// Scan -> Plan (Lauf im Status `planned`).
    pub async fn scan(&self, source: &dyn ImportSource) -> ImportRun {
        let run = self
            .app
            .import_scan(
                &self.alice_ctx(),
                ImportScanInput {
                    source: "linear".into(),
                    token: TOKEN.into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(run.status, "scanning");
        self.engine(source, EngineConfig::default())
            .scan(self.org, run.id)
            .await
            .unwrap();
        let planned = self
            .app
            .import_get_run(&self.alice_ctx(), run.id)
            .await
            .unwrap();
        assert_eq!(
            planned.status, "planned",
            "scan failed: {:?}",
            planned.error
        );
        planned
    }

    pub async fn start(&self, run: Uuid, input: ImportStartInput) -> ImportRun {
        self.app
            .import_start(
                &self.alice_ctx(),
                ImportStartInput {
                    run_id: run,
                    ..input
                },
            )
            .await
            .unwrap()
    }

    /// Scan + Start + Lauf mit Standardoptionen und kleiner Seitengröße.
    pub async fn import_all(
        &self,
        source: &dyn ImportSource,
        input: ImportStartInput,
    ) -> (Uuid, RunOutcome) {
        let planned = self.scan(source).await;
        self.start(planned.id, input).await;
        let out = self
            .engine(
                source,
                EngineConfig {
                    page_size: 40,
                    ..Default::default()
                },
            )
            .run(self.org, planned.id)
            .await
            .unwrap();
        (planned.id, out)
    }
}

pub async fn env(pool: PgPool) -> Env {
    env_n(pool, 1).await
}

/// Workspace Nr. `n` (mehrere Workspaces in einer Testdatenbank, z. B. für Vergleichsläufe).
pub async fn env_n(pool: PgPool, n: u128) -> Env {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("warn")
        .try_init();
    let org = OrgId(Uuid::from_u128(
        0xAC3E_0000_0000_7000_8000_00FF_0000_0000 + n,
    ));
    let alice = UserId(Uuid::from_u128(
        0xA11C_E000_0000_7000_8000_00FF_0000_0000 + n,
    ));
    let bob = UserId(Uuid::from_u128(
        0xB0B0_0000_0000_7000_8000_00FF_0000_0000 + n,
    ));
    sqlx::query("INSERT INTO organization (id, name, url_key) VALUES ($1, 'Acme', $2)")
        .bind(org.0)
        .bind(if n == 1 {
            "acme".to_owned()
        } else {
            format!("acme-{n}")
        })
        .execute(&pool)
        .await
        .unwrap();
    for (id, name, display, email, role) in [
        (alice, "Alice Admin", "alice", "alice@acme.test", "owner"),
        (bob, "Bob Builder", "bob", "BOB@acme.test", "user"),
    ] {
        sqlx::query(
            "INSERT INTO \"user\" (id, organization_id, name, display_name, email, role) VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(id.0)
        .bind(org.0)
        .bind(name)
        .bind(display)
        .bind(email)
        .bind(role)
        .execute(&pool)
        .await
        .unwrap();
    }
    let clock = Arc::new(TestClock::at(t0()));
    let mut app = Arc::try_unwrap(test_app(Arc::new(PgStore::new(pool.clone()))))
        .ok()
        .unwrap();
    app.clock = clock.clone();
    app.ids = Arc::new(V7Ids);
    Env {
        app: Arc::new(app),
        pool,
        org,
        alice,
        bob,
        clock,
        sink: Arc::new(MemoryBlobSink::default()),
    }
}

pub fn fake() -> FakeSource {
    FakeSource::from_json(&fixtures::acceptance_workspace()).unwrap()
}
