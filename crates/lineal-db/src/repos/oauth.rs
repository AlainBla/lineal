//! OAuth-Persistenz (WP16): Client-Registrierung, Codes, Tokens. Inhärente Methoden von `PgTx`,
//! die `WebhookRepo` (siehe `webhook.rs`) weiterleitet.

use crate::db_err;
use crate::store::PgTx;
use chrono::{DateTime, Utc};
use lineal_app::AppResult;
use lineal_app::ports::repos::webhook::{
    AuthCodeRecord, ClientRegistration, NewAuthCode, NewOauthToken, OauthTokenRecord, TokenRotation,
};
use lineal_domain::ids::{OrgId, UserId};
use uuid::Uuid;

/// Spiegel-Zeile des Clients im Workspace; `client_secret_hash` gehört zu einem nie ausgegebenen
/// Zufallswert (dynamisch registrierte Clients sind öffentlich).
fn unusable_secret_hash() -> String {
    // SHA-256 über zwei Zufalls-UUIDs; nur als Platzhalter für die NOT-NULL-Spalte.
    use std::fmt::Write;
    let mut s = String::new();
    let _ = write!(s, "unused:{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    s
}

impl PgTx {
    pub(crate) async fn oauth_register_client_impl(
        &mut self,
        c: &ClientRegistration,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO oauth_client_registration (client_id, client_name, redirect_uris, created_at) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&c.client_id)
        .bind(&c.client_name)
        .bind(&c.redirect_uris)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    pub(crate) async fn oauth_find_client_impl(
        &mut self,
        client_id: &str,
    ) -> AppResult<Option<ClientRegistration>> {
        let row: Option<(String, String, Vec<String>)> = sqlx::query_as(
            "SELECT client_id, client_name, redirect_uris FROM oauth_client_registration WHERE client_id = $1",
        )
        .bind(client_id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(|(client_id, client_name, redirect_uris)| ClientRegistration {
            client_id,
            client_name,
            redirect_uris,
        }))
    }

    pub(crate) async fn oauth_ensure_application_impl(
        &mut self,
        client: &ClientRegistration,
        creator: UserId,
        now: DateTime<Utc>,
    ) -> AppResult<Uuid> {
        let existing: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM oauth_application WHERE organization_id = $1 AND dcr_client_id = $2 AND archived_at IS NULL",
        )
        .bind(self.org.0)
        .bind(&client.client_id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if let Some(id) = existing {
            return Ok(id);
        }
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO oauth_application (organization_id, name, client_secret_hash, redirect_uris, distribution, \
             grant_types, developer, developer_url, creator_id, dcr_client_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, 'public', ARRAY['authorization_code'], $2, 'urn:lineal:dcr', $5, $6, $7, $7) \
             RETURNING id",
        )
        .bind(self.org.0)
        .bind(&client.client_name)
        .bind(unusable_secret_hash())
        .bind(&client.redirect_uris)
        .bind(creator.0)
        .bind(&client.client_id)
        .bind(now)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(id)
    }

    pub(crate) async fn oauth_insert_code_impl(&mut self, c: &NewAuthCode) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO oauth_authorization_code (code_hash, organization_id, application_id, user_id, redirect_uri, \
             scopes, code_challenge, code_challenge_method, actor, family_id, expires_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'S256', $8, $9, $10, $11)",
        )
        .bind(&c.code_hash)
        .bind(self.org.0)
        .bind(c.application_id)
        .bind(c.user_id.0)
        .bind(&c.redirect_uri)
        .bind(&c.scopes)
        .bind(&c.code_challenge)
        .bind(&c.actor)
        .bind(c.family_id)
        .bind(c.expires_at)
        .bind(c.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    pub(crate) async fn oauth_find_code_impl(
        &mut self,
        code_hash: &str,
    ) -> AppResult<Option<AuthCodeRecord>> {
        type Row = (
            Uuid,
            Uuid,
            Option<String>,
            Uuid,
            String,
            Vec<String>,
            String,
            String,
            Uuid,
            DateTime<Utc>,
            Option<DateTime<Utc>>,
        );
        let row: Option<Row> = sqlx::query_as(
            "SELECT c.organization_id, c.application_id, a.dcr_client_id, c.user_id, c.redirect_uri, c.scopes, \
             c.code_challenge, c.actor, c.family_id, c.expires_at, c.used_at \
             FROM oauth_authorization_code c JOIN oauth_application a \
               ON a.organization_id = c.organization_id AND a.id = c.application_id \
             WHERE c.code_hash = $1",
        )
        .bind(code_hash)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(
            |(org, app, client, user, redirect_uri, scopes, challenge, actor, family, expires_at, used_at)| {
                AuthCodeRecord {
                    org_id: OrgId(org),
                    application_id: app,
                    client_id: client.unwrap_or_default(),
                    user_id: UserId(user),
                    redirect_uri,
                    scopes,
                    code_challenge: challenge,
                    actor,
                    family_id: family,
                    expires_at,
                    used_at,
                }
            },
        ))
    }

    pub(crate) async fn oauth_consume_code_impl(
        &mut self,
        code_hash: &str,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let res = sqlx::query(
            "UPDATE oauth_authorization_code SET used_at = $2 WHERE code_hash = $1 AND used_at IS NULL",
        )
        .bind(code_hash)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() == 1)
    }

    pub(crate) async fn oauth_insert_token_impl(&mut self, t: &NewOauthToken) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO oauth_token (id, organization_id, application_id, user_id, actor, scopes, access_token_hash, \
             access_expires_at, refresh_token_hash, family_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11)",
        )
        .bind(t.id)
        .bind(self.org.0)
        .bind(t.application_id)
        .bind(t.user_id.0)
        .bind(&t.actor)
        .bind(&t.scopes)
        .bind(&t.access_token_hash)
        .bind(t.access_expires_at)
        .bind(&t.refresh_token_hash)
        .bind(t.family_id)
        .bind(t.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    pub(crate) async fn oauth_revoke_family_impl(
        &mut self,
        family: Uuid,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let res = sqlx::query(
            "UPDATE oauth_token SET revoked_at = $3, version = version + 1, updated_at = $3 \
             WHERE organization_id = $1 AND family_id = $2 AND revoked_at IS NULL",
        )
        .bind(self.org.0)
        .bind(family)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected())
    }

    pub(crate) async fn oauth_find_by_refresh_hash_impl(
        &mut self,
        hash: &str,
    ) -> AppResult<Option<OauthTokenRecord>> {
        type Row = (
            Uuid,
            Uuid,
            Uuid,
            Option<String>,
            Uuid,
            String,
            Vec<String>,
            Option<String>,
            Option<String>,
            Option<DateTime<Utc>>,
            Uuid,
            Option<DateTime<Utc>>,
        );
        let row: Option<Row> = sqlx::query_as(
            "SELECT t.id, t.organization_id, t.application_id, a.dcr_client_id, t.user_id, t.actor, t.scopes, \
             t.refresh_token_hash, t.previous_refresh_token_hash, t.rotated_at, t.family_id, t.revoked_at \
             FROM oauth_token t JOIN oauth_application a \
               ON a.organization_id = t.organization_id AND a.id = t.application_id \
             WHERE t.refresh_token_hash = $1 OR t.previous_refresh_token_hash = $1 \
             ORDER BY (t.refresh_token_hash = $1) DESC LIMIT 1",
        )
        .bind(hash)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(
            |(id, org, app, client, user, actor, scopes, refresh, prev, rotated_at, family, revoked_at)| {
                OauthTokenRecord {
                    id,
                    org_id: OrgId(org),
                    application_id: app,
                    client_id: client.unwrap_or_default(),
                    user_id: UserId(user),
                    actor,
                    scopes,
                    refresh_token_hash: refresh,
                    previous_refresh_token_hash: prev,
                    rotated_at,
                    family_id: family,
                    revoked_at,
                }
            },
        ))
    }

    pub(crate) async fn oauth_rotate_token_impl(
        &mut self,
        id: Uuid,
        expected_current: &str,
        r: &TokenRotation,
    ) -> AppResult<bool> {
        let res = sqlx::query(
            "UPDATE oauth_token SET access_token_hash = $4, access_expires_at = $5, \
             previous_refresh_token_hash = refresh_token_hash, refresh_token_hash = $6, rotated_at = $7, \
             version = version + 1, updated_at = $7 \
             WHERE organization_id = $1 AND id = $2 AND refresh_token_hash = $3 AND revoked_at IS NULL",
        )
        .bind(self.org.0)
        .bind(id)
        .bind(expected_current)
        .bind(&r.access_token_hash)
        .bind(r.access_expires_at)
        .bind(&r.refresh_token_hash)
        .bind(r.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() == 1)
    }

    pub(crate) async fn oauth_revoke_by_token_hash_impl(
        &mut self,
        hash: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<(OrgId, String)>> {
        let row: Option<(Uuid, Uuid, Option<String>)> = sqlx::query_as(
            "SELECT t.organization_id, t.family_id, a.dcr_client_id FROM oauth_token t \
             JOIN oauth_application a ON a.organization_id = t.organization_id AND a.id = t.application_id \
             WHERE t.revoked_at IS NULL AND (t.access_token_hash = $1 OR t.refresh_token_hash = $1) LIMIT 1",
        )
        .bind(hash)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let Some((org, family, client)) = row else {
            return Ok(None);
        };
        sqlx::query(
            "UPDATE oauth_token SET revoked_at = $3, version = version + 1, updated_at = $3 \
             WHERE organization_id = $1 AND family_id = $2 AND revoked_at IS NULL",
        )
        .bind(org)
        .bind(family)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(Some((OrgId(org), client.unwrap_or_default())))
    }
}
