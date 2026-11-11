#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP29: persönlicher Account – Profil, Passwort ändern, Sessions (echte Datenbank).

use chrono::{DateTime, Utc};
use lineal_app::context::Scopes;
use lineal_app::ports::{Clock, Store};
use lineal_app::usecases::account::UpdateViewerInput;
use lineal_app::usecases::attachment::PrepareUploadInput;
use lineal_app::usecases::auth::{CreateApiKeyInput, LoginInput, hash_token};
use lineal_app::{App, ErrorKind, RequestCtx};
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;

struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

async fn setup(pool: PgPool) -> App {
    let store = PgStore::new(pool);
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.clock = Arc::new(SystemClock);
    for user in [SeedUser::Alice, SeedUser::Bob, SeedUser::Carol] {
        app.set_user_password(&ctx_for(user), "me", "correct-horse")
            .await
            .unwrap();
    }
    app
}

fn kind<T>(r: Result<T, lineal_app::AppError>) -> ErrorKind {
    r.err().expect("expected an error").kind
}

async fn login(app: &App, email: &str, password: &str) -> String {
    app.login(LoginInput {
        email: email.into(),
        password: password.into(),
        ..Default::default()
    })
    .await
    .unwrap()
    .token
}

async fn alive(app: &App, token: &str) -> bool {
    app.principal_for_token(token).await.is_ok()
}

async fn audit_actions(pool: &PgPool, action: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = $1")
        .bind(action)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn password_change_checks_the_old_password_and_signs_out_other_sessions(pool: PgPool) {
    let app = setup(pool.clone()).await;
    let alice = ctx_for(SeedUser::Alice);
    let here = login(&app, "alice@acme.example", "correct-horse").await;
    let other = login(&app, "alice@acme.example", "correct-horse").await;
    let bob_token = login(&app, "bob@acme.example", "correct-horse").await;
    let hash = hash_token(&here);

    // Falsches altes Passwort: Fehler, Audit-Eintrag, nichts widerrufen.
    let r = app
        .change_own_password(&alice, Some(&hash), "wrong-old", "brand-new-pw", false)
        .await;
    let e = r.err().unwrap();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    assert_eq!(e.details["field"], "currentPassword");
    assert_eq!(audit_actions(&pool, "user.password_change_failed").await, 1);
    assert!(alive(&app, &other).await);
    assert!(
        app.login(LoginInput {
            email: "alice@acme.example".into(),
            password: "brand-new-pw".into(),
            ..Default::default()
        })
        .await
        .is_err(),
        "the password is unchanged"
    );

    // Neue Regeln wie beim Anlegen: zu kurz, identisch.
    assert_eq!(
        kind(
            app.change_own_password(&alice, Some(&hash), "correct-horse", "short", false)
                .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            app.change_own_password(&alice, Some(&hash), "correct-horse", "correct-horse", false)
                .await
        ),
        ErrorKind::InvalidRequest
    );
    assert!(alive(&app, &other).await);

    // Erfolg: alle anderen Sessions weg (auch die von Bob bleibt unberührt), diese bleibt.
    let revoked = app
        .change_own_password(&alice, Some(&hash), "correct-horse", "brand-new-pw", false)
        .await
        .unwrap();
    assert_eq!(revoked, 1);
    assert!(!alive(&app, &other).await, "the old session is invalid");
    assert!(alive(&app, &here).await, "the current session stays");
    assert!(alive(&app, &bob_token).await, "other users are untouched");
    assert_eq!(audit_actions(&pool, "user.password_changed").await, 1);
    let _ = login(&app, "alice@acme.example", "brand-new-pw").await;
    assert!(
        app.login(LoginInput {
            email: "alice@acme.example".into(),
            password: "correct-horse".into(),
            ..Default::default()
        })
        .await
        .is_err(),
        "the old password no longer works"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn password_change_needs_a_browser_session_with_write_scope(pool: PgPool) {
    let app = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let token = login(&app, "alice@acme.example", "correct-horse").await;
    let hash = hash_token(&token);
    let change = |ctx: RequestCtx, h: Option<String>| {
        let app = &app;
        async move {
            app.change_own_password(&ctx, h.as_deref(), "correct-horse", "brand-new-pw", false)
                .await
        }
    };
    // API-Key/OAuth (keine Web-Session): verboten.
    let key_ctx = RequestCtx {
        via_session: false,
        ..alice.clone()
    };
    assert_eq!(
        kind(change(key_ctx, Some(hash.clone())).await),
        ErrorKind::Forbidden
    );
    // Nur Lesen: verboten.
    let ro = RequestCtx {
        scopes: Scopes::read_only(),
        ..alice.clone()
    };
    assert_eq!(
        kind(change(ro, Some(hash.clone())).await),
        ErrorKind::Forbidden
    );
    // Ohne Kennung der aktuellen Session: verboten.
    assert_eq!(
        kind(change(alice.clone(), None).await),
        ErrorKind::Forbidden
    );
    // System/App-Akteure haben keinen persönlichen Account.
    let system = RequestCtx::system(SeedData::acme().org, "req");
    assert_eq!(
        kind(change(system, Some(hash.clone())).await),
        ErrorKind::Forbidden
    );
    // Das Passwort ist in all dem unverändert.
    let _ = login(&app, "alice@acme.example", "correct-horse").await;
}

#[sqlx::test(migrations = "./migrations")]
async fn sessions_are_listed_and_revoked_only_for_the_owner(pool: PgPool) {
    let app = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let a1 = login(&app, "alice@acme.example", "correct-horse").await;
    let a2 = login(&app, "alice@acme.example", "correct-horse").await;
    let a3 = login(&app, "alice@acme.example", "correct-horse").await;
    let b1 = login(&app, "bob@acme.example", "correct-horse").await;
    let h1 = hash_token(&a1);

    let list = app.list_own_sessions(&alice, Some(&h1)).await.unwrap();
    assert_eq!(list.len(), 3, "only Alice's sessions");
    assert_eq!(list.iter().filter(|s| s.current).count(), 1);
    assert!(list.iter().any(|s| s.current && s.id != uuid::Uuid::nil()));
    let bob_list = app
        .list_own_sessions(&bob, Some(&hash_token(&b1)))
        .await
        .unwrap();
    assert_eq!(bob_list.len(), 1);
    let bob_session = bob_list[0].id;

    // Fremde Session-ID: not_found, Bobs Session lebt weiter.
    assert_eq!(
        kind(app.revoke_own_session(&alice, Some(&h1), bob_session).await),
        ErrorKind::NotFound
    );
    assert!(alive(&app, &b1).await);
    // Unbekannte ID.
    assert_eq!(
        kind(
            app.revoke_own_session(&alice, Some(&h1), uuid::Uuid::now_v7())
                .await
        ),
        ErrorKind::NotFound
    );
    // Die aktuelle Session wird hier nicht abgemeldet.
    let current = list.iter().find(|s| s.current).unwrap().id;
    assert_eq!(
        kind(app.revoke_own_session(&alice, Some(&h1), current).await),
        ErrorKind::InvalidRequest
    );
    assert!(alive(&app, &a1).await);

    // Eine andere eigene Session abmelden.
    let victim = list.iter().find(|s| !s.current).unwrap().id;
    app.revoke_own_session(&alice, Some(&h1), victim)
        .await
        .unwrap();
    assert_eq!(
        app.list_own_sessions(&alice, Some(&h1))
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        kind(app.revoke_own_session(&alice, Some(&h1), victim).await),
        ErrorKind::NotFound,
        "already revoked"
    );

    // Alle anderen abmelden.
    let n = app.revoke_other_sessions(&alice, Some(&h1)).await.unwrap();
    assert_eq!(n, 1);
    assert!(alive(&app, &a1).await);
    assert!(!alive(&app, &a2).await && !alive(&app, &a3).await);
    assert!(alive(&app, &b1).await, "Bob stays signed in");

    // Ohne Browser-Session geht nichts.
    let key_ctx = RequestCtx {
        via_session: false,
        ..alice.clone()
    };
    assert_eq!(
        kind(app.list_own_sessions(&key_ctx, Some(&h1)).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.revoke_other_sessions(&key_ctx, Some(&h1)).await),
        ErrorKind::Forbidden
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn profile_updates_validate_and_only_touch_the_caller(pool: PgPool) {
    let app = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);

    let u = app
        .update_viewer(
            &bob,
            UpdateViewerInput {
                name: Some("  Robert Example ".into()),
                display_name: Some("rob".into()),
                timezone: Some(Some("Europe/Berlin".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        (u.name.as_str(), u.display_name.as_str()),
        ("Robert Example", "rob")
    );
    assert_eq!(u.timezone.as_deref(), Some("Europe/Berlin"));
    // Alice ist unverändert.
    let a = app.get_viewer(&alice).await.unwrap();
    assert_eq!(a.name, "Alice Example");
    assert_eq!(a.display_name, "alice");
    // Zeitzone leeren.
    let u = app
        .update_viewer(
            &bob,
            UpdateViewerInput {
                timezone: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(u.timezone.is_none());

    let bad = |input: UpdateViewerInput| {
        let app = &app;
        let bob = bob.clone();
        async move { kind(app.update_viewer(&bob, input).await) }
    };
    // Anzeigename vergeben (auch in anderer Schreibweise), ungültig, reserviert.
    assert_eq!(
        bad(UpdateViewerInput {
            display_name: Some("alice".into()),
            ..Default::default()
        })
        .await,
        ErrorKind::Conflict
    );
    for d in ["Alice", "bad name", "", "me", "none", &"a".repeat(41)] {
        assert_eq!(
            bad(UpdateViewerInput {
                display_name: Some(d.into()),
                ..Default::default()
            })
            .await,
            ErrorKind::InvalidRequest,
            "{d:?}"
        );
    }
    // Name leer/zu lang, Zeitzone unbekannt.
    for n in ["", "   ", &"x".repeat(256)] {
        assert_eq!(
            bad(UpdateViewerInput {
                name: Some(n.into()),
                ..Default::default()
            })
            .await,
            ErrorKind::InvalidRequest
        );
    }
    assert_eq!(
        bad(UpdateViewerInput {
            timezone: Some(Some("Mars/Base".into())),
            ..Default::default()
        })
        .await,
        ErrorKind::InvalidRequest
    );
    // Der eigene Anzeigename in anderer Schreibweise ist erlaubt (kein Konflikt mit sich selbst).
    app.update_viewer(
        &bob,
        UpdateViewerInput {
            display_name: Some("rob".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // Nur Lesen: verboten.
    let ro = RequestCtx {
        scopes: Scopes::read_only(),
        ..bob.clone()
    };
    assert_eq!(
        kind(
            app.update_viewer(
                &ro,
                UpdateViewerInput {
                    name: Some("X".into()),
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::Forbidden
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn avatar_must_be_an_uploaded_image_of_this_workspace(pool: PgPool) {
    let app = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let upload = |name: &str, content_type: &str, size: i64| {
        let app = &app;
        let alice = alice.clone();
        let (name, content_type) = (name.to_owned(), content_type.to_owned());
        async move {
            app.prepare_attachment_upload(
                &alice,
                PrepareUploadInput {
                    issue: None,
                    filename: name,
                    content_type,
                    size,
                    title: None,
                    subtitle: None,
                },
            )
            .await
            .unwrap()
            .asset_url
        }
    };
    let set = |url: Option<String>| {
        let app = &app;
        let alice = alice.clone();
        async move {
            app.update_viewer(
                &alice,
                UpdateViewerInput {
                    avatar_url: Some(url),
                    ..Default::default()
                },
            )
            .await
        }
    };
    // Fremde URLs, Skripte und bloße Pfade: abgelehnt.
    for url in [
        "https://evil.example/a.png",
        "javascript:alert(1)",
        "/assets/x/y/z",
    ] {
        assert_eq!(
            kind(set(Some(url.into())).await),
            ErrorKind::InvalidRequest,
            "{url}"
        );
    }
    // Kein Bild / zu groß: abgelehnt.
    let pdf = upload("a.pdf", "application/pdf", 100).await;
    assert_eq!(kind(set(Some(pdf)).await), ErrorKind::InvalidRequest);
    let big = upload("big.png", "image/png", 6 * 1024 * 1024).await;
    assert_eq!(kind(set(Some(big)).await), ErrorKind::InvalidRequest);
    // Ein Bild: gesetzt, dann wieder entfernt.
    let ok = upload("me.png", "image/png", 2048).await;
    let u = set(Some(ok.clone())).await.unwrap();
    assert_eq!(u.avatar_url.as_deref(), Some(ok.as_str()));
    assert!(set(None).await.unwrap().avatar_url.is_none());
}

// --- S-21: Passwortwechsel widerruft App-Zugriffe (OAuth immer, API-Keys/Kalender optional) ---

async fn seed_oauth_token(pool: &PgPool, user: uuid::Uuid) -> uuid::Uuid {
    let org = SeedData::acme().org.0;
    let app_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO oauth_application (organization_id, name, client_secret_hash, redirect_uris, developer, developer_url, creator_id) \
         VALUES ($1,'TestApp','x',ARRAY['https://x.example/cb'],'dev','https://x.example',$2) RETURNING id",
    )
    .bind(org)
    .bind(user)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO oauth_token (organization_id, application_id, user_id, access_token_hash) \
         VALUES ($1,$2,$3,'tokhash') RETURNING id",
    )
    .bind(org)
    .bind(app_id)
    .bind(user)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn seed_calendar_feed(pool: &PgPool, user: uuid::Uuid) {
    sqlx::query(
        "INSERT INTO calendar_feed (organization_id, user_id, token_hash) VALUES ($1,$2,'calhash')",
    )
    .bind(SeedData::acme().org.0)
    .bind(user)
    .execute(pool)
    .await
    .unwrap();
}

async fn oauth_revoked(pool: &PgPool, token: uuid::Uuid) -> bool {
    sqlx::query_scalar("SELECT revoked_at IS NOT NULL FROM oauth_token WHERE id = $1")
        .bind(token)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn has_calendar(pool: &PgPool, user: uuid::Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM calendar_feed WHERE user_id = $1)")
        .bind(user)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn last_password_change_audit(pool: &PgPool) -> serde_json::Value {
    sqlx::query_scalar(
        "SELECT details FROM audit_log WHERE action = 'user.password_changed' ORDER BY occurred_at DESC, id DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn alice_api_key(app: &App) -> String {
    app.create_api_key(
        &ctx_for(SeedUser::Alice),
        CreateApiKeyInput {
            label: "ci".into(),
            scopes: vec!["read".into()],
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .key
}

#[sqlx::test(migrations = "./migrations")]
async fn password_change_revokes_oauth_and_keeps_other_credentials_by_default(pool: PgPool) {
    let app = setup(pool.clone()).await;
    let alice = ctx_for(SeedUser::Alice);
    let aid = SeedData::acme().alice.0;
    let here = login(&app, "alice@acme.example", "correct-horse").await;
    let key = alice_api_key(&app).await;
    let oauth = seed_oauth_token(&pool, aid).await;
    seed_calendar_feed(&pool, aid).await;
    assert!(alive(&app, &key).await, "Key gilt zunächst");

    app.change_own_password(
        &alice,
        Some(&hash_token(&here)),
        "correct-horse",
        "brand-new-pw",
        false,
    )
    .await
    .unwrap();

    assert!(
        oauth_revoked(&pool, oauth).await,
        "OAuth-Token wird immer widerrufen"
    );
    assert!(alive(&app, &key).await, "API-Key bleibt ohne Flag gültig");
    assert!(
        has_calendar(&pool, aid).await,
        "Kalender-Abo bleibt ohne Flag"
    );
    let d = last_password_change_audit(&pool).await;
    assert_eq!(d["revokedOauthTokens"], serde_json::json!(1));
    assert_eq!(d["revokedApiKeys"], serde_json::json!(0));
    assert_eq!(d["calendarFeedRevoked"], serde_json::json!(false));
}

#[sqlx::test(migrations = "./migrations")]
async fn password_change_revokes_api_keys_and_calendar_when_requested(pool: PgPool) {
    let app = setup(pool.clone()).await;
    let alice = ctx_for(SeedUser::Alice);
    let aid = SeedData::acme().alice.0;
    let here = login(&app, "alice@acme.example", "correct-horse").await;
    let key = alice_api_key(&app).await;
    let oauth = seed_oauth_token(&pool, aid).await;
    seed_calendar_feed(&pool, aid).await;

    app.change_own_password(
        &alice,
        Some(&hash_token(&here)),
        "correct-horse",
        "brand-new-pw",
        true,
    )
    .await
    .unwrap();

    assert!(oauth_revoked(&pool, oauth).await, "OAuth-Token widerrufen");
    assert!(!alive(&app, &key).await, "API-Key mit Flag widerrufen");
    assert!(
        !has_calendar(&pool, aid).await,
        "Kalender-Abo mit Flag entfernt"
    );
    let d = last_password_change_audit(&pool).await;
    assert_eq!(d["revokedApiKeys"], serde_json::json!(1));
    assert_eq!(d["calendarFeedRevoked"], serde_json::json!(true));
}

async fn insert_app(pool: &PgPool, creator: uuid::Uuid, name: &str) -> uuid::Uuid {
    sqlx::query_scalar(
        "INSERT INTO oauth_application (organization_id, name, client_secret_hash, redirect_uris, developer, developer_url, creator_id) \
         VALUES ($1,$2,'x',ARRAY['https://x.example/cb'],'dev','https://x.example',$3) RETURNING id",
    )
    .bind(SeedData::acme().org.0)
    .bind(name)
    .bind(creator)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_token(pool: &PgPool, app_id: uuid::Uuid, user: uuid::Uuid, scopes: &[&str]) {
    sqlx::query("INSERT INTO oauth_token (organization_id, application_id, user_id, access_token_hash, scopes) VALUES ($1,$2,$3,$4,$5)")
        .bind(SeedData::acme().org.0)
        .bind(app_id)
        .bind(user)
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(scopes.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn oauth_grants_group_per_app_and_revoke(pool: PgPool) {
    let app = setup(pool.clone()).await;
    let alice = ctx_for(SeedUser::Alice);
    let aid = SeedData::acme().alice.0;
    let app_a = insert_app(&pool, aid, "App A").await;
    let app_b = insert_app(&pool, aid, "App B").await;
    // App A: zwei Tokens (zwei Geräte), Scopes werden vereint. App B: eins.
    insert_token(&pool, app_a, aid, &["read"]).await;
    insert_token(&pool, app_a, aid, &["read", "write"]).await;
    insert_token(&pool, app_b, aid, &["read"]).await;

    let grants = app.list_oauth_grants(&alice).await.unwrap();
    assert_eq!(grants.len(), 2, "je App genau ein Grant");
    let a = grants.iter().find(|g| g.application_id == app_a).unwrap();
    assert_eq!(a.application_name, "App A");
    assert_eq!(
        a.scopes,
        vec!["read".to_string(), "write".to_string()],
        "Scopes vereint und sortiert"
    );

    // Widerruf von App A entfernt beide Tokens → nur noch App B.
    app.revoke_oauth_grant(&alice, app_a).await.unwrap();
    let grants = app.list_oauth_grants(&alice).await.unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].application_id, app_b);

    // Erneuter Widerruf ohne aktive Freigabe → not_found.
    assert_eq!(
        kind(app.revoke_oauth_grant(&alice, app_a).await),
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn login_ip_known_matches_a_recent_session_ip(pool: PgPool) {
    let app = setup(pool.clone()).await;
    // Erfolgreicher Login von einer IP legt eine Session mit dieser IP an.
    app.login(LoginInput {
        email: "alice@acme.example".into(),
        password: "correct-horse".into(),
        ip_address: Some("203.0.113.9".into()),
        ..Default::default()
    })
    .await
    .unwrap();
    let now = Utc::now();

    // Bekannte IP (ohne und mit passendem Workspace) → true.
    assert!(
        app.login_ip_known("alice@acme.example", Some("203.0.113.9"), None, now)
            .await
    );
    assert!(
        app.login_ip_known("alice@acme.example", Some("203.0.113.9"), Some("acme"), now)
            .await
    );
    // Fremder Workspace → false (Cross-Tenant-Trust ausgeschlossen).
    assert!(
        !app.login_ip_known(
            "alice@acme.example",
            Some("203.0.113.9"),
            Some("other"),
            now
        )
        .await
    );
    // Andere IP → false.
    assert!(
        !app.login_ip_known("alice@acme.example", Some("198.51.100.1"), None, now)
            .await
    );
    // Nicht existierende E-Mail → false (gleiche Abfrage, kein Timing-Orakel).
    assert!(
        !app.login_ip_known("nobody@acme.example", Some("203.0.113.9"), None, now)
            .await
    );
    // Ohne IP → false.
    assert!(
        !app.login_ip_known("alice@acme.example", None, None, now)
            .await
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn login_ip_known_requires_a_single_workspace_without_an_explicit_one(pool: PgPool) {
    let app = setup(pool.clone()).await;
    let ip = "203.0.113.9";
    let now = Utc::now();
    // Alices Session in acme von dieser IP.
    app.login(LoginInput {
        email: "alice@acme.example".into(),
        password: "correct-horse".into(),
        ip_address: Some(ip.into()),
        ..Default::default()
    })
    .await
    .unwrap();
    // Dieselbe E-Mail in einem zweiten Workspace, ebenfalls mit einer Session von dieser IP.
    let org2 = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO organization (id, name, url_key) VALUES ($1,'Other','other2')")
        .bind(org2)
        .execute(&pool)
        .await
        .unwrap();
    let user2 = uuid::Uuid::now_v7();
    sqlx::query("INSERT INTO \"user\" (id, organization_id, name, display_name, email) VALUES ($1,$2,'Alice Two','alicetwo','alice@acme.example')")
        .bind(user2)
        .bind(org2)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO user_session (organization_id, user_id, token_hash, expires_at, ip_address, created_at) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(org2)
        .bind(user2)
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(now + chrono::Duration::days(14))
        .bind(ip)
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();

    // Ohne Workspace: E-Mail jetzt in zwei Workspaces → keine Ausnahme.
    assert!(
        !app.login_ip_known("alice@acme.example", Some(ip), None, now)
            .await,
        "mehrdeutige E-Mail ohne Workspace darf keine bekannte IP ergeben"
    );
    // Mit explizitem Workspace bleibt die Ausnahme je Mandant korrekt.
    assert!(
        app.login_ip_known("alice@acme.example", Some(ip), Some("acme"), now)
            .await
    );
    assert!(
        app.login_ip_known("alice@acme.example", Some(ip), Some("other2"), now)
            .await
    );
}
