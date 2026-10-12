#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP25: Audit-Log gegen echte Datenbank (Seed `acme`): Einträge bei den protokollierten
//! Aktionen, keine Geheimnisse in den Details, Rechte, Paginierung, Org-Isolation,
//! Append-only-Trigger und Aufbewahrung.

use chrono::{DateTime, Duration, Utc};
use lineal_app::context::Actor;
use lineal_app::ports::Clock;
use lineal_app::ports::Store;
use lineal_app::ports::repos::{
    AuditActorKind, AuditCursor, AuditEntry, AuditFilter, NewAuditEntry,
};
use lineal_app::usecases::audit::{AUDIT_RETENTION_DEFAULT_DAYS, ListAuditInput};
use lineal_app::usecases::auth::{CreateApiKeyInput, LoginInput};
use lineal_app::usecases::invitation::{AcceptInvitationInput, InviteUsersInput};
use lineal_app::usecases::org::{CreateWorkspaceInput, SecuritySettingsPatch};
use lineal_app::usecases::team::CreateTeamInput;
use lineal_app::usecases::team_lifecycle::SubTeamHandling;
use lineal_app::usecases::webhook::CreateWebhookInput;
use lineal_app::{App, ErrorKind, RequestCtx};
use lineal_db::PgStore;
use lineal_domain::enums::UserRole;
use lineal_domain::ids::{OrgId, UserId};
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

const ALICE_PASSWORD: &str = "correct-horse";
const SECRET_PATH: &str = "shared-webhook-secret-1234";

struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

struct Fx {
    app: App,
    pool: PgPool,
    data: SeedData,
}

async fn setup(pool: PgPool) -> Fx {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.clock = Arc::new(SystemClock);
    app.config.base_url = "https://lineal.example".into();
    Fx { app, pool, data }
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

/// Vollständiger Log des Workspaces (große Seite, mehrere Abfragen falls nötig).
async fn all_entries(app: &App, ctx: &RequestCtx) -> Vec<AuditEntry> {
    let mut out = Vec::new();
    let mut after = None;
    loop {
        let page = app
            .list_audit_log(
                ctx,
                ListAuditInput {
                    first: Some(100),
                    after,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        out.extend(page.entries);
        match page.end_cursor {
            Some(c) if page.has_next_page => after = Some(c),
            _ => break,
        }
    }
    out
}

fn actions(entries: &[AuditEntry]) -> Vec<String> {
    entries.iter().map(|e| e.action.clone()).collect()
}

fn user_ref(id: UserId) -> String {
    id.0.to_string()
}

fn invite(email: &str) -> InviteUsersInput {
    InviteUsersInput {
        emails: vec![email.into()],
        ..Default::default()
    }
}

fn secret_patch(roles: &[(&str, UserRole)], days: Option<i64>) -> SecuritySettingsPatch {
    SecuritySettingsPatch {
        roles: roles.iter().map(|(k, r)| ((*k).to_owned(), *r)).collect(),
        allowed_invite_domains: None,
        audit_log_retention_days: days,
    }
}

/// Alle Aktionen des Hauptszenarios; danach ist der Log gefüllt.
async fn exercise_all_actions(fx: &Fx) -> (String, String) {
    let app = &fx.app;
    let alice_ctx = alice();
    // Seed-Hash ist kein Testkit-Hash: Passwort zuerst setzen (wie in identity.rs).
    app.set_user_password(&alice_ctx, "me", ALICE_PASSWORD)
        .await
        .unwrap();

    // Login erfolgreich (mit IP) und fehlgeschlagen (Konto existiert); unbekannte E-Mail ohne Eintrag.
    let login = app
        .login(LoginInput {
            email: "alice@acme.example".into(),
            password: ALICE_PASSWORD.into(),
            ip_address: Some("203.0.113.7".into()),
            request_id: Some("req_login_test".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    let _ = app
        .login(LoginInput {
            email: "alice@acme.example".into(),
            password: "wrong-horse-password".into(),
            ..Default::default()
        })
        .await
        .expect_err("wrong password");
    let _ = app
        .login(LoginInput {
            email: "nobody@acme.example".into(),
            password: "whatever-password".into(),
            ..Default::default()
        })
        .await
        .expect_err("unknown account");

    // Security-Settings (vorher/nachher, Aufbewahrung).
    app.update_security_settings(
        &alice_ctx,
        secret_patch(&[("invitationsRole", UserRole::Admin)], Some(90)),
    )
    .await
    .unwrap();

    // Rolle und Sperren (Carol ist Gast; Bob wird Admin).
    app.set_user_role(&alice_ctx, &user_ref(fx.data.bob), UserRole::Admin)
        .await
        .unwrap();
    app.set_user_active(&alice_ctx, &user_ref(fx.data.carol), false)
        .await
        .unwrap();
    app.set_user_active(&alice_ctx, &user_ref(fx.data.carol), true)
        .await
        .unwrap();

    // Einladung: erstellt, erneut gesendet, widerrufen; zweite Einladung angenommen.
    let created = app
        .invite_users(&alice_ctx, invite("dave@acme.example"))
        .await
        .unwrap();
    let inv_id = created[0].invitation.id;
    app.resend_invitation(&alice_ctx, inv_id).await.unwrap();
    app.revoke_invitation(&alice_ctx, inv_id).await.unwrap();
    let accepted_token = app
        .invite_users(&alice_ctx, invite("erin@acme.example"))
        .await
        .unwrap()
        .remove(0)
        .token;
    app.accept_invitation(AcceptInvitationInput {
        token: accepted_token,
        email: "erin@acme.example".into(),
        name: Some("Erin".into()),
        password: Some("erin-password-1".into()),
    })
    .await
    .unwrap();

    // API-Key: erstellt (Klartext nur in der Antwort), widerrufen.
    let key = app
        .create_api_key(
            &alice_ctx,
            CreateApiKeyInput {
                label: "CI".into(),
                scopes: vec![],
                teams: None,
                expires_at: None,
            },
        )
        .await
        .unwrap();
    app.revoke_api_key(&alice_ctx, key.record.id).await.unwrap();

    // Team angelegt, archiviert, gelöscht.
    app.create_team(
        &alice_ctx,
        CreateTeamInput {
            name: "Audit Team".into(),
            key: Some("AUD".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    app.retire_team(
        &alice_ctx,
        "AUD",
        None,
        lineal_app::usecases::team_lifecycle::OpenIssuesOnRetirement::default(),
    )
    .await
    .unwrap();
    app.delete_team(&alice_ctx, "AUD", None::<SubTeamHandling>)
        .await
        .unwrap();

    // Webhook angelegt (mit geheimem Pfad) und gelöscht.
    let hook = app
        .create_webhook(
            &alice_ctx,
            CreateWebhookInput {
                label: Some("CI".into()),
                url: format!("https://93.184.216.34/{SECRET_PATH}?token=query-secret"),
                team: None,
                all_public_teams: true,
                resource_types: vec!["Issue".into()],
            },
        )
        .await
        .unwrap();
    app.delete_webhook(&alice_ctx, hook.webhook.id)
        .await
        .unwrap();

    // Logout der Session.
    assert!(
        app.logout(&login.token, Some("203.0.113.7".into()), None)
            .await
            .unwrap()
    );

    (key.key, login.token)
}

#[sqlx::test(migrations = "./migrations")]
async fn entries_are_written_for_audited_actions(pool: PgPool) {
    let fx = setup(pool).await;
    let (key_secret, session_token) = exercise_all_actions(&fx).await;
    let entries = all_entries(&fx.app, &alice()).await;
    let acts = actions(&entries);
    for expected in [
        "auth.login_succeeded",
        "auth.login_failed",
        "auth.logout",
        "workspace.security_settings_updated",
        "user.role_changed",
        "user.suspended",
        "user.reactivated",
        "invitation.created",
        "invitation.resent",
        "invitation.revoked",
        "invitation.accepted",
        "api_key.created",
        "api_key.revoked",
        "team.created",
        "team.archived",
        "team.deleted",
        "webhook.created",
        "webhook.deleted",
    ] {
        assert!(
            acts.iter().any(|a| a == expected),
            "missing {expected}: {acts:?}"
        );
    }
    // Unbekannte E-Mail: kein Eintrag (nur ein Fehlversuch mit Konto zählt).
    let failed: Vec<&AuditEntry> = entries
        .iter()
        .filter(|e| e.action == "auth.login_failed")
        .collect();
    assert_eq!(failed.len(), 1, "{acts:?}");
    assert_eq!(failed[0].details["reason"], "invalid_password");
    assert_eq!(failed[0].actor_user_id, Some(fx.data.alice));

    // Erfolgreicher Login: Session, IP und Request-ID.
    let login = entries
        .iter()
        .find(|e| e.action == "auth.login_succeeded" && e.actor_user_id == Some(fx.data.alice))
        .unwrap();
    assert_eq!(login.credential_kind.as_deref(), Some("session"));
    assert_eq!(login.ip.as_deref(), Some("203.0.113.7"));
    assert_eq!(login.request_id, "req_login_test");
    assert_eq!(login.actor_kind, "user");

    // Security-Settings: vorher/nachher je Einstellung.
    let sec = entries
        .iter()
        .find(|e| e.action == "workspace.security_settings_updated")
        .unwrap();
    assert_eq!(sec.details["changes"]["invitationsRole"]["from"], "user");
    assert_eq!(sec.details["changes"]["invitationsRole"]["to"], "admin");
    assert_eq!(sec.details["changes"]["auditLogRetentionDays"]["from"], 365);
    assert_eq!(sec.details["changes"]["auditLogRetentionDays"]["to"], 90);

    // Rollenwechsel mit alter und neuer Rolle.
    let role = entries
        .iter()
        .find(|e| e.action == "user.role_changed")
        .unwrap();
    assert_eq!(role.target_id, Some(fx.data.bob.0));
    assert_eq!(role.details["from"], "user");
    assert_eq!(role.details["to"], "admin");

    // API-Key: nur Präfix, nie der Klartext.
    let key_created = entries
        .iter()
        .find(|e| e.action == "api_key.created")
        .unwrap();
    assert!(
        key_created.details["key_prefix"]
            .as_str()
            .unwrap()
            .starts_with("lin_api_")
    );
    assert_eq!(
        key_created.details["key_prefix"].as_str().unwrap().len(),
        12
    );

    // Webhook: nur der Host, kein Pfad, kein Query, kein Secret.
    let hook = entries
        .iter()
        .find(|e| e.action == "webhook.created")
        .unwrap();
    assert_eq!(hook.details["host"], "93.184.216.34");

    // Keine Geheimnisse in irgendeinem Eintrag (Klartext-Key, Session-Token, Passwörter, Webhook-Pfad).
    let blob = entries
        .iter()
        .map(|e| {
            format!(
                "{} {} {}",
                e.action,
                e.details,
                e.ip.clone().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    for secret in [
        key_secret.as_str(),
        session_token.as_str(),
        ALICE_PASSWORD,
        "wrong-horse-password",
        "erin-password-1",
        SECRET_PATH,
        "query-secret",
    ] {
        assert!(
            !blob.contains(secret),
            "secret `{secret}` leaked into audit log"
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn only_admins_with_admin_scope_can_read(pool: PgPool) {
    let fx = setup(pool).await;
    exercise_all_actions(&fx).await;
    // Owner mit Admin-Scope: ja.
    assert!(
        fx.app
            .list_audit_log(&alice(), ListAuditInput::default())
            .await
            .is_ok()
    );
    // Gast: forbidden.
    assert_eq!(
        kind(
            fx.app
                .list_audit_log(&carol(), ListAuditInput::default())
                .await
        ),
        ErrorKind::Forbidden
    );
    // Owner ohne Admin-Scope (z. B. API-Key mit read/write): forbidden.
    let mut no_admin = alice();
    no_admin.scopes.admin = false;
    assert_eq!(
        kind(
            fx.app
                .list_audit_log(&no_admin, ListAuditInput::default())
                .await
        ),
        ErrorKind::Forbidden
    );
    // Ohne Read-Scope ebenfalls.
    let mut no_read = alice();
    no_read.scopes.read = false;
    assert_eq!(
        kind(
            fx.app
                .list_audit_log(&no_read, ListAuditInput::default())
                .await
        ),
        ErrorKind::Forbidden
    );
    // App-Akteur: forbidden.
    let app_ctx = RequestCtx {
        actor: Actor::App { id: fx.data.alice },
        ..alice()
    };
    assert_eq!(
        kind(
            fx.app
                .list_audit_log(&app_ctx, ListAuditInput::default())
                .await
        ),
        ErrorKind::Forbidden
    );
    // Ein Mitglied mit Rolle User (vor der Beförderung in exercise_all_actions) ist ausgeschlossen.
    let member = RequestCtx {
        actor: Actor::User {
            id: fx.data.bob,
            role: UserRole::User,
        },
        ..bob()
    };
    assert_eq!(
        kind(
            fx.app
                .list_audit_log(&member, ListAuditInput::default())
                .await
        ),
        ErrorKind::Forbidden
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn pagination_filters_and_limits(pool: PgPool) {
    let fx = setup(pool).await;
    exercise_all_actions(&fx).await;
    let ctx = alice();
    let all = all_entries(&fx.app, &ctx).await;
    assert!(all.len() > 5);
    // Seitengröße und Cursor: zusammengesetzt ergibt der Log dieselbe Folge.
    let mut walked = Vec::new();
    let mut after = None;
    loop {
        let page = fx
            .app
            .list_audit_log(
                &ctx,
                ListAuditInput {
                    first: Some(2),
                    after,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(page.entries.len() <= 2);
        walked.extend(page.entries.iter().map(|e| e.id));
        if !page.has_next_page {
            break;
        }
        // Cursor über den Text-Umweg (wie in GraphQL).
        let text = page.end_cursor.unwrap().encode();
        after = Some(AuditCursor::decode(&text).unwrap());
    }
    assert_eq!(walked, all.iter().map(|e| e.id).collect::<Vec<_>>());
    // Neueste zuerst.
    assert!(all.windows(2).all(|w| w[0].occurred_at >= w[1].occurred_at));

    // Grenzen: 0 und 101 sind ungültig.
    for bad in [0, 101] {
        assert_eq!(
            kind(
                fx.app
                    .list_audit_log(
                        &ctx,
                        ListAuditInput {
                            first: Some(bad),
                            ..Default::default()
                        }
                    )
                    .await
            ),
            ErrorKind::InvalidRequest
        );
    }
    // Filter Aktion.
    let roles = fx
        .app
        .list_audit_log(
            &ctx,
            ListAuditInput {
                action: Some("user.role_changed".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(roles.entries.len(), 1);
    // Filter Akteur: Alice hat alles ausgelöst, Bob nichts.
    let by_bob = fx
        .app
        .list_audit_log(
            &ctx,
            ListAuditInput {
                actor_user_id: Some(fx.data.bob),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(by_bob.entries.is_empty());
    // Zeitraum: since in der Zukunft liefert nichts; since >= until ist ungültig.
    let future = Utc::now() + Duration::days(1);
    let none = fx
        .app
        .list_audit_log(
            &ctx,
            ListAuditInput {
                since: Some(future),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(none.entries.is_empty());
    assert_eq!(
        kind(
            fx.app
                .list_audit_log(
                    &ctx,
                    ListAuditInput {
                        since: Some(future),
                        until: Some(future - Duration::days(1)),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn workspaces_are_isolated(pool: PgPool) {
    let fx = setup(pool).await;
    exercise_all_actions(&fx).await;
    // Zweiter Workspace mit eigenem Owner.
    let created = fx
        .app
        .create_workspace(CreateWorkspaceInput {
            name: "Beta".into(),
            url_key: Some("beta-audit".into()),
            owner_name: "Dana".into(),
            owner_email: "dana@beta.example".into(),
            owner_password: None,
            credential_kind: Some("cli"),
        })
        .await
        .unwrap();
    let beta_org: OrgId = created.org.id;
    assert_ne!(beta_org, fx.data.org);
    let dana = RequestCtx {
        org_id: beta_org,
        actor: Actor::User {
            id: created.owner.id,
            role: UserRole::Owner,
        },
        request_id: "req_dana".into(),
        scopes: lineal_app::Scopes::all(),
        team_scope: None,
        via_session: true,
        client_ip: None,
        credential_kind: Some("session"),
    };

    // Admin von Workspace A sieht nichts aus B.
    let a_entries = all_entries(&fx.app, &alice()).await;
    assert!(a_entries.iter().all(|e| e.organization_id == fx.data.org));
    // Admin von B sieht nur den eigenen Workspace (Anlage per CLI) und nichts aus A.
    let b_entries = all_entries(&fx.app, &dana).await;
    assert!(!b_entries.is_empty());
    assert!(b_entries.iter().all(|e| e.organization_id == beta_org));
    let created_entry = b_entries
        .iter()
        .find(|e| e.action == "workspace.created")
        .expect("workspace.created in B");
    assert_eq!(created_entry.credential_kind.as_deref(), Some("cli"));
    // Der Filter kann keinen fremden Akteur hervorlocken.
    let foreign = fx
        .app
        .list_audit_log(
            &dana,
            ListAuditInput {
                actor_user_id: Some(fx.data.alice),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(foreign.entries.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn append_only_trigger_blocks_update_delete_truncate(pool: PgPool) {
    let fx = setup(pool.clone()).await;
    exercise_all_actions(&fx).await;
    let upd = sqlx::query("UPDATE audit_log SET action = 'changed'")
        .execute(&pool)
        .await
        .expect_err("update must fail");
    assert!(upd.to_string().contains("append-only"), "{upd}");
    let del = sqlx::query("DELETE FROM audit_log")
        .execute(&pool)
        .await
        .expect_err("delete must fail");
    assert!(del.to_string().contains("append-only"), "{del}");
    let trunc = sqlx::query("TRUNCATE audit_log").execute(&pool).await;
    assert!(trunc.is_err(), "truncate must fail");
    // Der Log ist unverändert.
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(n > 5);
}

#[sqlx::test(migrations = "./migrations")]
async fn retention_purges_only_expired_entries(pool: PgPool) {
    let fx = setup(pool).await;
    let ctx = alice();
    // Default: 365 Tage.
    let p = fx.app.get_security_settings(&ctx).await.unwrap();
    assert_eq!(p.audit_log_retention_days, AUDIT_RETENTION_DEFAULT_DAYS);

    // Validierung: Minimum 30, Maximum 3650.
    for bad in [29, 3651] {
        assert_eq!(
            kind(
                fx.app
                    .update_security_settings(&ctx, secret_patch(&[], Some(bad)))
                    .await
            ),
            ErrorKind::InvalidRequest
        );
    }

    // Alte und neue Einträge direkt schreiben (Zeitstempel in der Vergangenheit).
    let now = Utc::now();
    let mut tx = fx.app.store.begin(fx.data.org).await.unwrap();
    for (days, action) in [(400, "old.entry"), (200, "mid.entry"), (5, "new.entry")] {
        tx.audit()
            .append(&NewAuditEntry {
                id: Uuid::new_v4(),
                organization_id: fx.data.org,
                occurred_at: now - Duration::days(days),
                actor_user_id: None,
                actor_kind: AuditActorKind::System,
                credential_kind: None,
                action: action.into(),
                target_type: "test".into(),
                target_id: None,
                details: serde_json::json!({}),
                ip: None,
                request_id: String::new(),
            })
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();

    // Mit 365 Tagen: nur der 400-Tage-Eintrag fällt weg.
    let deleted = fx.app.purge_audit_log(now).await.unwrap();
    assert_eq!(deleted, 1);
    let names = actions(&all_entries(&fx.app, &ctx).await);
    assert!(!names.iter().any(|a| a == "old.entry"));
    assert!(names.iter().any(|a| a == "mid.entry"));

    // Auf 90 Tage gesetzt (wie im Hauptszenario) fällt auch der 200-Tage-Eintrag weg.
    fx.app
        .update_security_settings(&ctx, secret_patch(&[], Some(90)))
        .await
        .unwrap();
    assert_eq!(fx.app.purge_audit_log(now).await.unwrap(), 1);
    let names = actions(&all_entries(&fx.app, &ctx).await);
    assert!(!names.iter().any(|a| a == "mid.entry"));
    assert!(names.iter().any(|a| a == "new.entry"));

    // Außerhalb der Aufbewahrungsfunktion bleibt das Löschen gesperrt.
    let direct = sqlx::query("DELETE FROM audit_log WHERE action = 'new.entry'")
        .execute(&fx.pool)
        .await;
    assert!(direct.is_err());
}

#[sqlx::test(migrations = "./migrations")]
async fn filter_by_store_uses_org(pool: PgPool) {
    // Repo-Ebene: Abfrage liefert nur Einträge der Transaktions-Org.
    let fx = setup(pool).await;
    exercise_all_actions(&fx).await;
    let store = PgStore::new(fx.pool.clone());
    let mut tx = store.begin(fx.data.org).await.unwrap();
    let rows = tx
        .audit()
        .list(&AuditFilter::default(), None, 500)
        .await
        .unwrap();
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|r| r.organization_id == fx.data.org));
    let mut other = store.begin(OrgId(Uuid::new_v4())).await.unwrap();
    assert!(
        other
            .audit()
            .list(&AuditFilter::default(), None, 500)
            .await
            .unwrap()
            .is_empty()
    );
}
