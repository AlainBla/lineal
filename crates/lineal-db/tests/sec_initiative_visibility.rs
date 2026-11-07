#![allow(clippy::unwrap_used, clippy::expect_used)]
//! LIN-32 (S-03, S-04, S-16): Eine Initiative mit privatem Lead-Team ist nur für Mitglieder
//! dieses Teams sichtbar (docs/04 §4.6). Geprüft werden Kommentare, Sync-Gruppen und die
//! Rechteprüfung beim Entfernen des Lead-Teams. Seed `acme`: DES ist privat, nur Bob ist
//! Mitglied; Alice (Owner) ist kein Mitglied, Carol ist Gast.

use chrono::{DateTime, Utc};
use lineal_app::ports::Store;
use lineal_app::ports::repos::initiative::NewInitiative;
use lineal_app::ports::repos::user::OrderBy;
use lineal_app::usecases::comment::{CreateCommentInput, ListCommentsInput, ParentSpec};
use lineal_app::usecases::initiative::SaveInitiativeInput;
use lineal_app::usecases::webhook::CreateWebhookInput;
use lineal_app::{App, ErrorKind};
use lineal_db::PgStore;
use lineal_domain::enums::InitiativeStatus;
use lineal_domain::ids::InitiativeId;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

struct Env {
    app: App,
    data: SeedData,
    pool: PgPool,
}

async fn setup(pool: PgPool) -> Env {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    Env { app, data, pool }
}

fn now() -> DateTime<Utc> {
    "2026-10-10T08:00:00Z".parse().unwrap()
}

/// Legt direkt eine Initiative mit dem gegebenen Lead-Team an und liefert ihre ID.
async fn insert_initiative(
    env: &Env,
    name: &str,
    lead: Option<lineal_domain::ids::TeamId>,
) -> Uuid {
    let mut tx = env.app.store.begin(env.data.org).await.unwrap();
    let n = tx.initiatives().next_number().await.unwrap();
    let id = InitiativeId(Uuid::now_v7());
    tx.initiatives()
        .insert(&NewInitiative {
            id,
            number: n,
            name: name.into(),
            description: None,
            content: None,
            color: None,
            icon: None,
            status: InitiativeStatus::Active,
            owner_id: Some(env.data.bob),
            creator_id: Some(env.data.bob),
            lead_team_id: lead,
            priority: 0,
            sort_order: 1000.0,
            target_date: None,
            target_date_resolution: None,
            started_at: Some(now()),
            completed_at: None,
            now: now(),
        })
        .await
        .unwrap();
    tx.commit().await.unwrap();
    id.0
}

fn kind<T>(r: Result<T, lineal_app::AppError>) -> ErrorKind {
    r.err().expect("expected an error").kind
}

// --- S-03: Kommentare an einer privaten Initiative ------------------------------------------

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn comments_on_private_initiative_follow_lead_team(pool: PgPool) {
    let env = setup(pool).await;
    let init = insert_initiative(&env, "Geheim", Some(env.data.des)).await;

    // Bob (DES-Mitglied) darf kommentieren und lesen.
    let bob = ctx_for(SeedUser::Bob);
    env.app
        .create_comment(
            &bob,
            CreateCommentInput {
                parent: Some(ParentSpec::Initiative(init.to_string())),
                reply_to: None,
                body: "intern".into(),
                quoted_text: None,
            },
        )
        .await
        .expect("bob kann kommentieren");
    let list = env
        .app
        .list_comments(
            &bob,
            ListCommentsInput {
                parent: ParentSpec::Initiative(init.to_string()),
                order_by: OrderBy::CreatedAt,
                cursor: None,
                limit: 50,
            },
        )
        .await
        .expect("bob kann lesen");
    assert_eq!(list.items.len(), 1);

    // Alice (Owner, kein DES-Mitglied): die Initiative existiert für sie nicht.
    let alice = ctx_for(SeedUser::Alice);
    assert_eq!(
        kind(
            env.app
                .list_comments(
                    &alice,
                    ListCommentsInput {
                        parent: ParentSpec::Initiative(init.to_string()),
                        order_by: OrderBy::CreatedAt,
                        cursor: None,
                        limit: 50,
                    },
                )
                .await
        ),
        ErrorKind::NotFound,
        "alice list"
    );
    assert_eq!(
        kind(
            env.app
                .create_comment(
                    &alice,
                    CreateCommentInput {
                        parent: Some(ParentSpec::Initiative(init.to_string())),
                        reply_to: None,
                        body: "x".into(),
                        quoted_text: None,
                    },
                )
                .await
        ),
        ErrorKind::NotFound,
        "alice create"
    );

    // Carol (Gast) sieht Initiativen grundsätzlich nicht.
    let carol = ctx_for(SeedUser::Carol);
    assert_eq!(
        kind(
            env.app
                .list_comments(
                    &carol,
                    ListCommentsInput {
                        parent: ParentSpec::Initiative(init.to_string()),
                        order_by: OrderBy::CreatedAt,
                        cursor: None,
                        limit: 50,
                    },
                )
                .await
        ),
        ErrorKind::Forbidden,
        "carol list"
    );
}

// --- S-04: Sync-Gruppen ----------------------------------------------------------------------

async fn groups(env: &Env, model: &str, id: Uuid) -> Vec<String> {
    let mut tx = env.app.store.begin(env.data.org).await.unwrap();
    let st = tx.outbox().sync_model(model, id).await.unwrap().unwrap();
    st.groups
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn sync_groups_respect_lead_team(pool: PgPool) {
    let env = setup(pool).await;
    let private = insert_initiative(&env, "Geheim", Some(env.data.des)).await;
    let public = insert_initiative(&env, "Offen", None).await;

    assert_eq!(
        groups(&env, "Initiative", private).await,
        vec![format!("iteam:{}", env.data.des.0)],
        "private initiative: nur Nicht-Gast-Mitglieder des Lead-Teams"
    );
    assert_eq!(
        groups(&env, "Initiative", public).await,
        vec!["org".to_string()],
        "öffentliche Initiative: workspace-weit (ohne Gäste)"
    );
}

// --- S-16: Webhooks ---------------------------------------------------------------------------

/// Schreibt alle offenen Events als Webhook-Zustellungen und liefert deren Anzahl je Hook.
async fn deliveries_for(env: &Env, hook: Uuid) -> i64 {
    let mut tx = env.app.store.begin_system().await.unwrap();
    let evs = tx
        .outbox()
        .claim_undelivered("sec-test", 10_000)
        .await
        .unwrap();
    drop(tx);
    for ev in &evs {
        env.app.plan_webhook_deliveries(ev).await.unwrap();
    }
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM webhook_delivery WHERE webhook_id = $1")
        .bind(hook)
        .fetch_one(&env.pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn private_initiative_events_skip_all_public_teams_hooks(pool: PgPool) {
    let env = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);

    // Workspace-Hook (allPublicTeams) auf Initiative-Events; Alice (Owner) darf ihn anlegen.
    let hook = env
        .app
        .create_webhook(
            &alice,
            CreateWebhookInput {
                label: Some("CI".into()),
                url: "https://93.184.216.34/hook".into(),
                team: None,
                all_public_teams: true,
                resource_types: vec!["Initiative".into()],
            },
        )
        .await
        .expect("hook")
        .webhook
        .id;

    // Private Initiative (Lead DES) von Bob angelegt: Das Event darf den Workspace-Hook NICHT
    // erreichen (docs/09 §4.1: niemals Events privater Teams an allPublicTeams).
    let bob = ctx_for(SeedUser::Bob);
    env.app
        .save_initiative(
            &bob,
            SaveInitiativeInput {
                name: Some("Geheim".into()),
                lead_team: Some(Some("DES".into())),
                ..Default::default()
            },
        )
        .await
        .expect("bob legt private Initiative an");
    assert_eq!(
        deliveries_for(&env, hook.0).await,
        0,
        "privat → kein allPublicTeams"
    );

    // Öffentliche Initiative (ohne Lead-Team): workspace-weit, der Hook erhält sie.
    env.app
        .save_initiative(
            &alice,
            SaveInitiativeInput {
                name: Some("Offen".into()),
                ..Default::default()
            },
        )
        .await
        .expect("alice legt öffentliche Initiative an");
    assert_eq!(
        deliveries_for(&env, hook.0).await,
        1,
        "öffentlich → zugestellt"
    );
}

// --- S-16: Lead-Team entfernen öffnet die Initiative -----------------------------------------

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn clearing_lead_team_requires_permission(pool: PgPool) {
    let env = setup(pool).await;
    let init = insert_initiative(&env, "Geheim", Some(env.data.des)).await;

    // Workspace verlangt für workspace-weite Initiativen (ohne Lead-Team) Admin-Rechte.
    sqlx::query(
        "UPDATE organization SET security_settings = \
         security_settings || '{\"workspaceInitiativesRole\":\"admin\"}' WHERE id = $1",
    )
    .bind(env.data.org.0)
    .execute(&env.pool)
    .await
    .unwrap();

    // Bob ist DES-Mitglied und darf die private Initiative sehen, aber eine workspace-weite
    // (ohne Lead-Team) zu öffnen verlangt das Admin-Recht, das er nicht hat (S-16, Parität zum
    // Anlegen).
    let bob = ctx_for(SeedUser::Bob);
    let res = env
        .app
        .save_initiative(
            &bob,
            SaveInitiativeInput {
                id: Some(init.to_string()),
                lead_team: Some(None),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(res), ErrorKind::Forbidden, "bob darf nicht öffnen");

    // Das Lead-Team bleibt unverändert.
    let lead: Option<Uuid> =
        sqlx::query_scalar("SELECT lead_team_id FROM initiative WHERE id = $1")
            .bind(init)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(lead, Some(env.data.des.0));
}
