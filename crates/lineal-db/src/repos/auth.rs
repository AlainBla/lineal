//! `AuthRepo` für `PgTx`: Token-Lookup (Session, API-Key, OAuth), Sessions, API-Keys.

use super::user::parse_role;
use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::auth::{
    ApiKeyRecord, LoginCandidate, NewApiKey, NewSession, OAuthGrant, SessionRecord,
};
use lineal_app::ports::repos::{AuthRepo, CredentialKind, Principal};
use lineal_app::{Actor, AppResult, Scopes};
use lineal_domain::enums::UserRole;
use lineal_domain::ids::{ApiKeyId, OrgId, TeamId, UserId};
use uuid::Uuid;

fn actor_for(user: UserId, role: UserRole) -> Actor {
    if role == UserRole::App {
        Actor::App { id: user }
    } else {
        Actor::User { id: user, role }
    }
}

/// Scopes aus der DB, begrenzt durch die aktuelle Rolle: Gäste und Apps nie `admin`.
fn scopes_from(list: &[String], role: UserRole) -> Scopes {
    Scopes {
        read: list.iter().any(|s| s == "read"),
        write: list.iter().any(|s| s == "write"),
        admin: list.iter().any(|s| s == "admin")
            && !matches!(role, UserRole::Guest | UserRole::App),
    }
}

#[derive(sqlx::FromRow)]
struct KeyRow {
    id: Uuid,
    user_id: Uuid,
    label: String,
    key_prefix: String,
    scopes: Vec<String>,
    team_ids: Option<Vec<Uuid>>,
    expires_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

impl From<KeyRow> for ApiKeyRecord {
    fn from(r: KeyRow) -> Self {
        ApiKeyRecord {
            id: ApiKeyId(r.id),
            user_id: UserId(r.user_id),
            label: r.label,
            key_prefix: r.key_prefix,
            scopes: r.scopes,
            team_ids: r.team_ids.map(|v| v.into_iter().map(TeamId).collect()),
            expires_at: r.expires_at,
            last_used_at: r.last_used_at,
            revoked_at: r.revoked_at,
            created_at: r.created_at,
        }
    }
}

const KEY_COLS: &str = "id, user_id, label, key_prefix, scopes, team_ids, expires_at, last_used_at, revoked_at, created_at";

#[async_trait]
impl AuthRepo for PgTx {
    async fn find_calendar_feed_principal(&mut self, hash: &str) -> AppResult<Option<Principal>> {
        let row: Option<(Uuid, Uuid, String)> = sqlx::query_as(
            "SELECT f.organization_id, u.id, u.role FROM calendar_feed f \
             JOIN \"user\" u ON u.organization_id = f.organization_id AND u.id = f.user_id \
             JOIN organization o ON o.id = f.organization_id AND o.archived_at IS NULL \
             WHERE f.token_hash = $1 AND u.active AND u.status = 'active' AND u.archived_at IS NULL",
        )
        .bind(hash)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let Some((org, user, role)) = row else {
            return Ok(None);
        };
        let role = parse_role(&role)?;
        Ok(Some(Principal {
            kind: CredentialKind::ApiKey,
            org_id: OrgId(org),
            actor: actor_for(UserId(user), role),
            scopes: Scopes {
                read: true,
                write: false,
                admin: false,
            },
            team_scope: None,
        }))
    }

    async fn set_calendar_feed(
        &mut self,
        user: UserId,
        hash: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO calendar_feed (organization_id, user_id, token_hash, created_at) \
             VALUES ($1, $2, $3, $4) ON CONFLICT (organization_id, user_id) \
             DO UPDATE SET token_hash = excluded.token_hash, created_at = excluded.created_at",
        )
        .bind(self.org.0)
        .bind(user.0)
        .bind(hash)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn delete_calendar_feed(&mut self, user: UserId) -> AppResult<()> {
        sqlx::query("DELETE FROM calendar_feed WHERE organization_id = $1 AND user_id = $2")
            .bind(self.org.0)
            .bind(user.0)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn has_calendar_feed(&mut self, user: UserId) -> AppResult<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM calendar_feed \
             WHERE organization_id = $1 AND user_id = $2)",
        )
        .bind(self.org.0)
        .bind(user.0)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn find_principal_by_token_hash(
        &mut self,
        hash: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Option<Principal>> {
        // 1. Web-Session
        let row: Option<(Uuid, Uuid, String)> = sqlx::query_as(
            "SELECT s.organization_id, u.id, u.role FROM user_session s \
             JOIN \"user\" u ON u.organization_id = s.organization_id AND u.id = s.user_id \
             JOIN organization o ON o.id = s.organization_id AND o.archived_at IS NULL \
             WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.archived_at IS NULL \
               AND s.expires_at > $2 AND u.active AND u.status = 'active' AND u.archived_at IS NULL",
        )
        .bind(hash)
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if let Some((org, user, role)) = row {
            let role = parse_role(&role)?;
            let mut scopes = Scopes::all();
            if matches!(role, UserRole::Guest | UserRole::App) {
                scopes.admin = false;
            }
            return Ok(Some(Principal {
                kind: CredentialKind::Session,
                org_id: OrgId(org),
                actor: actor_for(UserId(user), role),
                scopes,
                team_scope: None,
            }));
        }

        // 2. Persönlicher API-Key
        let row: Option<(Uuid, Uuid, String, Vec<String>, Option<Vec<Uuid>>)> = sqlx::query_as(
            "SELECT k.organization_id, u.id, u.role, k.scopes, k.team_ids FROM api_key k \
             JOIN \"user\" u ON u.organization_id = k.organization_id AND u.id = k.user_id \
             JOIN organization o ON o.id = k.organization_id AND o.archived_at IS NULL \
             WHERE k.key_hash = $1 AND k.revoked_at IS NULL AND k.archived_at IS NULL \
               AND (k.expires_at IS NULL OR k.expires_at > $2) AND u.active AND u.status = 'active' AND u.archived_at IS NULL",
        )
        .bind(hash)
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if let Some((org, user, role, scopes, team_ids)) = row {
            let role = parse_role(&role)?;
            return Ok(Some(Principal {
                kind: CredentialKind::ApiKey,
                org_id: OrgId(org),
                actor: actor_for(UserId(user), role),
                scopes: scopes_from(&scopes, role),
                // `NULL` = alle sichtbaren Teams; eine Liste beschränkt den Key auf diese Teams.
                team_scope: team_ids.map(|ids| ids.into_iter().map(TeamId).collect()),
            }));
        }

        // 3. OAuth-Access-Token
        let row: Option<(Uuid, Uuid, String, String, Vec<String>)> = sqlx::query_as(
            "SELECT t.organization_id, u.id, u.role, t.actor, t.scopes FROM oauth_token t \
             JOIN \"user\" u ON u.organization_id = t.organization_id AND u.id = t.user_id \
             JOIN organization o ON o.id = t.organization_id AND o.archived_at IS NULL \
             WHERE t.access_token_hash = $1 AND t.revoked_at IS NULL AND t.archived_at IS NULL \
               AND t.access_expires_at > $2 AND u.active AND u.status = 'active' AND u.archived_at IS NULL",
        )
        .bind(hash)
        .bind(now)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if let Some((org, user, role, actor, scopes)) = row {
            let role = parse_role(&role)?;
            let actor = if actor == "app" {
                Actor::App { id: UserId(user) }
            } else {
                actor_for(UserId(user), role)
            };
            return Ok(Some(Principal {
                kind: CredentialKind::OAuth,
                org_id: OrgId(org),
                actor,
                scopes: scopes_from(&scopes, role),
                team_scope: None,
            }));
        }
        Ok(None)
    }

    async fn find_login_candidates(
        &mut self,
        email: &str,
        url_key: Option<&str>,
    ) -> AppResult<Vec<LoginCandidate>> {
        let rows: Vec<(Uuid, String, Uuid, String, bool, Option<String>)> = sqlx::query_as(
            "SELECT u.organization_id, o.url_key, u.id, u.role, u.active, u.password_hash \
             FROM \"user\" u JOIN organization o ON o.id = u.organization_id \
             WHERE lower(u.email) = lower($1) AND u.status = 'active' AND u.archived_at IS NULL AND o.archived_at IS NULL \
               AND ($2::text IS NULL OR o.url_key = $2) \
             ORDER BY o.created_at, u.id",
        )
        .bind(email)
        .bind(url_key)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter()
            .map(|(org, url_key, user, role, active, password_hash)| {
                Ok(LoginCandidate {
                    org_id: OrgId(org),
                    url_key,
                    user_id: UserId(user),
                    role: parse_role(&role)?,
                    active,
                    password_hash,
                })
            })
            .collect()
    }

    async fn insert_session(&mut self, s: &NewSession) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO user_session (id, organization_id, user_id, token_hash, csrf_token_hash, user_agent, \
             ip_address, expires_at, last_used_at, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9, $9)",
        )
        .bind(s.id)
        .bind(self.org.0)
        .bind(s.user_id.0)
        .bind(&s.token_hash)
        .bind(&s.csrf_token_hash)
        .bind(&s.user_agent)
        .bind(&s.ip_address)
        .bind(s.expires_at)
        .bind(s.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn revoke_session(&mut self, token_hash: &str, now: DateTime<Utc>) -> AppResult<bool> {
        // Mandantenübergreifend über den Token-Hash (der Hash ist global eindeutig).
        let res = sqlx::query(
            "UPDATE user_session SET revoked_at = $2, version = version + 1, updated_at = $2 \
             WHERE token_hash = $1 AND revoked_at IS NULL",
        )
        .bind(token_hash)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn list_sessions(
        &mut self,
        user_id: UserId,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<SessionRecord>> {
        #[allow(clippy::type_complexity)]
        let rows: Vec<(
            Uuid,
            String,
            Option<String>,
            Option<String>,
            DateTime<Utc>,
            Option<DateTime<Utc>>,
            DateTime<Utc>,
        )> = sqlx::query_as(
            "SELECT id, token_hash, user_agent, ip_address, created_at, last_used_at, expires_at \
             FROM user_session \
             WHERE organization_id = $1 AND user_id = $2 AND revoked_at IS NULL \
               AND archived_at IS NULL AND expires_at > $3 \
             ORDER BY created_at DESC, id DESC LIMIT 100",
        )
        .bind(self.org.0)
        .bind(user_id.0)
        .bind(now)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(
                |(id, token_hash, user_agent, ip_address, created_at, last_used_at, expires_at)| {
                    SessionRecord {
                        id,
                        token_hash,
                        user_agent,
                        ip_address,
                        created_at,
                        last_used_at,
                        expires_at,
                    }
                },
            )
            .collect())
    }

    async fn revoke_session_by_id(
        &mut self,
        user_id: UserId,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let res = sqlx::query(
            "UPDATE user_session SET revoked_at = $4, version = version + 1, updated_at = $4 \
             WHERE organization_id = $1 AND user_id = $2 AND id = $3 AND revoked_at IS NULL",
        )
        .bind(self.org.0)
        .bind(user_id.0)
        .bind(id)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn revoke_other_sessions(
        &mut self,
        user_id: UserId,
        keep_token_hash: &str,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let res = sqlx::query(
            "UPDATE user_session SET revoked_at = $4, version = version + 1, updated_at = $4 \
             WHERE organization_id = $1 AND user_id = $2 AND token_hash <> $3 AND revoked_at IS NULL",
        )
        .bind(self.org.0)
        .bind(user_id.0)
        .bind(keep_token_hash)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected())
    }

    async fn insert_api_key(&mut self, k: &NewApiKey) -> AppResult<()> {
        let team_ids: Option<Vec<Uuid>> =
            k.team_ids.as_ref().map(|v| v.iter().map(|t| t.0).collect());
        sqlx::query(
            "INSERT INTO api_key (id, organization_id, user_id, label, key_prefix, key_hash, scopes, team_ids, \
             expires_at, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10)",
        )
        .bind(k.id.0)
        .bind(self.org.0)
        .bind(k.user_id.0)
        .bind(&k.label)
        .bind(&k.key_prefix)
        .bind(&k.key_hash)
        .bind(&k.scopes)
        .bind(team_ids)
        .bind(k.expires_at)
        .bind(k.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_api_key(&mut self, id: ApiKeyId) -> AppResult<Option<ApiKeyRecord>> {
        let row = sqlx::query_as::<_, KeyRow>(&format!(
            "SELECT {KEY_COLS} FROM api_key WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(ApiKeyRecord::from))
    }

    async fn list_api_keys(&mut self, user_id: UserId) -> AppResult<Vec<ApiKeyRecord>> {
        let rows = sqlx::query_as::<_, KeyRow>(&format!(
            "SELECT {KEY_COLS} FROM api_key WHERE organization_id = $1 AND user_id = $2 AND archived_at IS NULL \
             ORDER BY created_at DESC, id DESC"
        ))
        .bind(self.org.0)
        .bind(user_id.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(ApiKeyRecord::from).collect())
    }

    async fn revoke_api_key(&mut self, id: ApiKeyId, now: DateTime<Utc>) -> AppResult<bool> {
        let res = sqlx::query(
            "UPDATE api_key SET revoked_at = $3, version = version + 1, updated_at = $3 \
             WHERE organization_id = $1 AND id = $2 AND revoked_at IS NULL",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(res.rows_affected() > 0)
    }

    async fn revoke_all_credentials_of_user(
        &mut self,
        user_id: UserId,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        for table in ["user_session", "api_key", "oauth_token"] {
            sqlx::query(&format!(
                "UPDATE {table} SET revoked_at = $3, version = version + 1, updated_at = $3 \
                 WHERE organization_id = $1 AND user_id = $2 AND revoked_at IS NULL"
            ))
            .bind(self.org.0)
            .bind(user_id.0)
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        }
        // Noch nicht eingelöste OAuth-Codes (Refresh-Tokens liegen in `oauth_token`, s. o.).
        sqlx::query(
            "UPDATE oauth_authorization_code SET used_at = $3 \
             WHERE organization_id = $1 AND user_id = $2 AND used_at IS NULL",
        )
        .bind(self.org.0)
        .bind(user_id.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        // Offene Einladungslinks des gesperrten Users.
        sqlx::query(
            "UPDATE invitation SET status = 'revoked', version = version + 1, updated_at = $3 \
             WHERE organization_id = $1 AND inviter_id = $2 AND status = 'pending'",
        )
        .bind(self.org.0)
        .bind(user_id.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn revoke_oauth_tokens_of_user(
        &mut self,
        user_id: UserId,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let n = sqlx::query(
            "UPDATE oauth_token SET revoked_at = $3, version = version + 1, updated_at = $3 \
             WHERE organization_id = $1 AND user_id = $2 AND revoked_at IS NULL",
        )
        .bind(self.org.0)
        .bind(user_id.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?
        .rows_affected();
        sqlx::query(
            "UPDATE oauth_authorization_code SET used_at = $3 \
             WHERE organization_id = $1 AND user_id = $2 AND used_at IS NULL",
        )
        .bind(self.org.0)
        .bind(user_id.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(n)
    }

    async fn revoke_api_keys_of_user(
        &mut self,
        user_id: UserId,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let n = sqlx::query(
            "UPDATE api_key SET revoked_at = $3, version = version + 1, updated_at = $3 \
             WHERE organization_id = $1 AND user_id = $2 AND revoked_at IS NULL",
        )
        .bind(self.org.0)
        .bind(user_id.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?
        .rows_affected();
        Ok(n)
    }

    async fn list_oauth_grants(&mut self, user_id: UserId) -> AppResult<Vec<OAuthGrant>> {
        // Ein aktives Token je Zeile; die Gruppierung je Anwendung (Scopes vereinen, früheste
        // Freigabe, jüngste Aktivität) in Rust – klarer als Array-Aggregate in SQL.
        let rows: Vec<(Uuid, String, Vec<String>, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
            "SELECT t.application_id, a.name, t.scopes, t.created_at, t.updated_at \
             FROM oauth_token t JOIN oauth_application a \
               ON a.organization_id = t.organization_id AND a.id = t.application_id \
             WHERE t.organization_id = $1 AND t.user_id = $2 AND t.revoked_at IS NULL",
        )
        .bind(self.org.0)
        .bind(user_id.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        let mut by_app: std::collections::BTreeMap<Uuid, OAuthGrant> =
            std::collections::BTreeMap::new();
        for (app_id, name, scopes, created, updated) in rows {
            let e = by_app.entry(app_id).or_insert_with(|| OAuthGrant {
                application_id: app_id,
                application_name: name,
                scopes: Vec::new(),
                granted_at: created,
                last_active_at: updated,
            });
            for s in scopes {
                if !e.scopes.contains(&s) {
                    e.scopes.push(s);
                }
            }
            e.granted_at = e.granted_at.min(created);
            e.last_active_at = e.last_active_at.max(updated);
        }
        let mut grants: Vec<OAuthGrant> = by_app.into_values().collect();
        for g in &mut grants {
            g.scopes.sort();
        }
        grants.sort_by(|a, b| b.granted_at.cmp(&a.granted_at));
        Ok(grants)
    }

    async fn revoke_oauth_grant(
        &mut self,
        user_id: UserId,
        application_id: Uuid,
        now: DateTime<Utc>,
    ) -> AppResult<u64> {
        let n = sqlx::query(
            "UPDATE oauth_token SET revoked_at = $4, version = version + 1, updated_at = $4 \
             WHERE organization_id = $1 AND user_id = $2 AND application_id = $3 AND revoked_at IS NULL",
        )
        .bind(self.org.0)
        .bind(user_id.0)
        .bind(application_id)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?
        .rows_affected();
        // Auch noch nicht eingelöste Authorization-Codes dieser App entwerten – sonst könnte ein
        // offener Code nach dem Widerruf noch ein neues Token lösen. In die Rückgabe zählen, damit
        // eine Freigabe mit nur einem offenen Code (ohne aktives Token) nicht als not_found gilt.
        let codes = sqlx::query(
            "UPDATE oauth_authorization_code SET used_at = $4 \
             WHERE organization_id = $1 AND user_id = $2 AND application_id = $3 AND used_at IS NULL",
        )
        .bind(self.org.0)
        .bind(user_id.0)
        .bind(application_id)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?
        .rows_affected();
        Ok(n + codes)
    }

    async fn ip_recently_succeeded(
        &mut self,
        email: &str,
        ip: &str,
        workspace: Option<&str>,
        since: DateTime<Utc>,
    ) -> AppResult<bool> {
        // Eine Abfrage, die „kein User" und „User ohne Session" gleich behandelt (kein
        // Timing-Orakel). Gegen Cross-Tenant-Trust:
        //  - mit `workspace` (url_key) exakt auf diesen Mandanten eingeschränkt;
        //  - ohne `workspace` nur, wenn die E-Mail in GENAU EINEM Workspace existiert (sonst wäre
        //    unklar, welches Konto gemeint ist, und ein Login in Workspace A würde die IP für einen
        //    Angriff auf Workspace B vertrauen).
        let found: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM \"user\" u \
               JOIN organization o ON o.id = u.organization_id \
               JOIN user_session s ON s.organization_id = u.organization_id AND s.user_id = u.id \
             WHERE lower(u.email) = lower($1) AND s.ip_address = $2 AND s.created_at >= $3 \
               AND ($4::text IS NULL OR o.url_key = $4)) \
             AND ($4::text IS NOT NULL OR \
                  (SELECT count(DISTINCT organization_id) FROM \"user\" WHERE lower(email) = lower($1)) = 1)",
        )
        .bind(email.trim())
        .bind(ip)
        .bind(since)
        .bind(workspace)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(found)
    }
}
