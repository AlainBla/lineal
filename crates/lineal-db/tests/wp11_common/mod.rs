//! Gemeinsame Testhilfen für WP11 (Views, Filter, Suche, Übersicht): Seed `acme` plus eine
//! feste Uhr (2026-10-10 07:15 UTC) und direkte Inserts für Issues und Verknüpfungen.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use chrono::{DateTime, TimeZone, Utc};
use lineal_app::ports::{Clock, Store};
use lineal_app::{App, ErrorKind, RequestCtx};
use lineal_db::PgStore;
use lineal_domain::ids::TeamId;
use lineal_testkit::seed::identity::seed_workflow_state_ids;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

pub struct FixedAt(pub DateTime<Utc>);

impl Clock for FixedAt {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

pub struct Fx {
    pub app: Arc<App>,
    pub data: SeedData,
    pub pool: PgPool,
}

pub fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 10, 7, 15, 0).unwrap()
}

pub async fn setup(pool: PgPool) -> Fx {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.clock = Arc::new(FixedAt(now()));
    Fx {
        app: Arc::new(app),
        data,
        pool,
    }
}

pub fn alice() -> RequestCtx {
    ctx_for(SeedUser::Alice)
}
pub fn bob() -> RequestCtx {
    ctx_for(SeedUser::Bob)
}
pub fn carol() -> RequestCtx {
    ctx_for(SeedUser::Carol)
}

pub fn kind<T>(r: Result<T, lineal_app::AppError>) -> ErrorKind {
    r.err().expect("expected an error").kind
}

/// Workspace-Label `Bug` (0), `Feature` (1), `Improvement` (2) aus dem Seed.
pub fn label(i: u128) -> Uuid {
    Uuid::from_u128(0x1ABE_0000_0000_7000_8000_0000_0000_0000 + i)
}

pub fn ts(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

impl Fx {
    /// Team-Slot (1 = ENG, 2 = WEB, 3 = DES) für die Status-IDs.
    pub fn slot(&self, team: TeamId) -> u128 {
        if team == self.data.eng {
            1
        } else if team == self.data.web {
            2
        } else {
            3
        }
    }

    /// Legt ein Issue direkt an (Nummer explizit, erste Nummern ≥ 100 vermeiden Seed-Kollisionen).
    /// `state` ist der Index in `Backlog, Todo, In Progress, Done, Canceled, Duplicate`.
    pub async fn issue(&self, team: TeamId, number: i32, title: &str, state: usize) -> Uuid {
        let id = Uuid::now_v7();
        let state_id = seed_workflow_state_ids(self.slot(team))[state].0;
        sqlx::query(
            "INSERT INTO issue (id, organization_id, team_id, number, title, state_id, creator_id, \
             created_at, updated_at, state_entered_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,'2026-10-09T09:00:00Z','2026-10-09T09:00:00Z','2026-10-01T09:00:00Z')",
        )
        .bind(id)
        .bind(self.data.org.0)
        .bind(team.0)
        .bind(number)
        .bind(title)
        .bind(state_id)
        .bind(self.data.alice.0)
        .execute(&self.pool)
        .await
        .unwrap();
        id
    }

    /// Setzt Felder eines Issues mit einem SQL-Fragment (`SET`-Liste, nur Testdaten).
    pub async fn set(&self, issue: Uuid, set_clause: &str) {
        sqlx::query(&format!("UPDATE issue SET {set_clause} WHERE id = $1"))
            .bind(issue)
            .execute(&self.pool)
            .await
            .unwrap();
    }

    pub async fn add_label(&self, issue: Uuid, label: Uuid) {
        sqlx::query(
            "INSERT INTO issue_to_label (organization_id, issue_id, label_id) VALUES ($1,$2,$3)",
        )
        .bind(self.data.org.0)
        .bind(issue)
        .bind(label)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    pub async fn relate(&self, from: Uuid, to: Uuid, kind: &str) {
        sqlx::query(
            "INSERT INTO issue_relation (organization_id, issue_id, related_issue_id, type) VALUES ($1,$2,$3,$4)",
        )
        .bind(self.data.org.0)
        .bind(from)
        .bind(to)
        .bind(kind)
        .execute(&self.pool)
        .await
        .unwrap();
    }
}
