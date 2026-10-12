#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP22: Security-Settings, Mitglieder-Admin, Rollen-Matrix und Sperren gegen echte Datenbank.

use chrono::{DateTime, Utc};
use lineal_app::context::Actor;
use lineal_app::ports::Clock;
use lineal_app::ports::Store;
use lineal_app::usecases::auth::{CreateApiKeyInput, LoginInput};
use lineal_app::usecases::invitation::InviteUsersInput;
use lineal_app::usecases::org::SecuritySettingsPatch;
use lineal_app::usecases::user::CreateUserInput;
use lineal_app::{App, ErrorKind, RequestCtx};
use lineal_db::PgStore;
use lineal_domain::enums::UserRole;
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
    app
}

fn kind<T>(r: Result<T, lineal_app::AppError>) -> ErrorKind {
    r.err().expect("expected an error").kind
}

fn with_role(base: &RequestCtx, role: UserRole) -> RequestCtx {
    let Actor::User { id, .. } = base.actor else {
        panic!("user ctx expected")
    };
    RequestCtx {
        actor: Actor::User { id, role },
        ..base.clone()
    }
}

fn patch(roles: &[(&str, UserRole)], domains: Option<&[&str]>) -> SecuritySettingsPatch {
    SecuritySettingsPatch {
        roles: roles.iter().map(|(k, r)| ((*k).to_owned(), *r)).collect(),
        allowed_invite_domains: domains.map(|d| d.iter().map(|s| (*s).to_owned()).collect()),
    }
}

fn invite(email: &str) -> InviteUsersInput {
    InviteUsersInput {
        emails: vec![email.into()],
        ..Default::default()
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn security_settings_read_write_and_validation(pool: PgPool) {
    let app = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let carol = ctx_for(SeedUser::Carol);

    // Lesen: Mitglieder ja, Gäste nein.
    let p = app.get_security_settings(&bob).await.unwrap();
    assert_eq!(p.invitations_role, UserRole::User);
    assert!(p.allowed_invite_domains.is_empty());
    assert_eq!(
        kind(app.get_security_settings(&carol).await),
        ErrorKind::Forbidden
    );

    // Schreiben: Member nein.
    assert_eq!(
        kind(
            app.update_security_settings(
                &bob,
                patch(&[("invitationsRole", UserRole::Admin)], None)
            )
            .await
        ),
        ErrorKind::Forbidden
    );

    // Owner: Rollen und Domains (normalisiert, dedupliziert).
    let p = app
        .update_security_settings(
            &alice,
            patch(
                &[
                    ("invitationsRole", UserRole::Admin),
                    ("importRole", UserRole::Owner),
                ],
                Some(&[" @ACME.example", "acme.example", "Sub.Acme.Example"]),
            ),
        )
        .await
        .unwrap();
    assert_eq!(p.invitations_role, UserRole::Admin);
    assert_eq!(p.import_role, UserRole::Owner);
    assert_eq!(
        p.allowed_invite_domains,
        ["acme.example", "sub.acme.example"]
    );
    // Unveränderte Felder bleiben; erneutes Lesen liefert dasselbe.
    assert_eq!(p.team_creation_role, UserRole::User);
    assert_eq!(app.get_security_settings(&bob).await.unwrap(), p);

    // Ungültige Eingaben mit field.
    for bad in [
        patch(&[("invitationsRole", UserRole::Guest)], None),
        patch(&[("adminManagementRole", UserRole::User)], None),
        patch(&[("noSuchRole", UserRole::User)], None),
        patch(&[], Some(&["not a domain"])),
        patch(&[], Some(&["localhost"])),
        patch(&[], Some(&["-a.example"])),
    ] {
        let e = app
            .update_security_settings(&alice, bad)
            .await
            .expect_err("must fail");
        assert_eq!(e.kind, ErrorKind::InvalidRequest);
        assert!(e.details.get("field").is_some(), "{e:?}");
    }
    let too_many: Vec<String> = (0..51).map(|i| format!("d{i}.example")).collect();
    let e = app
        .update_security_settings(
            &alice,
            SecuritySettingsPatch {
                roles: vec![],
                allowed_invite_domains: Some(too_many),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);

    // Leere Liste hebt die Beschränkung auf.
    let p = app
        .update_security_settings(&alice, patch(&[], Some(&[])))
        .await
        .unwrap();
    assert!(p.allowed_invite_domains.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn admin_may_change_settings_but_not_admin_management(pool: PgPool) {
    let app = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob_admin = with_role(&ctx_for(SeedUser::Bob), UserRole::Admin);
    app.set_user_role(&alice, "bob", UserRole::Admin)
        .await
        .unwrap();
    app.update_security_settings(
        &bob_admin,
        patch(&[("teamCreationRole", UserRole::Admin)], None),
    )
    .await
    .unwrap();
    assert_eq!(
        kind(
            app.update_security_settings(
                &bob_admin,
                patch(&[("adminManagementRole", UserRole::Owner)], None)
            )
            .await
        ),
        ErrorKind::Forbidden
    );
    // Der Owner bleibt immer berechtigt, auch wenn alles auf `owner` steht.
    let all_owner: Vec<(&str, UserRole)> = lineal_app::usecases::org::SECURITY_ROLE_KEYS
        .iter()
        .map(|k| (*k, UserRole::Owner))
        .collect();
    app.update_security_settings(&alice, patch(&all_owner, None))
        .await
        .unwrap();
    app.invite_users(&alice, invite("dana@acme.example"))
        .await
        .unwrap();
    assert_eq!(
        kind(
            app.invite_users(&bob_admin, invite("erin@acme.example"))
                .await
        ),
        ErrorKind::Forbidden
    );
    app.update_security_settings(
        &alice,
        patch(&[("adminManagementRole", UserRole::Admin)], None),
    )
    .await
    .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn invitations_role_and_allowed_domains_are_enforced(pool: PgPool) {
    let app = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    app.invite_users(&bob, invite("dana@other.example"))
        .await
        .unwrap();

    app.update_security_settings(
        &alice,
        patch(
            &[("invitationsRole", UserRole::Admin)],
            Some(&["acme.example"]),
        ),
    )
    .await
    .unwrap();
    // Bob (user) darf nicht mehr einladen.
    assert_eq!(
        kind(app.invite_users(&bob, invite("erin@acme.example")).await),
        ErrorKind::Forbidden
    );
    // Alice: nur erlaubte Domain.
    let e = app
        .invite_users(&alice, invite("erin@evil.example"))
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    assert_eq!(e.details["field"], "emails");
    // Gemischte Liste: nichts wird angelegt.
    let e = app
        .invite_users(
            &alice,
            InviteUsersInput {
                emails: vec!["ok@acme.example".into(), "no@evil.example".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    app.invite_users(&alice, invite("ERIN@Acme.Example"))
        .await
        .unwrap();
    // Sub-Domains zählen nicht als erlaubt.
    assert_eq!(
        kind(app.invite_users(&alice, invite("x@sub.acme.example")).await),
        ErrorKind::InvalidRequest
    );

    // create_user folgt derselben Regel.
    let new_user = |email: &str| CreateUserInput {
        name: "Dan".into(),
        email: email.into(),
        role: UserRole::User,
        password: None,
        teams: vec![],
    };
    let e = app
        .create_user(&alice, new_user("dan@evil.example"))
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    assert_eq!(e.details["field"], "email");
    app.create_user(&alice, new_user("dan@acme.example"))
        .await
        .unwrap();

    // Leere Liste: alle Domains.
    app.update_security_settings(&alice, patch(&[], Some(&[])))
        .await
        .unwrap();
    app.create_user(&alice, new_user("dora@evil.example"))
        .await
        .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn member_list_is_admin_only_and_shows_email_role_status(pool: PgPool) {
    let app = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let carol = ctx_for(SeedUser::Carol);
    assert_eq!(
        kind(app.list_members(&bob, false, 250).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.list_members(&carol, true, 250).await),
        ErrorKind::Forbidden
    );
    let rows = app.list_members(&alice, false, 250).await.unwrap();
    let alice_row = rows
        .iter()
        .find(|r| r.user.email == "alice@acme.example")
        .expect("alice listed");
    assert_eq!(alice_row.user.role, UserRole::Owner);
    assert!(rows.iter().any(|r| r.user.email == "bob@acme.example"));

    app.set_user_active(&alice, "bob", false).await.unwrap();
    let active_only = app.list_members(&alice, false, 250).await.unwrap();
    assert!(
        !active_only
            .iter()
            .any(|r| r.user.email == "bob@acme.example")
    );
    let all = app.list_members(&alice, true, 250).await.unwrap();
    let bob_row = all
        .iter()
        .find(|r| r.user.email == "bob@acme.example")
        .unwrap();
    assert!(!bob_row.user.active);
    // Ein Admin darf die Liste ebenfalls, ein Key ohne admin-Scope nicht.
    app.set_user_active(&alice, &SeedData::acme().bob.to_string(), true)
        .await
        .unwrap();
    app.set_user_role(&alice, "bob", UserRole::Admin)
        .await
        .unwrap();
    let bob_admin = with_role(&bob, UserRole::Admin);
    assert!(app.list_members(&bob_admin, false, 10).await.is_ok());
    let mut no_admin_scope = bob_admin.clone();
    no_admin_scope.scopes.admin = false;
    assert_eq!(
        kind(app.list_members(&no_admin_scope, false, 10).await),
        ErrorKind::Forbidden
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn role_matrix(pool: PgPool) {
    let app = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    // Member ändert nichts.
    assert_eq!(
        kind(app.set_user_role(&bob, "carol", UserRole::User).await),
        ErrorKind::Forbidden
    );
    // Niemand ändert die eigene Rolle (auch nicht der letzte Owner).
    assert_eq!(
        kind(app.set_user_role(&alice, "me", UserRole::Admin).await),
        ErrorKind::Forbidden
    );
    app.set_user_role(&alice, "bob", UserRole::Admin)
        .await
        .unwrap();
    let bob_admin = with_role(&bob, UserRole::Admin);
    // Admin darf Admin vergeben (adminManagementRole=admin), aber keine Owner-Rolle.
    app.set_user_role(&bob_admin, "carol", UserRole::User)
        .await
        .unwrap();
    app.set_user_role(&bob_admin, "carol", UserRole::Admin)
        .await
        .unwrap();
    assert_eq!(
        kind(
            app.set_user_role(&bob_admin, "carol", UserRole::Owner)
                .await
        ),
        ErrorKind::Forbidden
    );
    // Admin darf keinen Owner herabstufen oder sperren.
    assert_eq!(
        kind(app.set_user_role(&bob_admin, "alice", UserRole::User).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_active(&bob_admin, "alice", false).await),
        ErrorKind::Forbidden
    );
    // Mit adminManagementRole=owner darf der Admin keine Admins mehr vergeben/entziehen.
    app.update_security_settings(
        &alice,
        patch(&[("adminManagementRole", UserRole::Owner)], None),
    )
    .await
    .unwrap();
    assert_eq!(
        kind(app.set_user_role(&bob_admin, "carol", UserRole::User).await),
        ErrorKind::Forbidden
    );
    app.set_user_role(&alice, "carol", UserRole::User)
        .await
        .unwrap();
    // Owner-Beförderung nur durch Owner; danach gibt es zwei Owner, der erste ist herabstufbar.
    app.set_user_role(&alice, "carol", UserRole::Owner)
        .await
        .unwrap();
    let carol_owner = with_role(&ctx_for(SeedUser::Carol), UserRole::Owner);
    app.set_user_role(&carol_owner, "alice", UserRole::Admin)
        .await
        .unwrap();
    // Carol ist jetzt der letzte Owner: nicht selbst herabstufbar, nicht sperrbar.
    assert_eq!(
        kind(app.set_user_role(&carol_owner, "me", UserRole::Admin).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_active(&carol_owner, "me", false).await),
        ErrorKind::InvalidRequest
    );
    // Die herabgestufte Alice kann Carol (Owner) weder herab- noch sperren.
    let alice_admin = with_role(&alice, UserRole::Admin);
    assert_eq!(
        kind(
            app.set_user_role(&alice_admin, "carol", UserRole::User)
                .await
        ),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_active(&alice_admin, "carol", false).await),
        ErrorKind::Forbidden
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn suspending_kills_sessions_keys_and_blocks_login(pool: PgPool) {
    let app = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    app.set_user_password(&bob, "me", "correct-horse")
        .await
        .unwrap();
    let login = |email: &str| LoginInput {
        email: email.into(),
        password: "correct-horse".into(),
        ..Default::default()
    };
    let session = app.login(login("bob@acme.example")).await.unwrap();
    let key = app
        .create_api_key(
            &bob,
            CreateApiKeyInput {
                label: "ci".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(app.principal_for_token(&session.token).await.is_ok());
    assert!(app.principal_for_token(&key.key).await.is_ok());

    // Rechte: Member sperrt nicht; man sperrt sich nicht selbst; Key ohne admin-Scope nicht.
    assert_eq!(
        kind(app.set_user_active(&bob, "alice", false).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(app.set_user_active(&alice, "me", false).await),
        ErrorKind::InvalidRequest
    );
    let mut no_admin = alice.clone();
    no_admin.scopes.admin = false;
    assert_eq!(
        kind(app.set_user_active(&no_admin, "bob", false).await),
        ErrorKind::Forbidden
    );

    app.set_user_active(&alice, "bob", false).await.unwrap();
    assert_eq!(
        kind(app.principal_for_token(&session.token).await),
        ErrorKind::Unauthenticated
    );
    assert_eq!(
        kind(app.principal_for_token(&key.key).await),
        ErrorKind::Unauthenticated
    );
    assert_eq!(
        kind(app.login(login("bob@acme.example")).await),
        ErrorKind::Unauthenticated
    );

    // Nach der Entsperrung bleiben die alten Zugangsdaten tot; ein neuer Login geht.
    app.set_user_active(&alice, &SeedData::acme().bob.to_string(), true)
        .await
        .unwrap();
    assert_eq!(
        kind(app.principal_for_token(&session.token).await),
        ErrorKind::Unauthenticated
    );
    assert_eq!(
        kind(app.principal_for_token(&key.key).await),
        ErrorKind::Unauthenticated
    );
    let fresh = app.login(login("bob@acme.example")).await.unwrap();
    assert!(app.principal_for_token(&fresh.token).await.is_ok());
}
