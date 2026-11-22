#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP16: Zustellung im Konsumenten `webhooks` – Signatur über den rohen Body, echter HTTP-Empfänger
//! (Dev-Policy), SSRF-Prüfung bei jeder Zustellung (Rebinding), Redirects werden nicht verfolgt.

use async_trait::async_trait;
use lineal_app::ports::Store;
use lineal_app::usecases::issue::{
    BatchPatch, BatchUpdateInput, CreateIssueInput, UpdateIssueInput,
};
use lineal_app::usecases::project::milestone::{OpenIssues, SaveMilestoneInput};
use lineal_app::usecases::webhook::{CreateWebhookInput, sign};
use lineal_app::{App, RequestCtx};
use lineal_db::PgStore;
use lineal_jobs::EventConsumer;
use lineal_jobs::consumers::webhooks::{
    ReqwestTransport, TransportResult, WebhookRequest, WebhookTransport, WebhooksConsumer,
    deliver_due,
};
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::Value;
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn alice() -> RequestCtx {
    ctx_for(SeedUser::Alice)
}

async fn setup(pool: PgPool, base_url: &str) -> (Arc<App>, SeedData) {
    let store = PgStore::new(pool);
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.config.base_url = base_url.into();
    (Arc::new(app), data)
}

async fn plan(app: &App) {
    let mut tx = app.store.begin_system().await.unwrap();
    let evs = tx.outbox().claim_undelivered("t", 1000).await.unwrap();
    drop(tx);
    for ev in evs {
        app.plan_webhook_deliveries(&ev).await.unwrap();
    }
}

fn issue() -> CreateIssueInput {
    CreateIssueInput {
        team: "ENG".into(),
        title: Some("Login kaputt".into()),
        ..Default::default()
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<WebhookRequest>>);

#[async_trait]
impl WebhookTransport for Recorder {
    async fn send(&self, req: WebhookRequest) -> TransportResult {
        self.0.lock().unwrap().push(req);
        TransportResult {
            status: Some(200),
            ..Default::default()
        }
    }
}

fn header<'a>(req: &'a WebhookRequest, name: &str) -> Option<&'a str> {
    req.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Meilensteine: Abschließen ist ein `update`, der Snapshot nennt Issues aller Teams und bleibt draußen.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn milestone_events_are_delivered_without_the_snapshot(pool: PgPool) {
    let (app, _) = setup(pool.clone(), "http://lineal.test").await;
    app.create_webhook(
        &alice(),
        CreateWebhookInput {
            url: "http://127.0.0.1:9/hook".into(),
            all_public_teams: true,
            resource_types: vec!["ProjectMilestone".into()],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (_, m) = app
        .save_milestone(
            &alice(),
            SaveMilestoneInput {
                project: "P-ENG-1".into(),
                name: Some("Beta".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let id = m.milestone.id.0.to_string();
    app.complete_milestone(&alice(), "P-ENG-1", &id, OpenIssues::Keep, Some("Fertig"))
        .await
        .unwrap();
    app.delete_milestone(&alice(), "P-ENG-1", &id)
        .await
        .unwrap();
    plan(&app).await;
    let rec = Recorder::default();
    assert_eq!(
        deliver_due(&app, &rec, app.clock.now(), 10).await.unwrap(),
        3
    );

    let reqs = rec.0.lock().unwrap().clone();
    let bodies: Vec<Value> = reqs
        .iter()
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect();
    let actions: Vec<_> = bodies
        .iter()
        .map(|v| v["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions, ["create", "update", "remove"]);
    let done = &bodies[1];
    assert_eq!(header(&reqs[1], "Lineal-Event"), Some("ProjectMilestone"));
    assert_eq!(done["type"], "ProjectMilestone");
    assert_eq!(done["data"]["name"], "Beta");
    assert_eq!(done["data"]["resultNote"], "Fertig");
    assert!(done["data"]["completedAt"].is_string());
    assert!(done["data"].get("snapshot").is_none());
    assert!(
        done["url"]
            .as_str()
            .unwrap()
            .ends_with(&format!("/milestone/{id}"))
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn delivery_is_signed_over_the_raw_body(pool: PgPool) {
    let (app, _) = setup(pool.clone(), "http://localhost:8080").await;
    let created = app
        .create_webhook(
            &alice(),
            CreateWebhookInput {
                url: "http://127.0.0.1:9/hook".into(),
                all_public_teams: true,
                resource_types: vec!["Issue".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let secret = created.secret.unwrap();
    app.create_issue(&alice(), issue()).await.unwrap();
    plan(&app).await;
    let rec = Recorder::default();
    assert_eq!(
        deliver_due(&app, &rec, app.clock.now(), 10).await.unwrap(),
        1
    );
    let reqs = rec.0.lock().unwrap().clone();
    let r = &reqs[0];
    assert_eq!(
        header(r, "Lineal-Signature"),
        Some(sign(&secret, &r.body).as_str())
    );
    assert_eq!(header(r, "Lineal-Event"), Some("Issue"));
    assert!(header(r, "Lineal-Delivery").is_some());
    let v: Value = serde_json::from_slice(&r.body).unwrap();
    assert_eq!(v["webhookId"], created.webhook.id.to_string());
    assert_eq!(
        header(r, "Lineal-Timestamp").unwrap(),
        v["webhookTimestamp"].to_string()
    );
    assert_eq!(v["action"], "create");
    assert_eq!(v["data"]["title"], "Login kaputt");
    let log: String = sqlx::query_scalar("SELECT status FROM webhook_delivery")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(log, "succeeded");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn rebinding_to_a_private_target_is_rejected_at_delivery(pool: PgPool) {
    let (app, _) = setup(pool.clone(), "https://lineal.example").await;
    let created = app
        .create_webhook(
            &alice(),
            CreateWebhookInput {
                url: "https://93.184.216.34/hook".into(),
                all_public_teams: true,
                resource_types: vec!["Issue".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    app.create_issue(&alice(), issue()).await.unwrap();
    plan(&app).await;
    // Das Ziel zeigt nach dem Speichern auf interne Adressen (DNS-Wechsel).
    for target in [
        "https://127.0.0.1/hook",
        "https://[::ffff:127.0.0.1]/hook",
        "https://169.254.169.254/latest",
    ] {
        sqlx::query("UPDATE webhook SET url = $2 WHERE id = $1")
            .bind(created.webhook.id.0)
            .bind(target)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE webhook_delivery SET status = 'pending', next_attempt_at = now() - interval '1 hour'")
            .execute(&pool)
            .await
            .unwrap();
        let rec = Recorder::default();
        let n = deliver_due(&app, &rec, chrono::Utc::now(), 10)
            .await
            .unwrap();
        assert_eq!(n, 1);
        assert!(
            rec.0.lock().unwrap().is_empty(),
            "{target}: kein Request darf rausgehen"
        );
    }
    let err: String = sqlx::query_scalar("SELECT error FROM webhook_delivery")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(err.contains("not allowed"), "{err}");
}

/// Minimaler HTTP-Empfänger: antwortet mit `status` (+ optional Location) und merkt sich Header/Body.
async fn receiver(
    status: u16,
    location: Option<&'static str>,
) -> (u16, Arc<Mutex<Vec<(String, Vec<u8>)>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let seen = seen2.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                        let len = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if buf.len() >= pos + 4 + len {
                            seen.lock()
                                .unwrap()
                                .push((head, buf[pos + 4..pos + 4 + len].to_vec()));
                            break;
                        }
                    }
                }
                let loc = location
                    .map(|l| format!("Location: {l}\r\n"))
                    .unwrap_or_default();
                let resp = format!(
                    "HTTP/1.1 {status} X\r\n{loc}Content-Length: 2\r\nConnection: close\r\n\r\nok"
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    (port, seen)
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn consumer_delivers_over_http_and_does_not_follow_redirects(pool: PgPool) {
    let (app, _) = setup(pool.clone(), "http://localhost:8080").await;
    let (ok_port, ok_seen) = receiver(200, None).await;
    let (redir_port, _) = receiver(302, Some("http://127.0.0.1:1/evil")).await;
    let hook = |port: u16| CreateWebhookInput {
        url: format!("http://127.0.0.1:{port}/hook"),
        all_public_teams: true,
        resource_types: vec!["Issue".into()],
        ..Default::default()
    };
    let a = app.create_webhook(&alice(), hook(ok_port)).await.unwrap();
    let b = app
        .create_webhook(&alice(), hook(redir_port))
        .await
        .unwrap();
    app.create_issue(&alice(), issue()).await.unwrap();
    // Über den echten Konsumenten (Plan + Versand).
    let mut tx = app.store.begin_system().await.unwrap();
    let evs = tx
        .outbox()
        .claim_undelivered("webhooks", 100)
        .await
        .unwrap();
    drop(tx);
    for ev in &evs {
        WebhooksConsumer.handle(&app, ev).await.unwrap();
    }
    let seen = ok_seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    let (head, body) = &seen[0];
    let sig = head
        .lines()
        .find_map(|l| {
            l.strip_prefix("lineal-signature: ")
                .or_else(|| l.strip_prefix("Lineal-Signature: "))
        })
        .expect("signature header");
    assert_eq!(sig.trim(), sign(&a.secret.unwrap(), body));
    let _ = b;
    let rows: Vec<(String, Option<i32>)> =
        sqlx::query_as("SELECT status, response_code FROM webhook_delivery ORDER BY status")
            .fetch_all(&pool)
            .await
            .unwrap();
    // 200 -> succeeded; 302 zählt als Fehlschlag (Redirect nicht gefolgt) und wird neu eingeplant.
    assert!(
        rows.contains(&("succeeded".to_string(), Some(200))),
        "{rows:?}"
    );
    assert!(
        rows.contains(&("pending".to_string(), Some(302))),
        "{rows:?}"
    );
}

/// Plant die neuen Outbox-Events und stellt die dadurch fälligen Zustellungen zu; liefert ihre Zahl.
/// Da erfolgreiche Zustellungen nicht erneut fällig werden, zählt der Rückgabewert genau die je
/// Schritt neu entstandenen Zustellungen.
async fn flush(app: &App, rec: &Recorder) -> usize {
    plan(app).await;
    deliver_due(app, rec, app.clock.now(), 50).await.unwrap()
}

/// LIN-77: Ein Hook mit `issue_status_changes_only` erhält Issue-`update`-Events nur bei echtem
/// Statuswechsel (`stateId`). Nicht-Status-Updates – auch per Bulk – bleiben draußen, während ein
/// gewöhnlicher Hook sie weiterhin erhält (Negativtest ist nicht leer). create bleibt unberührt.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn status_only_hook_skips_non_status_updates(pool: PgPool) {
    let (app, _) = setup(pool.clone(), "http://lineal.test").await;
    // Zwei Workspace-Hooks auf Issue; nur der erste filtert auf Statuswechsel.
    app.create_webhook(
        &alice(),
        CreateWebhookInput {
            url: "http://127.0.0.1:9/status".into(),
            all_public_teams: true,
            resource_types: vec!["Issue".into()],
            issue_status_changes_only: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    app.create_webhook(
        &alice(),
        CreateWebhookInput {
            url: "http://127.0.0.1:9/all".into(),
            all_public_teams: true,
            resource_types: vec!["Issue".into()],
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let rec = Recorder::default();
    let created = app.create_issue(&alice(), issue()).await.unwrap();
    let id = created.issue.id.to_string();

    // Schritt 1 – Anlage: beide Hooks erhalten das create-Event (Flag ist kein Filter für create).
    assert_eq!(flush(&app, &rec).await, 2, "create geht an beide Hooks");

    // Schritt 2 – Bulk-Änderung OHNE Statuswechsel (Priorität): nur der ungefilterte Hook.
    app.issue_batch_update(
        &alice(),
        BatchUpdateInput {
            ids: vec![id.clone()],
            patch: BatchPatch {
                priority: Some(1),
                ..Default::default()
            },
        },
    )
    .await
    .unwrap();
    assert_eq!(
        flush(&app, &rec).await,
        1,
        "Nicht-Status-Bulk-Update nur an den ungefilterten Hook"
    );

    // Schritt 3 – Statuswechsel: beide Hooks.
    app.update_issue(
        &alice(),
        UpdateIssueInput {
            id: id.clone(),
            state: Some("In Progress".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        flush(&app, &rec).await,
        2,
        "Statuswechsel geht an beide Hooks"
    );

    // Gesamtbild je Ziel-URL.
    let reqs = rec.0.lock().unwrap().clone();
    let by_path = |p: &str| -> Vec<Value> {
        reqs.iter()
            .filter(|r| r.url.path() == p)
            .map(|r| serde_json::from_slice(&r.body).unwrap())
            .collect()
    };
    let status = by_path("/status");
    let all = by_path("/all");
    // Gefilterter Hook: create + Statuswechsel, NICHT die Prioritätsänderung.
    let status_actions: Vec<&str> = status
        .iter()
        .map(|v| v["action"].as_str().unwrap())
        .collect();
    assert_eq!(status_actions, ["create", "update"], "{status:?}");
    assert_eq!(
        status[1]["data"]["state"]["name"], "In Progress",
        "das einzige Update am gefilterten Hook ist der Statuswechsel"
    );
    // Ungefilterter Hook: create + zwei Updates (Priorität und Status) – beweist, dass das
    // ausgefilterte Update real existierte und sonst zugestellt worden wäre.
    let all_updates = all.iter().filter(|v| v["action"] == "update").count();
    assert_eq!(all_updates, 2, "{all:?}");
}

/// Wie `receiver`, antwortet aber mit frei wählbarem Status plus `Retry-After`-Header und zählt die
/// eingegangenen Requests (für die 429-Burst-Prüfung über den echten `ReqwestTransport`).
async fn receiver_retry_after(status: u16, retry_after: &'static str) -> (u16, Arc<Mutex<usize>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(Mutex::new(0usize));
    let hits2 = hits.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let hits = hits2.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    let n = sock.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                        let len = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if buf.len() >= pos + 4 + len {
                            *hits.lock().unwrap() += 1;
                            break;
                        }
                    }
                }
                let resp = format!(
                    "HTTP/1.1 {status} X\r\nRetry-After: {retry_after}\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    (port, hits)
}

/// LIN-89 (echter Transport): Ein 429 mit `Retry-After` wird vom `ReqwestTransport` bis zur
/// Aufzeichnung durchgereicht und pausiert die ganze Subscription. Von mehreren gleichzeitig
/// fälligen Zustellungen desselben Hooks geht in der Pause nur die erste raus (Burst-Schutz), keine
/// erhöht den Strike-Zähler.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn http_429_from_real_transport_pauses_and_suppresses_siblings(pool: PgPool) {
    let (app, _) = setup(pool.clone(), "http://localhost:8080").await;
    let (port, hits) = receiver_retry_after(429, "120").await;
    app.create_webhook(
        &alice(),
        CreateWebhookInput {
            url: format!("http://127.0.0.1:{port}/hook"),
            all_public_teams: true,
            resource_types: vec!["Issue".into()],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    for _ in 0..3 {
        app.create_issue(&alice(), issue()).await.unwrap();
    }
    plan(&app).await;

    let t0 = app.clock.now();
    deliver_due(&app, &ReqwestTransport, t0, 50).await.unwrap();

    // Burst-Schutz: trotz 3 fälliger Zustellungen genau ein Request raus.
    assert_eq!(*hits.lock().unwrap(), 1, "nur die erste Zustellung sendet");

    // Header durchgereicht: Subscription ~120 s pausiert, kein Strike, alle Zustellungen pending.
    let (paused_until, failures): (Option<chrono::DateTime<chrono::Utc>>, i32) =
        sqlx::query_as("SELECT paused_until, failure_count FROM webhook")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (paused_until.expect("paused_until") - t0).num_seconds(),
        120
    );
    assert_eq!(failures, 0, "429 ist kein Strike");
    let statuses: Vec<String> = sqlx::query_scalar("SELECT status FROM webhook_delivery")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(statuses.len(), 3);
    assert!(statuses.iter().all(|s| s == "pending"), "{statuses:?}");
}
