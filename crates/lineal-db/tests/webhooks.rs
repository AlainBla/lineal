#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP16: Webhooks (CRUD, Rechte, SSRF, Planung/Sichtbarkeit, Retry und Deaktivierung),
//! Einladungen und Team-Lebenszyklus gegen echte Datenbank (Seed `acme`).

use chrono::{DateTime, Duration, TimeZone, Utc};
use lineal_app::ports::repos::StoredEvent;
use lineal_app::ports::{Clock, Store};
use lineal_app::usecases::invitation::{AcceptInvitationInput, InviteUsersInput};
use lineal_app::usecases::issue::CreateIssueInput;
use lineal_app::usecases::project::SaveProjectInput;
use lineal_app::usecases::team::UpdateTeamInput;
use lineal_app::usecases::team_lifecycle::{OpenIssuesOnRetirement, SubTeamHandling};
use lineal_app::usecases::webhook::{CreateWebhookInput, UpdateWebhookInput};
use lineal_app::{App, ErrorKind, RequestCtx, Scopes};
use lineal_db::PgStore;
use lineal_domain::enums::UserRole;
use lineal_domain::events::EventOrigin;
use lineal_domain::ids::WebhookId;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::{Arc, Mutex};

struct TestClock(Mutex<DateTime<Utc>>);

impl TestClock {
    fn advance(&self, secs: i64) {
        let mut t = self.0.lock().unwrap();
        *t += Duration::seconds(secs);
    }
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

struct Fx {
    app: Arc<App>,
    data: SeedData,
    pool: PgPool,
    clock: Arc<TestClock>,
}

async fn setup(pool: PgPool) -> Fx {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let clock = Arc::new(TestClock(Mutex::new(
        Utc.with_ymd_and_hms(2026, 10, 10, 7, 15, 0).unwrap(),
    )));
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.clock = clock.clone();
    // Produktionsnahe SSRF-Regeln: https-Basis-URL = strenge Policy.
    app.config.base_url = "https://lineal.example".into();
    Fx {
        app: Arc::new(app),
        data,
        pool,
        clock,
    }
}

fn kind<T>(r: Result<T, lineal_app::AppError>) -> ErrorKind {
    r.err().expect("expected an error").kind
}

fn alice() -> RequestCtx {
    ctx_for(SeedUser::Alice)
}
fn bob() -> RequestCtx {
    ctx_for(SeedUser::Bob)
}
fn carol() -> RequestCtx {
    ctx_for(SeedUser::Carol)
}

const URL: &str = "https://93.184.216.34/hook";

fn ws_hook(types: &[&str]) -> CreateWebhookInput {
    CreateWebhookInput {
        label: Some("CI".into()),
        url: URL.into(),
        team: None,
        all_public_teams: true,
        resource_types: types.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn team_hook(team: &str, types: &[&str]) -> CreateWebhookInput {
    CreateWebhookInput {
        team: Some(team.into()),
        all_public_teams: false,
        ..ws_hook(types)
    }
}

/// Alle bisher geschriebenen Events des Workspace in Reihenfolge.
async fn events(fx: &Fx) -> Vec<StoredEvent> {
    let mut tx = fx.app.store.begin_system().await.unwrap();
    let evs = tx
        .outbox()
        .claim_undelivered("wp16-test", 10_000)
        .await
        .unwrap();
    // Nicht als zugestellt markieren: Rollback.
    drop(tx);
    evs
}

async fn plan_all(fx: &Fx) -> usize {
    let mut n = 0;
    for ev in events(fx).await {
        n += fx.app.plan_webhook_deliveries(&ev).await.unwrap();
    }
    n
}

async fn deliveries(fx: &Fx, hook: WebhookId) -> Vec<Value> {
    let rows: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM webhook_delivery WHERE webhook_id = $1 ORDER BY created_at, id",
    )
    .bind(hook.0)
    .fetch_all(&fx.pool)
    .await
    .unwrap();
    rows
}

fn issue(team: &str, title: &str) -> CreateIssueInput {
    CreateIssueInput {
        team: team.into(),
        title: Some(title.into()),
        ..Default::default()
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn webhook_crud_rights_and_secret_storage(pool: PgPool) {
    let fx = setup(pool).await;
    let created = fx
        .app
        .create_webhook(&alice(), ws_hook(&["Issue"]))
        .await
        .unwrap();
    let secret = created.secret.clone().unwrap();
    assert_eq!(secret.len(), 64);
    assert!(created.webhook.enabled);
    // Das Secret liegt nur verschlüsselt in der Datenbank.
    let stored: String = sqlx::query_scalar("SELECT secret FROM webhook WHERE id = $1")
        .bind(created.webhook.id.0)
        .fetch_one(&fx.pool)
        .await
        .unwrap();
    assert!(stored.starts_with("v1:") && !stored.contains(&secret));
    // Lesen liefert das Secret nie wieder.
    let listed = fx.app.list_webhooks(&alice()).await.unwrap();
    assert_eq!(listed.len(), 1);

    // Rechte: Bob (User) darf keinen Workspace-Webhook, nur für Teams, deren Owner er ist.
    assert_eq!(
        kind(fx.app.create_webhook(&bob(), ws_hook(&["Issue"])).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(
            fx.app
                .create_webhook(&bob(), team_hook("ENG", &["Issue"]))
                .await
        ),
        ErrorKind::Forbidden
    );
    fx.app
        .create_webhook(&bob(), team_hook("DES", &["Issue"]))
        .await
        .unwrap();
    // Gast: nie.
    assert!(matches!(
        kind(
            fx.app
                .create_webhook(&carol(), team_hook("WEB", &["Issue"]))
                .await
        ),
        ErrorKind::Forbidden | ErrorKind::NotFound
    ));
    // Alice sieht das private Team DES nicht -> not_found, und sieht Bobs Team-Webhook nicht.
    assert_eq!(
        kind(
            fx.app
                .create_webhook(&alice(), team_hook("DES", &["Issue"]))
                .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(fx.app.list_webhooks(&alice()).await.unwrap().len(), 1);
    let bobs = fx.app.list_webhooks(&bob()).await.unwrap();
    assert_eq!(bobs.len(), 1);
    assert_eq!(
        kind(fx.app.get_webhook(&alice(), bobs[0].id).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(fx.app.delete_webhook(&alice(), bobs[0].id).await),
        ErrorKind::NotFound
    );

    // Read-Token kann keine Webhooks anlegen; ohne Admin-Scope auch keine Workspace-Webhooks.
    let mut read = alice();
    read.scopes = Scopes::read_only();
    assert_eq!(
        kind(fx.app.create_webhook(&read, ws_hook(&["Issue"])).await),
        ErrorKind::Forbidden
    );
    let mut rw = alice();
    rw.scopes = Scopes::read_write();
    assert_eq!(
        kind(fx.app.create_webhook(&rw, ws_hook(&["Issue"])).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(fx.app.delete_webhook(&read, created.webhook.id).await),
        ErrorKind::Forbidden
    );

    // Validierung.
    for bad in [
        CreateWebhookInput {
            resource_types: vec![],
            ..ws_hook(&["Issue"])
        },
        CreateWebhookInput {
            resource_types: vec!["Team".into()],
            ..ws_hook(&["Issue"])
        },
        CreateWebhookInput {
            team: Some("ENG".into()),
            ..ws_hook(&["Issue"])
        },
        CreateWebhookInput {
            all_public_teams: false,
            team: None,
            ..ws_hook(&["Issue"])
        },
    ] {
        assert_eq!(
            kind(fx.app.create_webhook(&alice(), bad).await),
            ErrorKind::InvalidRequest
        );
    }

    // Update, Rotation, Löschen.
    let updated = fx
        .app
        .update_webhook(
            &alice(),
            created.webhook.id,
            UpdateWebhookInput {
                label: Some(Some("Neu".into())),
                resource_types: Some(vec!["Issue".into(), "Comment".into()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.label.as_deref(), Some("Neu"));
    assert_eq!(updated.resource_types.len(), 2);
    let rotated = fx
        .app
        .rotate_webhook_secret(&alice(), created.webhook.id)
        .await
        .unwrap();
    assert_ne!(rotated.secret.clone().unwrap(), secret);
    let (cur, prev) = fx
        .app
        .webhook_signing_secrets(&rotated.webhook, fx.app.clock.now())
        .unwrap();
    assert_eq!(cur, rotated.secret.clone().unwrap());
    assert_eq!(prev.as_deref(), Some(secret.as_str()));
    // Nach 24 h kein Previous mehr.
    let (_, prev) = fx
        .app
        .webhook_signing_secrets(&rotated.webhook, fx.app.clock.now() + Duration::hours(25))
        .unwrap();
    assert!(prev.is_none());
    fx.app
        .delete_webhook(&alice(), created.webhook.id)
        .await
        .unwrap();
    assert_eq!(
        kind(fx.app.get_webhook(&alice(), created.webhook.id).await),
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn webhook_targets_are_ssrf_checked_on_create_and_update(pool: PgPool) {
    let fx = setup(pool).await;
    for url in [
        "http://93.184.216.34/hook",
        "https://127.0.0.1/hook",
        "https://10.1.2.3/hook",
        "https://169.254.169.254/latest/meta-data",
        "https://[::1]/hook",
        "https://localhost/hook",
        "https://user:pw@93.184.216.34/",
    ] {
        let r = fx
            .app
            .create_webhook(
                &alice(),
                CreateWebhookInput {
                    url: url.into(),
                    ..ws_hook(&["Issue"])
                },
            )
            .await;
        assert_eq!(kind(r), ErrorKind::InvalidRequest, "{url}");
    }
    let ok = fx
        .app
        .create_webhook(&alice(), ws_hook(&["Issue"]))
        .await
        .unwrap();
    let r = fx
        .app
        .update_webhook(
            &alice(),
            ok.webhook.id,
            UpdateWebhookInput {
                url: Some("https://192.168.0.10/x".into()),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
}

#[sqlx::test(migrations = "./migrations")]
async fn deliveries_are_planned_with_visibility_filter(pool: PgPool) {
    let fx = setup(pool).await;
    let ws = fx
        .app
        .create_webhook(&alice(), ws_hook(&["Issue"]))
        .await
        .unwrap()
        .webhook;
    // Bob: Team-Webhooks für DES (privat) und für WEB; Alice: für ENG.
    let des = fx
        .app
        .create_webhook(&bob(), team_hook("DES", &["Issue"]))
        .await
        .unwrap()
        .webhook;
    let web = fx
        .app
        .create_webhook(&alice(), team_hook("WEB", &["Issue"]))
        .await
        .unwrap()
        .webhook;
    let eng = fx
        .app
        .create_webhook(&alice(), team_hook("ENG", &["Issue", "Comment"]))
        .await
        .unwrap()
        .webhook;

    fx.app
        .create_issue(&alice(), issue("ENG", "Login kaputt"))
        .await
        .unwrap();
    fx.app
        .create_issue(&bob(), issue("DES", "Neues Logo"))
        .await
        .unwrap();
    assert_eq!(plan_all(&fx).await, 3, "ws+eng fuer ENG, des fuer DES");

    let ws_d = deliveries(&fx, ws.id).await;
    assert_eq!(
        ws_d.len(),
        1,
        "private Teams gelangen nie in Workspace-Webhooks"
    );
    assert_eq!(ws_d[0]["action"], "create");
    assert_eq!(ws_d[0]["type"], "Issue");
    assert_eq!(ws_d[0]["data"]["title"], "Login kaputt");
    assert!(
        ws_d[0]["data"]["identifier"]
            .as_str()
            .unwrap()
            .starts_with("ENG-")
    );
    assert_eq!(ws_d[0]["data"]["team"]["key"], "ENG");
    assert_eq!(ws_d[0]["data"]["priority"]["name"], "No priority");
    assert_eq!(ws_d[0]["organizationId"], fx.data.org.0.to_string());
    assert!(
        ws_d[0]["url"]
            .as_str()
            .unwrap()
            .starts_with("https://lineal.example/acme/issue/ENG-")
    );
    assert_eq!(ws_d[0]["actor"]["type"], "user");
    assert!(
        ws_d[0]["actor"].get("email").is_none(),
        "keine E-Mail im Actor"
    );
    assert!(
        ws_d[0].get("webhookTimestamp").is_none(),
        "wird je Versuch gesetzt"
    );
    assert_eq!(deliveries(&fx, eng.id).await.len(), 1);
    assert_eq!(
        deliveries(&fx, web.id).await.len(),
        0,
        "WEB-Hook sieht keine ENG-Issues"
    );
    let des_d = deliveries(&fx, des.id).await;
    assert_eq!(des_d.len(), 1);
    assert_eq!(des_d[0]["data"]["title"], "Neues Logo");

    // Idempotenz (at-least-once): erneutes Planen legt nichts doppelt an.
    assert_eq!(plan_all(&fx).await, 0);

    // Events aus dem Import werden übersprungen.
    let mut ev = events(&fx)
        .await
        .into_iter()
        .find(|e| {
            matches!(
                e.event,
                lineal_domain::events::DomainEvent::IssueCreated { .. }
            )
        })
        .unwrap();
    ev.origin = EventOrigin::Import;
    ev.id = lineal_domain::ids::EventId::new_v7();
    assert_eq!(fx.app.plan_webhook_deliveries(&ev).await.unwrap(), 0);

    // Deaktivierte Webhooks erhalten nichts mehr.
    fx.app
        .update_webhook(
            &alice(),
            ws.id,
            UpdateWebhookInput {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    fx.app
        .create_issue(&alice(), issue("ENG", "Zweites"))
        .await
        .unwrap();
    plan_all(&fx).await;
    assert_eq!(deliveries(&fx, ws.id).await.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn project_payload_hides_private_teams_from_workspace_hooks(pool: PgPool) {
    let fx = setup(pool).await;
    let ws = fx
        .app
        .create_webhook(&alice(), ws_hook(&["Project"]))
        .await
        .unwrap()
        .webhook;
    let des = fx
        .app
        .create_webhook(&bob(), team_hook("DES", &["Project"]))
        .await
        .unwrap()
        .webhook;
    fx.app
        .save_project(
            &bob(),
            SaveProjectInput {
                name: Some("Relaunch".into()),
                set_teams: Some(vec!["ENG".into(), "DES".into()]),
                lead_team: Some("ENG".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    plan_all(&fx).await;
    let ws_d = deliveries(&fx, ws.id).await;
    assert_eq!(ws_d.len(), 1);
    let eng = fx.data.eng.0.to_string();
    let des_id = fx.data.des.0.to_string();
    assert_eq!(ws_d[0]["data"]["teamIds"], serde_json::json!([eng]));
    assert!(
        !ws_d[0].to_string().contains(&des_id),
        "private Team-ID darf nicht auftauchen"
    );
    // Der Team-Webhook des privaten Teams sieht nur sein Team.
    let des_d = deliveries(&fx, des.id).await;
    assert_eq!(des_d.len(), 1);
    assert_eq!(des_d[0]["data"]["teamIds"], serde_json::json!([des_id]));
}

#[sqlx::test(migrations = "./migrations")]
async fn retries_follow_the_plan_and_the_webhook_is_disabled_after_failures(pool: PgPool) {
    let fx = setup(pool).await;
    let hook = fx
        .app
        .create_webhook(&alice(), ws_hook(&["Issue"]))
        .await
        .unwrap()
        .webhook;
    for i in 0..5 {
        fx.app
            .create_issue(&alice(), issue("ENG", &format!("Issue {i}")))
            .await
            .unwrap();
    }
    plan_all(&fx).await;
    assert_eq!(deliveries(&fx, hook.id).await.len(), 5);

    // Versuch 1: alle 5 fällig; Fehlschlag -> Retry in ca. 1 min.
    let due = fx
        .app
        .claim_due_webhook_deliveries(fx.app.clock.now(), 100)
        .await
        .unwrap();
    assert_eq!(due.len(), 5);
    assert_eq!(due[0].attempt, 0);
    // Während des Leases (10 min) werden sie nicht erneut vergeben.
    assert!(
        fx.app
            .claim_due_webhook_deliveries(fx.app.clock.now(), 100)
            .await
            .unwrap()
            .is_empty()
    );
    for d in &due {
        fx.app
            .record_webhook_attempt(d, false, Some(500), 12, Some("HTTP 500".into()))
            .await
            .unwrap();
    }
    let next: DateTime<Utc> =
        sqlx::query_scalar("SELECT min(next_attempt_at) FROM webhook_delivery")
            .fetch_one(&fx.pool)
            .await
            .unwrap();
    let delay = (next - fx.app.clock.now()).num_seconds();
    assert!((54..=66).contains(&delay), "1 min +- 10 %: {delay}");

    // Versuch 2 nach 1 h, Versuch 3 nach 6 h, Versuch 4 -> aufgegeben.
    let mut attempts = vec![due.len()];
    for (wait, expected_attempt) in [(2 * 60, 1), (66 * 60 + 10, 2), (6 * 3600 + 2400, 3)] {
        fx.clock.advance(wait);
        let due = fx
            .app
            .claim_due_webhook_deliveries(fx.app.clock.now(), 100)
            .await
            .unwrap();
        assert_eq!(due.len(), 5, "attempt {expected_attempt}");
        assert!(due.iter().all(|d| d.attempt == expected_attempt));
        attempts.push(due.len());
        let mut last_disabled = false;
        for d in &due {
            last_disabled = fx
                .app
                .record_webhook_attempt(d, false, None, 5, Some("timeout".into()))
                .await
                .unwrap();
        }
        if expected_attempt == 3 {
            assert!(
                last_disabled,
                "5. aufgegebene Zustellung deaktiviert den Webhook"
            );
        }
    }
    let status: Vec<String> = sqlx::query_scalar("SELECT DISTINCT status FROM webhook_delivery")
        .fetch_all(&fx.pool)
        .await
        .unwrap();
    assert_eq!(status, vec!["given_up".to_string()]);
    let w = fx.app.get_webhook(&alice(), hook.id).await.unwrap();
    assert!(!w.enabled);
    assert_eq!(w.disabled_reason.as_deref(), Some("delivery_failures"));
    assert_eq!(w.consecutive_failures, 5);
    // Nichts mehr fällig.
    fx.clock.advance(7 * 24 * 3600);
    assert!(
        fx.app
            .claim_due_webhook_deliveries(fx.app.clock.now(), 100)
            .await
            .unwrap()
            .is_empty()
    );
    // Manuelle Reaktivierung setzt den Zähler zurück.
    let w = fx
        .app
        .update_webhook(
            &alice(),
            hook.id,
            UpdateWebhookInput {
                enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(w.enabled && w.consecutive_failures == 0 && w.disabled_reason.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn a_successful_delivery_resets_the_failure_counter(pool: PgPool) {
    let fx = setup(pool).await;
    let hook = fx
        .app
        .create_webhook(&alice(), ws_hook(&["Issue"]))
        .await
        .unwrap()
        .webhook;
    fx.app
        .create_issue(&alice(), issue("ENG", "A"))
        .await
        .unwrap();
    plan_all(&fx).await;
    sqlx::query("UPDATE webhook SET failure_count = 3 WHERE id = $1")
        .bind(hook.id.0)
        .execute(&fx.pool)
        .await
        .unwrap();
    let due = fx
        .app
        .claim_due_webhook_deliveries(fx.app.clock.now(), 10)
        .await
        .unwrap();
    fx.app
        .record_webhook_attempt(&due[0], true, Some(200), 8, None)
        .await
        .unwrap();
    let w = fx.app.get_webhook(&alice(), hook.id).await.unwrap();
    assert_eq!(w.consecutive_failures, 0);
    assert!(w.last_success_at.is_some());
    let log = fx
        .app
        .list_webhook_deliveries(&alice(), hook.id)
        .await
        .unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].status, "succeeded");
    assert_eq!(log[0].response_code, Some(200));
    // „Erneut senden“.
    fx.app
        .resend_webhook_delivery(&alice(), hook.id, log[0].id)
        .await
        .unwrap();
    assert_eq!(
        fx.app
            .claim_due_webhook_deliveries(fx.app.clock.now(), 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

// ---------------------------------------------------------------------------
// Einladungen
// ---------------------------------------------------------------------------

fn invite(emails: &[&str], role: UserRole, teams: &[&str]) -> InviteUsersInput {
    InviteUsersInput {
        emails: emails.iter().map(|s| (*s).to_string()).collect(),
        role: Some(role),
        teams: teams.iter().map(|s| (*s).to_string()).collect(),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn invitations_invite_accept_resend_revoke(pool: PgPool) {
    let fx = setup(pool).await;
    let created = fx
        .app
        .invite_users(
            &alice(),
            invite(&["dave@acme.example"], UserRole::User, &["ENG"]),
        )
        .await
        .unwrap();
    assert_eq!(created.len(), 1);
    let (inv, token) = (created[0].invitation.clone(), created[0].token.clone());
    assert_eq!(inv.status, "pending");
    assert_eq!(token.len(), 96);
    // Nur der Hash steht in der Datenbank.
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM invitation WHERE id = $1")
        .bind(inv.id)
        .fetch_one(&fx.pool)
        .await
        .unwrap();
    assert_eq!(stored, lineal_app::usecases::auth::hash_token(&token));
    assert_ne!(stored, token);

    // Rechte, Duplikate, Validierung.
    assert_eq!(
        kind(
            fx.app
                .invite_users(&carol(), invite(&["x@acme.example"], UserRole::User, &[]))
                .await
        ),
        ErrorKind::Forbidden
    );
    let mut read = alice();
    read.scopes = Scopes::read_only();
    assert_eq!(
        kind(
            fx.app
                .invite_users(&read, invite(&["x@acme.example"], UserRole::User, &[]))
                .await
        ),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(
            fx.app
                .invite_users(
                    &alice(),
                    invite(&["DAVE@acme.example"], UserRole::User, &[])
                )
                .await
        ),
        ErrorKind::Conflict
    );
    assert_eq!(
        kind(
            fx.app
                .invite_users(&alice(), invite(&["bob@acme.example"], UserRole::User, &[]))
                .await
        ),
        ErrorKind::Conflict
    );
    assert_eq!(
        kind(
            fx.app
                .invite_users(&alice(), invite(&["g@acme.example"], UserRole::Guest, &[]))
                .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            fx.app
                .invite_users(&alice(), invite(&["nope"], UserRole::User, &[]))
                .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            fx.app
                .invite_users(&alice(), invite(&["o@acme.example"], UserRole::Owner, &[]))
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // Bob darf (invitationsRole = user), aber keinen Admin einladen und nicht in fremde Teams.
    assert_eq!(
        kind(
            fx.app
                .invite_users(&bob(), invite(&["a@acme.example"], UserRole::Admin, &[]))
                .await
        ),
        ErrorKind::Forbidden
    );
    assert_eq!(
        fx.app.list_invitations(&alice(), true).await.unwrap().len(),
        1
    );

    // Annehmen: falsche/unbekannte Token -> not_found.
    let accept = |token: &str| AcceptInvitationInput {
        token: token.into(),
        email: "dave@acme.example".into(),
        name: Some("Dave Example".into()),
        password: Some("correct horse battery".into()),
    };
    assert_eq!(
        kind(fx.app.accept_invitation(accept("nope")).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(fx.app.accept_invitation(accept(&"0".repeat(96))).await),
        ErrorKind::NotFound
    );
    let ok = fx.app.accept_invitation(accept(&token)).await.unwrap();
    assert_eq!(ok.url_key, "acme");
    assert!(ok.login.is_some());
    // Eingeladener ist User, Mitglied von ENG, kann sich anmelden.
    let (role, active): (String, bool) =
        sqlx::query_as("SELECT role, active FROM \"user\" WHERE id = $1")
            .bind(ok.user_id.0)
            .fetch_one(&fx.pool)
            .await
            .unwrap();
    assert_eq!((role.as_str(), active), ("user", true));
    let member: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM team_membership WHERE user_id = $1 AND team_id = $2",
    )
    .bind(ok.user_id.0)
    .bind(fx.data.eng.0)
    .fetch_one(&fx.pool)
    .await
    .unwrap();
    assert_eq!(member, 1);
    let principal = fx
        .app
        .principal_for_token(&ok.login.unwrap().token)
        .await
        .unwrap();
    assert_eq!(principal.org_id, fx.data.org);
    // Einmalig.
    assert_eq!(
        kind(fx.app.accept_invitation(accept(&token)).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        fx.app.list_invitations(&alice(), true).await.unwrap().len(),
        0
    );

    // Resend macht den alten Link ungültig; Revoke den neuen.
    let c2 = fx
        .app
        .invite_users(
            &alice(),
            invite(&["erin@acme.example"], UserRole::Guest, &["WEB"]),
        )
        .await
        .unwrap();
    let (id2, tok_old) = (c2[0].invitation.id, c2[0].token.clone());
    let re = fx.app.resend_invitation(&alice(), id2).await.unwrap();
    assert_ne!(re.token, tok_old);
    assert_eq!(
        kind(fx.app.accept_invitation(accept(&tok_old)).await),
        ErrorKind::NotFound
    );
    let revoked = fx.app.revoke_invitation(&alice(), id2).await.unwrap();
    assert_eq!(revoked.status, "revoked");
    assert_eq!(
        kind(fx.app.accept_invitation(accept(&re.token)).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(fx.app.revoke_invitation(&alice(), id2).await),
        ErrorKind::Conflict
    );
    assert_eq!(
        kind(fx.app.resend_invitation(&alice(), id2).await),
        ErrorKind::Conflict
    );

    // Abgelaufen (7 Tage).
    let c3 = fx
        .app
        .invite_users(
            &alice(),
            invite(&["fred@acme.example"], UserRole::User, &[]),
        )
        .await
        .unwrap();
    fx.clock.advance(8 * 24 * 3600);
    assert_eq!(
        kind(fx.app.accept_invitation(accept(&c3[0].token)).await),
        ErrorKind::NotFound
    );
    // Nach Ablauf kann neu eingeladen werden.
    fx.app
        .invite_users(
            &alice(),
            invite(&["fred@acme.example"], UserRole::User, &[]),
        )
        .await
        .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn accepting_links_the_imported_placeholder_user(pool: PgPool) {
    let fx = setup(pool).await;
    let placeholder = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO \"user\" (id, organization_id, name, display_name, email, role, status, active) \
         VALUES ($1, $2, 'Gina (importiert)', 'gina', 'gina@acme.example', 'user', 'imported', false)",
    )
    .bind(placeholder)
    .bind(fx.data.org.0)
    .execute(&fx.pool)
    .await
    .unwrap();
    let c = fx
        .app
        .invite_users(
            &alice(),
            invite(&["gina@acme.example"], UserRole::User, &["ENG"]),
        )
        .await
        .unwrap();
    let ok = fx
        .app
        .accept_invitation(AcceptInvitationInput {
            token: c[0].token.clone(),
            email: "GINA@acme.example".into(),
            name: Some("Gina Example".into()),
            password: Some("correct horse battery".into()),
        })
        .await
        .unwrap();
    assert_eq!(
        ok.user_id.0, placeholder,
        "Platzhalter wird verknüpft, kein zweiter User"
    );
    let (status, active, name): (String, bool, String) =
        sqlx::query_as("SELECT status, active, name FROM \"user\" WHERE id = $1")
            .bind(placeholder)
            .fetch_one(&fx.pool)
            .await
            .unwrap();
    assert_eq!(
        (status.as_str(), active, name.as_str()),
        ("active", true, "Gina Example")
    );
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM \"user\" WHERE lower(email) = 'gina@acme.example'",
    )
    .fetch_one(&fx.pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
    assert!(ok.login.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn invitations_of_another_workspace_cannot_be_accepted_or_managed(pool: PgPool) {
    let fx = setup(pool).await;
    // Zweiter Workspace "beta" mit Owner Zoe.
    let beta = fx
        .app
        .create_workspace(lineal_app::usecases::org::CreateWorkspaceInput {
            name: "Beta".into(),
            url_key: Some("beta".into()),
            owner_name: "Zoe Example".into(),
            owner_email: "zoe@beta.example".into(),
            owner_password: Some("correct horse battery".into()),
        })
        .await
        .unwrap();
    let zoe = RequestCtx {
        org_id: beta.org.id,
        actor: lineal_app::Actor::User {
            id: beta.owner.id,
            role: UserRole::Owner,
        },
        request_id: "r".into(),
        scopes: Scopes::all(),
        team_scope: None,
    };
    let c = fx
        .app
        .invite_users(
            &alice(),
            invite(&["dave@acme.example"], UserRole::User, &[]),
        )
        .await
        .unwrap();
    // Verwaltung: Zoe sieht die Einladung von acme nicht.
    assert_eq!(
        kind(fx.app.resend_invitation(&zoe, c[0].invitation.id).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(fx.app.revoke_invitation(&zoe, c[0].invitation.id).await),
        ErrorKind::NotFound
    );
    assert!(
        fx.app
            .list_invitations(&zoe, false)
            .await
            .unwrap()
            .is_empty()
    );
    // Annahme über eine Konto-Session mit anderer E-Mail: nicht annehmbar.
    assert_eq!(
        kind(fx.app.accept_invitation_as_account(&zoe, &c[0].token).await),
        ErrorKind::NotFound
    );
    // Auch Alices eigene Session (falsche Identität) nicht.
    assert_eq!(
        kind(
            fx.app
                .accept_invitation_as_account(&alice(), &c[0].token)
                .await
        ),
        ErrorKind::NotFound
    );
    // Passt die E-Mail (Zoe wird von acme unter ihrer Beta-Adresse eingeladen), geht es.
    let c2 = fx
        .app
        .invite_users(
            &alice(),
            invite(&["zoe@beta.example"], UserRole::User, &["ENG"]),
        )
        .await
        .unwrap();
    let ok = fx
        .app
        .accept_invitation_as_account(&zoe, &c2[0].token)
        .await
        .unwrap();
    assert_eq!(ok.org_id, fx.data.org);
    let hash: Option<String> =
        sqlx::query_scalar("SELECT password_hash FROM \"user\" WHERE id = $1")
            .bind(ok.user_id.0)
            .fetch_one(&fx.pool)
            .await
            .unwrap();
    assert!(hash.is_some(), "Passwort der Konto-Session wird übernommen");
}

// ---------------------------------------------------------------------------
// Team-Lebenszyklus
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "./migrations")]
async fn retire_and_restore(pool: PgPool) {
    let fx = setup(pool).await;
    // ENG hat das Sub-Team WEB: ohne Angabe invalid_request.
    let none = fx
        .app
        .retire_team(&alice(), "ENG", None, OpenIssuesOnRetirement::Keep)
        .await;
    assert_eq!(kind(none), ErrorKind::InvalidRequest);
    // Nur Owner/Admin: Bob (Team-Owner von WEB, nur Mitglied von ENG) darf ENG nicht.
    assert_eq!(
        kind(
            fx.app
                .retire_team(
                    &bob(),
                    "ENG",
                    Some(SubTeamHandling::Cascade),
                    OpenIssuesOnRetirement::Keep
                )
                .await
        ),
        ErrorKind::Forbidden
    );
    // Kaskade: beide read-only.
    let v = fx
        .app
        .retire_team(
            &alice(),
            "ENG",
            Some(SubTeamHandling::Cascade),
            OpenIssuesOnRetirement::Keep,
        )
        .await
        .unwrap();
    assert!(v.team.retired_at.is_some());
    let web = fx.app.get_team(&alice(), "WEB").await.unwrap();
    assert!(web.team.retired_at.is_some());
    let r = fx.app.create_issue(&alice(), issue("ENG", "x")).await;
    let err = r.err().unwrap();
    assert_eq!(err.kind, ErrorKind::Forbidden);
    assert_eq!(err.details["reason"], "team_retired");
    // Restore: erst das Sub-Team braucht aktiven Parent.
    assert_eq!(
        kind(fx.app.restore_team(&alice(), "WEB").await),
        ErrorKind::InvalidRequest
    );
    fx.app.restore_team(&alice(), "ENG").await.unwrap();
    fx.app.restore_team(&alice(), "WEB").await.unwrap();
    fx.app
        .create_issue(&alice(), issue("ENG", "wieder da"))
        .await
        .unwrap();
    assert_eq!(
        kind(fx.app.restore_team(&alice(), "ENG").await),
        ErrorKind::InvalidRequest
    );

    // Unnest: WEB wird Root-Team, ENG stillgelegt.
    fx.app
        .retire_team(
            &alice(),
            "ENG",
            Some(SubTeamHandling::Unnest),
            OpenIssuesOnRetirement::Keep,
        )
        .await
        .unwrap();
    let web = fx.app.get_team(&alice(), "WEB").await.unwrap();
    assert!(web.team.parent_id.is_none() && web.team.retired_at.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn delete_restore_and_purge_a_team(pool: PgPool) {
    let fx = setup(pool).await;
    // DES (privat; Bob ist Owner) mit Issue und eigenem Projekt; Multi-Team-Projekt mit ENG.
    fx.app
        .create_issue(&bob(), issue("DES", "Logo"))
        .await
        .unwrap();
    let sole = fx
        .app
        .save_project(
            &bob(),
            SaveProjectInput {
                name: Some("Nur Design".into()),
                set_teams: Some(vec!["DES".into()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let shared = fx
        .app
        .save_project(
            &bob(),
            SaveProjectInput {
                name: Some("Gemeinsam".into()),
                set_teams: Some(vec!["DES".into(), "ENG".into()]),
                lead_team: Some("DES".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // Alice (Admin) sieht DES nicht und darf es nicht löschen? Admin darf laut Matrix (Metadaten):
    // ohne Sichtbarkeit des Inhalts nur über Management-Auflösung.
    let v = fx.app.delete_team(&bob(), "DES", None).await.unwrap();
    assert!(v.team.trashed && v.team.trashed_at.is_some());
    // Für Nicht-Admins sofort unsichtbar.
    assert_eq!(
        kind(fx.app.get_team(&bob(), "DES").await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(fx.app.delete_team(&bob(), "DES", None).await),
        ErrorKind::NotFound
    );
    // Nur-DES-Projekt im Papierkorb; das gemeinsame Projekt lebt weiter mit neuem Lead ENG.
    let trashed: bool = sqlx::query_scalar("SELECT trashed FROM project WHERE id = $1")
        .bind(sole.project.id.0)
        .fetch_one(&fx.pool)
        .await
        .unwrap();
    assert!(trashed);
    let (lead, shared_trashed): (uuid::Uuid, bool) =
        sqlx::query_as("SELECT lead_team_id, trashed FROM project WHERE id = $1")
            .bind(shared.project.id.0)
            .fetch_one(&fx.pool)
            .await
            .unwrap();
    assert_eq!(lead, fx.data.eng.0);
    assert!(!shared_trashed);
    let still: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM project_team WHERE project_id = $1 AND team_id = $2",
    )
    .bind(shared.project.id.0)
    .bind(fx.data.des.0)
    .fetch_one(&fx.pool)
    .await
    .unwrap();
    assert_eq!(still, 0, "Team wurde aus dem Multi-Team-Projekt entfernt");

    // Wiederherstellen: nur Admin/Owner (Bob nicht), Alice kann.
    assert_eq!(
        kind(
            fx.app
                .restore_deleted_team(&bob(), &fx.data.des.0.to_string())
                .await
        ),
        ErrorKind::NotFound
    );
    let back = fx
        .app
        .restore_deleted_team(&alice(), &fx.data.des.0.to_string())
        .await
        .unwrap();
    assert!(!back.team.trashed);
    let trashed: bool = sqlx::query_scalar("SELECT trashed FROM project WHERE id = $1")
        .bind(sole.project.id.0)
        .fetch_one(&fx.pool)
        .await
        .unwrap();
    assert!(!trashed, "Projekte aus dem Papierkorb kommen mit zurück");
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM project_team WHERE project_id = $1 AND team_id = $2",
    )
    .bind(shared.project.id.0)
    .bind(fx.data.des.0)
    .fetch_one(&fx.pool)
    .await
    .unwrap();
    assert_eq!(n, 0, "entfernte Multi-Team-Zuordnungen kommen nicht zurück");
    fx.app.get_team(&bob(), "DES").await.unwrap();

    // Erneut löschen, Frist abwarten, purgen: alles weg, Key frei.
    fx.app.delete_team(&bob(), "DES", None).await.unwrap();
    fx.clock.advance(29 * 24 * 3600);
    assert_eq!(
        fx.app
            .run_team_purge(fx.app.clock.now())
            .await
            .unwrap()
            .processed,
        0
    );
    // Nach 30 Tagen ist die Wiederherstellung nicht mehr möglich.
    fx.clock.advance(2 * 24 * 3600);
    assert_eq!(
        kind(
            fx.app
                .restore_deleted_team(&alice(), &fx.data.des.0.to_string())
                .await
        ),
        ErrorKind::NotFound
    );
    let stats = fx.app.run_team_purge(fx.app.clock.now()).await.unwrap();
    assert!(stats.errors.is_empty(), "{:?}", stats.errors);
    assert_eq!(stats.processed, 1);
    for (table, col) in [
        ("team", "id"),
        ("issue", "team_id"),
        ("workflow_state", "team_id"),
        ("team_membership", "team_id"),
    ] {
        let n: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table} WHERE {col} = $1"))
            .bind(fx.data.des.0)
            .fetch_one(&fx.pool)
            .await
            .unwrap();
        assert_eq!(n, 0, "{table}");
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM project WHERE id = $1")
        .bind(sole.project.id.0)
        .fetch_one(&fx.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    // Das gemeinsame Projekt (jetzt ENG) bleibt.
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM project WHERE id = $1")
        .bind(shared.project.id.0)
        .fetch_one(&fx.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
    // Der Key ist wieder frei.
    let create = fx
        .app
        .create_team(
            &alice(),
            lineal_app::usecases::team::CreateTeamInput {
                name: "Design Neu".into(),
                key: Some("DES".into()),
                ..Default::default()
            },
        )
        .await;
    assert!(create.is_ok(), "{create:?}");
}

#[sqlx::test(migrations = "./migrations")]
async fn deleting_a_team_with_sub_teams_needs_a_decision(pool: PgPool) {
    let fx = setup(pool).await;
    assert_eq!(
        kind(fx.app.delete_team(&alice(), "ENG", None).await),
        ErrorKind::InvalidRequest
    );
    fx.app
        .delete_team(&alice(), "ENG", Some(SubTeamHandling::Cascade))
        .await
        .unwrap();
    let trashed: Vec<bool> =
        sqlx::query_scalar("SELECT trashed FROM team WHERE id = ANY($1) ORDER BY key")
            .bind(vec![fx.data.eng.0, fx.data.web.0])
            .fetch_all(&fx.pool)
            .await
            .unwrap();
    assert_eq!(trashed, vec![true, true]);
    // Wiederherstellen bringt das gemeinsam gelöschte Sub-Team mit.
    fx.app.restore_deleted_team(&alice(), "ENG").await.unwrap();
    let trashed: Vec<bool> = sqlx::query_scalar("SELECT trashed FROM team WHERE id = ANY($1)")
        .bind(vec![fx.data.eng.0, fx.data.web.0])
        .fetch_all(&fx.pool)
        .await
        .unwrap();
    assert_eq!(trashed, vec![false, false]);
    // Unnest: WEB wird Root, ENG gelöscht.
    fx.app
        .delete_team(&alice(), "ENG", Some(SubTeamHandling::Unnest))
        .await
        .unwrap();
    let web = fx.app.get_team(&alice(), "WEB").await.unwrap();
    assert!(web.team.parent_id.is_none() && !web.team.trashed);
}

#[sqlx::test(migrations = "./migrations")]
async fn team_key_change_keeps_history_and_old_identifiers_resolve(pool: PgPool) {
    let fx = setup(pool).await;
    let i = fx
        .app
        .create_issue(&alice(), issue("ENG", "Alt"))
        .await
        .unwrap();
    let old_ident = i.issue.identifier.clone();
    assert!(old_ident.starts_with("ENG-"));
    fx.app
        .update_team(
            &alice(),
            "ENG",
            UpdateTeamInput {
                key: Some("CORE".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let t = fx.app.get_team(&alice(), "ENG").await.unwrap();
    assert_eq!(t.team.key, "CORE", "alter Key löst auf das Team auf");
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM team_key WHERE team_id = $1 AND key = 'ENG'")
            .bind(fx.data.eng.0)
            .fetch_one(&fx.pool)
            .await
            .unwrap();
    assert_eq!(n, 1);
    let again = fx.app.get_issue(&alice(), &old_ident).await.unwrap();
    assert!(again.view.issue.identifier.starts_with("CORE-"));
}

#[sqlx::test(migrations = "./migrations")]
async fn cycle_settings_changes_drive_the_cycle_engine(pool: PgPool) {
    let fx = setup(pool).await;
    // ENG aktiviert Cycles über update_team: Cycles entstehen sofort (WP07-Anbindung).
    fx.app
        .update_team(
            &alice(),
            "ENG",
            UpdateTeamInput {
                cycles_enabled: Some(true),
                cycle_duration: Some(2),
                upcoming_cycle_count: Some(3),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let (_, cycles) = fx.app.list_cycles(&alice(), "ENG", None).await.unwrap();
    assert!(cycles.len() >= 3, "{} Cycles", cycles.len());
    // Sub-Team WEB erbt (Nesting-Zustand ist im Seed gegeben; WEB hat Cycles aktiviert nachgezogen).
    let (_, web) = fx.app.list_cycles(&alice(), "WEB", None).await.unwrap();
    assert!(web.is_empty() || web.iter().all(|c| c.cycle.inherited_from_id.is_some()));
    // Schedule-Änderung wird neu berechnet (künftige Cycles bleiben in der Anzahl).
    fx.app
        .update_team(
            &alice(),
            "ENG",
            UpdateTeamInput {
                cycle_duration: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let (_, after) = fx.app.list_cycles(&alice(), "ENG", None).await.unwrap();
    assert_eq!(after.iter().filter(|c| c.is_future).count(), 3);
    // Deaktivieren.
    fx.app
        .update_team(
            &alice(),
            "ENG",
            UpdateTeamInput {
                cycles_enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let (_, off) = fx.app.list_cycles(&alice(), "ENG", None).await.unwrap();
    assert!(off.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn invitation_authorization_regressions(pool: PgPool) {
    let fx = setup(pool).await;
    let c = fx
        .app
        .invite_users(
            &alice(),
            invite(&["dave@acme.example"], UserRole::User, &["ENG"]),
        )
        .await
        .unwrap();
    // Falsche E-Mail zum Token: nicht annehmbar (auch case-insensitiv passende geht).
    let bad = fx
        .app
        .accept_invitation(AcceptInvitationInput {
            token: c[0].token.clone(),
            email: "mallory@acme.example".into(),
            name: None,
            password: Some("correct horse battery".into()),
        })
        .await;
    assert_eq!(kind(bad), ErrorKind::NotFound);
    // Bob (nicht Ersteller, kein Admin) darf weder erneut senden noch widerrufen.
    assert_eq!(
        kind(fx.app.resend_invitation(&bob(), c[0].invitation.id).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(fx.app.revoke_invitation(&bob(), c[0].invitation.id).await),
        ErrorKind::NotFound
    );
    // Gast in ein Team, dessen Owner Bob nicht ist: verboten; als Owner von WEB erlaubt.
    assert_eq!(
        kind(
            fx.app
                .invite_users(
                    &bob(),
                    invite(&["g1@acme.example"], UserRole::Guest, &["ENG"])
                )
                .await
        ),
        ErrorKind::Forbidden
    );
    fx.app
        .invite_users(
            &bob(),
            invite(&["g2@acme.example"], UserRole::Guest, &["WEB"]),
        )
        .await
        .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn webhook_target_is_revalidated_at_delivery_and_creator_visibility_is_enforced(
    pool: PgPool,
) {
    let fx = setup(pool).await;
    let hook = fx
        .app
        .create_webhook(&alice(), ws_hook(&["Issue"]))
        .await
        .unwrap()
        .webhook;
    // Rebinding-Szenario: das Ziel wurde beim Speichern geprüft, zeigt zur Zustellung aber auf Loopback
    // (hier direkt in der DB; zur Laufzeit würde DNS wechseln). Die Zustellung muss neu prüfen.
    fx.app
        .create_issue(&alice(), issue("ENG", "A"))
        .await
        .unwrap();
    plan_all(&fx).await;
    sqlx::query("UPDATE webhook SET url = 'https://127.0.0.1/hook' WHERE id = $1")
        .bind(hook.id.0)
        .execute(&fx.pool)
        .await
        .unwrap();
    let due = fx
        .app
        .claim_due_webhook_deliveries(fx.app.clock.now(), 10)
        .await
        .unwrap();
    assert_eq!(due.len(), 1);
    let policy = lineal_app::usecases::webhook::UrlPolicy::for_app(&fx.app);
    assert_eq!(policy, lineal_app::usecases::webhook::UrlPolicy::STRICT);
    let url = lineal_app::usecases::webhook::parse_target_url(&due[0].webhook.url, policy);
    assert!(
        url.is_err(),
        "Loopback-Ziel wird bei der Zustellung abgelehnt"
    );
    for mapped in [
        "https://[::ffff:127.0.0.1]/",
        "https://[::ffff:10.0.0.1]/",
        "https://0.0.0.0/",
        "https://100.64.1.1/",
        "https://224.0.0.1/",
    ] {
        assert!(
            lineal_app::usecases::webhook::parse_target_url(mapped, policy).is_err(),
            "{mapped}"
        );
    }

    // Ersteller verliert die Sicht auf das private Team: keine Zustellung mehr.
    let des = fx
        .app
        .create_webhook(&bob(), team_hook("DES", &["Issue"]))
        .await
        .unwrap()
        .webhook;
    sqlx::query("DELETE FROM team_membership WHERE team_id = $1 AND user_id = $2")
        .bind(fx.data.des.0)
        .bind(fx.data.bob.0)
        .execute(&fx.pool)
        .await
        .unwrap();
    // (Alice legt das Issue an: als Admin ohne Mitgliedschaft nicht möglich; Bob war Mitglied -> direkt per SQL.)
    let before = deliveries(&fx, des.id).await.len();
    fx.app
        .create_issue(&alice(), issue("ENG", "B"))
        .await
        .unwrap();
    plan_all(&fx).await;
    assert_eq!(deliveries(&fx, des.id).await.len(), before);
}

#[sqlx::test(migrations = "./migrations")]
async fn delivery_context_is_reevaluated_for_every_event(pool: PgPool) {
    use lineal_app::usecases::team_lifecycle::{OpenIssuesOnRetirement, SubTeamHandling};
    let fx = setup(pool).await;
    // Update kann weder Ziel noch Scope ändern: Rechte und Ziel bleiben die der Anlage.
    let w = fx
        .app
        .create_webhook(&bob(), team_hook("DES", &["Issue"]))
        .await
        .unwrap()
        .webhook;
    let upd = fx
        .app
        .update_webhook(
            &bob(),
            w.id,
            UpdateWebhookInput {
                resource_types: Some(vec!["Issue".into(), "Comment".into()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!((upd.team_id, upd.all_public_teams), (w.team_id, false));
    // Ein Team-Owner kann den Webhook nicht zum Workspace-Webhook machen, und fremde Webhooks nicht ändern.
    assert_eq!(
        kind(fx.app.create_webhook(&bob(), ws_hook(&["Issue"])).await),
        ErrorKind::Forbidden
    );

    // WEB-Webhook (Bob ist Team-Owner): Zustellung nur solange Team aktiv und Ersteller aktiv/Mitglied.
    let web = fx
        .app
        .create_webhook(&bob(), team_hook("WEB", &["Issue"]))
        .await
        .unwrap()
        .webhook;
    fx.app
        .create_issue(&bob(), issue("WEB", "eins"))
        .await
        .unwrap();
    plan_all(&fx).await;
    assert_eq!(deliveries(&fx, web.id).await.len(), 1);

    // Stillgelegtes Team: keine Zustellung (Alice legt WEB still, Bob kann nicht mehr schreiben).
    fx.app
        .retire_team(
            &alice(),
            "WEB",
            Some(SubTeamHandling::Cascade),
            OpenIssuesOnRetirement::Keep,
        )
        .await
        .unwrap();
    let ev = lineal_domain::events::DomainEvent::IssueUpdated {
        issue_id: sqlx::query_scalar::<_, uuid::Uuid>(
            "SELECT id FROM issue WHERE team_id = $1 LIMIT 1",
        )
        .bind(fx.data.web.0)
        .fetch_one(&fx.pool)
        .await
        .map(lineal_domain::ids::IssueId)
        .unwrap(),
        team_id: fx.data.web,
        changed: vec!["title".into()],
        mentioned_user_ids: vec![],
    };
    let stored = |e: lineal_domain::events::DomainEvent| StoredEvent {
        id: lineal_domain::ids::EventId::new_v7(),
        org_id: fx.data.org,
        origin: EventOrigin::User,
        actor_id: None,
        sync_id: 1,
        event: e,
        created_at: Utc::now(),
    };
    assert_eq!(
        fx.app
            .plan_webhook_deliveries(&stored(ev.clone()))
            .await
            .unwrap(),
        0,
        "retired"
    );
    fx.app.restore_team(&alice(), "ENG").await.ok();
    fx.app.restore_team(&alice(), "WEB").await.ok();
    fx.app.restore_team(&alice(), "ENG").await.ok();
    assert_eq!(
        fx.app
            .plan_webhook_deliveries(&stored(ev.clone()))
            .await
            .unwrap(),
        1,
        "active again"
    );

    // Ersteller suspendiert -> keine Zustellung.
    sqlx::query("UPDATE \"user\" SET active = false WHERE id = $1")
        .bind(fx.data.bob.0)
        .execute(&fx.pool)
        .await
        .unwrap();
    assert_eq!(
        fx.app
            .plan_webhook_deliveries(&stored(ev.clone()))
            .await
            .unwrap(),
        0,
        "suspended creator"
    );
    sqlx::query("UPDATE \"user\" SET active = true WHERE id = $1")
        .bind(fx.data.bob.0)
        .execute(&fx.pool)
        .await
        .unwrap();
    assert_eq!(
        fx.app
            .plan_webhook_deliveries(&stored(ev.clone()))
            .await
            .unwrap(),
        1
    );

    // Gelöschtes Team und privates Team ohne Mitgliedschaft des Erstellers.
    sqlx::query("UPDATE team SET trashed = true, trashed_at = now() WHERE id = $1")
        .bind(fx.data.web.0)
        .execute(&fx.pool)
        .await
        .unwrap();
    assert_eq!(
        fx.app.plan_webhook_deliveries(&stored(ev)).await.unwrap(),
        0,
        "trashed"
    );
}
