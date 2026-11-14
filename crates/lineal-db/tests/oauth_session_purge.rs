#![allow(clippy::unwrap_used, clippy::expect_used)]
//! LIN-62 L-9: `delete_expired_sessions`, `oauth_delete_expired_codes` und
//! `oauth_delete_stale_registrations` löschen nur Altlasten, nie gültige/benutzte Zeilen.

use chrono::{Duration, Utc};
use lineal_app::ports::Store;
use lineal_db::PgStore;
use lineal_testkit::{SeedData, seed};
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn purge_deletes_expired_and_stale_oauth_rows_only(pool: PgPool) {
    let d = SeedData::acme();
    let store = PgStore::new(pool.clone());
    {
        let mut tx = store.begin(d.org).await.unwrap();
        seed::run(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
    }
    let now = Utc::now();
    let org = d.org.0;
    let alice = d.alice.0;

    // Sessions: abgelaufen / gültig / widerrufen-aber-noch-gültig.
    for (tok, exp, revoked) in [
        (
            "sess-expired",
            now - Duration::hours(1),
            None::<chrono::DateTime<Utc>>,
        ),
        ("sess-valid", now + Duration::hours(1), None),
        ("sess-revoked-valid", now + Duration::hours(1), Some(now)),
    ] {
        sqlx::query("INSERT INTO user_session (organization_id, user_id, token_hash, expires_at, revoked_at) VALUES ($1,$2,$3,$4,$5)")
            .bind(org).bind(alice).bind(tok).bind(exp).bind(revoked)
            .execute(&pool).await.unwrap();
    }

    // DCR-Registrierungen (vor der App wegen Fremdschlüssel): alt-ohne-App / frisch-ohne-App /
    // alt-mit-App.
    for (cid, created) in [
        ("stale-reg", now - Duration::hours(25)),
        ("fresh-reg", now - Duration::hours(1)),
        ("used-reg", now - Duration::hours(25)),
    ] {
        sqlx::query("INSERT INTO oauth_client_registration (client_id, client_name, redirect_uris, created_at) VALUES ($1,'C',ARRAY['https://a/cb'],$2)")
            .bind(cid).bind(created).execute(&pool).await.unwrap();
    }
    let app_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO oauth_application (organization_id, name, client_secret_hash, redirect_uris, developer, developer_url, creator_id, dcr_client_id) \
         VALUES ($1,'App','x',ARRAY['https://a/cb'],'Dev','https://d',$2,'used-reg') RETURNING id")
        .bind(org).bind(alice).fetch_one(&pool).await.unwrap();

    // Auth-Codes: abgelaufen / gültig.
    for (ch, exp) in [
        ("code-expired", now - Duration::hours(1)),
        ("code-valid", now + Duration::hours(1)),
    ] {
        sqlx::query("INSERT INTO oauth_authorization_code (code_hash, organization_id, application_id, user_id, redirect_uri, scopes, code_challenge, code_challenge_method, family_id, expires_at) \
             VALUES ($1,$2,$3,$4,'https://a/cb',ARRAY['read'],'chal','S256',$5,$6)")
            .bind(ch).bind(org).bind(app_id).bind(alice).bind(uuid::Uuid::new_v4()).bind(exp)
            .execute(&pool).await.unwrap();
    }

    // Aufräumen über die Repo-Methoden (system-weit, wie der Purge-Job).
    let mut tx = store.begin_system().await.unwrap();
    let sessions = tx.auth().delete_expired_sessions(now).await.unwrap();
    let codes = tx.webhooks().oauth_delete_expired_codes(now).await.unwrap();
    let regs = tx
        .webhooks()
        .oauth_delete_stale_registrations(now - Duration::hours(24))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        (sessions, codes, regs),
        (1, 1, 1),
        "je genau eine Altlast gelöscht"
    );

    // Übrig bleiben die gültigen bzw. benutzten.
    let left_sessions: Vec<String> =
        sqlx::query_scalar("SELECT token_hash FROM user_session ORDER BY token_hash")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(left_sessions, vec!["sess-revoked-valid", "sess-valid"]);
    let left_codes: Vec<String> =
        sqlx::query_scalar("SELECT code_hash FROM oauth_authorization_code")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(left_codes, vec!["code-valid"]);
    let left_regs: Vec<String> =
        sqlx::query_scalar("SELECT client_id FROM oauth_client_registration ORDER BY client_id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(left_regs, vec!["fresh-reg", "used-reg"]);
}
