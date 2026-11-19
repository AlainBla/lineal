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
                ..Default::default()
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
                        ..Default::default()
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

    // Update und eine Reaktion direkt am Update tragen ebenfalls das Lead-Team.
    let update = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO initiative_update (id, organization_id, initiative_id, user_id, body) \
         VALUES ($1, $2, $3, $4, 'Stand')",
    )
    .bind(update)
    .bind(env.data.org.0)
    .bind(private)
    .bind(env.data.bob.0)
    .execute(&env.pool)
    .await
    .unwrap();
    let reaction = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO reaction (id, organization_id, emoji, user_id, initiative_update_id) \
         VALUES ($1, $2, '👍', $3, $4)",
    )
    .bind(reaction)
    .bind(env.data.org.0)
    .bind(env.data.bob.0)
    .bind(update)
    .execute(&env.pool)
    .await
    .unwrap();
    assert_eq!(
        groups(&env, "InitiativeUpdate", update).await,
        vec![format!("iteam:{}", env.data.des.0)],
        "Update einer privaten Initiative"
    );
    assert_eq!(
        groups(&env, "Reaction", reaction).await,
        vec![format!("iteam:{}", env.data.des.0)],
        "Reaktion am Update einer privaten Initiative"
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
                ..Default::default()
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

// --- N14: Papierkorb kaskadiert Dokumente, Verknüpfen prüft Archiv, Zyklusfehler maskiert ------

/// Hängt ein Dokument an die Initiative (per SQL, umgeht Authz).
async fn insert_initiative_document(env: &Env, initiative: Uuid) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO document (id, organization_id, title, initiative_id, creator_id) \
         VALUES ($1,$2,'Spezifikation',$3,$4)",
    )
    .bind(id)
    .bind(env.data.org.0)
    .bind(initiative)
    .bind(env.data.bob.0)
    .execute(&env.pool)
    .await
    .unwrap();
    id
}

/// `(trashed, trashed_with_parent)` eines Dokuments.
async fn doc_trashed(env: &Env, doc: Uuid) -> (bool, bool) {
    sqlx::query_as::<_, (bool, bool)>(
        "SELECT trashed, trashed_with_parent FROM document WHERE id = $1",
    )
    .bind(doc)
    .fetch_one(&env.pool)
    .await
    .unwrap()
}

/// N14: Eine getrashte Initiative nimmt ihre Dokumente mit in den Papierkorb (wie Projekte);
/// Wiederherstellen holt die mitgelöschten Dokumente zurück.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn trashing_initiative_cascades_documents(pool: PgPool) {
    let env = setup(pool).await;
    let init = insert_initiative(&env, "Mit Doc", Some(env.data.des)).await;
    let doc = insert_initiative_document(&env, init).await;
    let bob = ctx_for(SeedUser::Bob);

    assert_eq!(doc_trashed(&env, doc).await, (false, false));

    env.app
        .trash_initiative(&bob, &init.to_string())
        .await
        .unwrap();
    assert_eq!(
        doc_trashed(&env, doc).await,
        (true, true),
        "Dokument muss mit der Initiative in den Papierkorb"
    );

    env.app
        .restore_initiative(&bob, &init.to_string())
        .await
        .unwrap();
    assert_eq!(
        doc_trashed(&env, doc).await,
        (false, false),
        "Wiederherstellen holt das mitgelöschte Dokument zurück"
    );
}

/// N14: Ein Projekt an eine archivierte (read-only) Initiative zu knüpfen ist unzulässig; die
/// Archivprüfung greift vor dem Auflösen des Projekts.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn linking_project_to_archived_initiative_is_rejected(pool: PgPool) {
    let env = setup(pool).await;
    // Workspace-weite Initiative (kein Lead-Team) -> für die Owner-Alice sichtbar.
    let init = insert_initiative(&env, "Archiviert", None).await;
    // Seed-Projekt (Team ENG, Alice ist Mitglied).
    let project = Uuid::from_u128(0x9201_EC70_0000_7000_8000_0000_0000_0001);
    let alice = ctx_for(SeedUser::Alice);

    env.app
        .set_initiative_archived(&alice, &init.to_string(), true)
        .await
        .unwrap();

    assert_eq!(
        kind(
            env.app
                .set_initiative_project(&alice, &init.to_string(), &project.to_string(), true)
                .await
        ),
        ErrorKind::InvalidRequest,
        "archivierte Initiative ist read-only"
    );
}

/// N14: Enthält ein erkannter Zyklus eine für den Aufrufer unsichtbare Initiative (privates
/// Lead-Team), darf der Fehler deren Kennung `I-n` nicht verraten.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn cycle_error_masks_invisible_initiative(pool: PgPool) {
    let env = setup(pool).await;
    // A und B sind workspace-weit (für Alice sichtbar); X hat privates Lead-Team DES (unsichtbar).
    let a = insert_initiative(&env, "Zyklus A", None).await;
    let x = insert_initiative(&env, "Zyklus X geheim", Some(env.data.des)).await;
    let b = insert_initiative(&env, "Zyklus B", None).await;

    // Kette B -> X -> A (Kante = (parent, child)).
    let mut tx = env.app.store.begin(env.data.org).await.unwrap();
    tx.initiatives()
        .insert_edge(InitiativeId(b), InitiativeId(x), 1.0, now())
        .await
        .unwrap();
    tx.initiatives()
        .insert_edge(InitiativeId(x), InitiativeId(a), 1.0, now())
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Kennung von X ermitteln (für die Negativ-Assertion).
    let x_number: i32 = sqlx::query_scalar("SELECT number FROM initiative WHERE id = $1")
        .bind(x)
        .fetch_one(&env.pool)
        .await
        .unwrap();
    let x_ident = format!("I-{x_number}");

    // Alice fügt A als Elternteil von B hinzu -> Zyklus A -> B -> X -> A.
    let alice = ctx_for(SeedUser::Alice);
    let err = env
        .app
        .save_initiative(
            &alice,
            SaveInitiativeInput {
                id: Some(b.to_string()),
                parent_initiatives: vec![a.to_string()],
                ..Default::default()
            },
        )
        .await
        .expect_err("Zyklus muss abgelehnt werden");

    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(err.details["reason"], "cycle");
    let cycle = err.details["cycle"].to_string();
    assert!(
        !cycle.contains(&x_ident) && !err.message.contains(&x_ident),
        "unsichtbare Initiative {x_ident} darf nicht erscheinen: {cycle} / {}",
        err.message
    );
    assert!(
        cycle.contains("a hidden initiative"),
        "unsichtbarer Knoten muss maskiert sein: {cycle}"
    );
}
