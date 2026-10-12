#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP29: persönlicher Account – Profil, Passwort ändern, Sessions (echte Datenbank).

use chrono::{DateTime, Utc};
use lineal_app::context::Scopes;
use lineal_app::ports::{Clock, Store};
use lineal_app::usecases::account::UpdateViewerInput;
use lineal_app::usecases::attachment::PrepareUploadInput;
use lineal_app::usecases::auth::{LoginInput, hash_token};
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
        .change_own_password(&alice, Some(&hash), "wrong-old", "brand-new-pw")
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
            app.change_own_password(&alice, Some(&hash), "correct-horse", "short")
                .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            app.change_own_password(&alice, Some(&hash), "correct-horse", "correct-horse")
                .await
        ),
        ErrorKind::InvalidRequest
    );
    assert!(alive(&app, &other).await);

    // Erfolg: alle anderen Sessions weg (auch die von Bob bleibt unberührt), diese bleibt.
    let revoked = app
        .change_own_password(&alice, Some(&hash), "correct-horse", "brand-new-pw")
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
            app.change_own_password(&ctx, h.as_deref(), "correct-horse", "brand-new-pw")
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
