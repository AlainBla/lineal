//! usecases – Authentifizierung (docs/09 §2): Passwort-Login mit Session, Logout, API-Keys.
//!
//! Token-Format: Sessions sind 96 Hex-Zeichen (3 × UUIDv4, > 256 Bit Zufall), API-Keys
//! `lin_api_` + 96 Hex-Zeichen. Gespeichert wird nur `hash_token` = SHA-256 (hex, Kleinbuchstaben)
//! über das **komplette** Token inklusive Präfix; Adapter hashen genauso vor dem Lookup.

use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::repos::Principal;
use crate::ports::repos::auth::{ApiKeyRecord, NewApiKey, NewSession};
use crate::resolve;
use crate::resolve::team::Visibility;
use chrono::{DateTime, Duration, Utc};
use lineal_domain::enums::UserRole;
use lineal_domain::ids::{ApiKeyId, OrgId, UserId};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Gültigkeit einer Web-Session.
pub const SESSION_TTL_DAYS: i64 = 14;
/// Präfix persönlicher API-Keys.
pub const API_KEY_PREFIX: &str = "lin_api_";

/// SHA-256 (hex, Kleinbuchstaben) über das komplette Token.
pub fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

/// 3 × UUIDv4 als Hex (366 Bit Zufall aus dem OS-RNG; nicht `IdGen`, das zeitbasiert ist).
fn random_hex() -> String {
    format!(
        "{}{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

fn invalid_credentials() -> AppError {
    AppError::unauthenticated("Invalid email or password.")
}

/// Eingabe für `login`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoginInput {
    pub email: String,
    pub password: String,
    /// `urlKey` des Workspace; nötig, wenn die E-Mail in mehreren Workspaces existiert.
    pub workspace: Option<String>,
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
}

/// Ergebnis von `login`.
#[derive(Debug, Clone, PartialEq)]
pub struct Login {
    /// Klartext-Token für das Session-Cookie (wird nie gespeichert).
    pub token: String,
    pub csrf_token: String,
    pub expires_at: DateTime<Utc>,
    pub org_id: OrgId,
    pub user_id: UserId,
}

/// Eingabe für `create_api_key`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CreateApiKeyInput {
    pub label: String,
    /// Teilmenge von `read`, `write`, `admin` (`read` ist immer enthalten).
    pub scopes: Vec<String>,
    /// Einschränkung auf Teams (Referenzen); `None` = alle Teams des Users.
    pub teams: Option<Vec<String>>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Neu angelegter Key; `key` ist der Klartext, der nur hier erscheint.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatedApiKey {
    pub record: ApiKeyRecord,
    pub key: String,
}

impl App {
    /// Eine Passwortprüfung gegen einen frisch erzeugten Hash (gleicher Aufwand wie eine echte).
    fn burn_verify(&self, password: &str) {
        if let Ok(h) = self.hasher.hash("lineal-dummy-password") {
            let _ = self.hasher.verify(password, &h);
        }
    }

    /// Passwort-Login (docs/11 `POST /auth/login`). Unbekannte E-Mail, falsches Passwort,
    /// Platzhalter-User ohne Passwort und suspendierte User liefern dieselbe Antwort; in jedem
    /// Fall läuft genau eine Passwortprüfung pro Kandidat (bzw. eine Dummy-Prüfung ohne Kandidat).
    pub async fn login(&self, input: LoginInput) -> AppResult<Login> {
        let mut tx = self.store.begin_system().await?;
        let candidates = tx
            .auth()
            .find_login_candidates(input.email.trim(), input.workspace.as_deref())
            .await?;
        drop(tx);
        let mut matched = Vec::new();
        if candidates.is_empty() {
            // Zeitverhalten angleichen; das Ergebnis wird verworfen.
            self.burn_verify(&input.password);
        }
        for c in &candidates {
            let ok = match &c.password_hash {
                Some(h) => self.hasher.verify(&input.password, h).unwrap_or(false),
                None => {
                    self.burn_verify(&input.password);
                    false
                }
            };
            if ok && c.active {
                matched.push(c);
            }
        }
        let chosen = match matched.as_slice() {
            [] => return Err(invalid_credentials()),
            [one] => *one,
            _ => {
                return Err(AppError::invalid_request(
                    "This email exists in several workspaces; pass `workspace`.",
                )
                .with_details(json!({ "field": "workspace" })));
            }
        };
        let now = self.clock.now();
        let token = random_hex();
        let csrf = random_hex();
        let expires_at = now + Duration::days(SESSION_TTL_DAYS);
        let mut tx = self.store.begin(chosen.org_id).await?;
        tx.auth()
            .insert_session(&NewSession {
                id: self.ids.new_uuid(),
                user_id: chosen.user_id,
                token_hash: hash_token(&token),
                csrf_token_hash: Some(hash_token(&csrf)),
                user_agent: input.user_agent.clone(),
                ip_address: input.ip_address.clone(),
                expires_at,
                now,
            })
            .await?;
        tx.commit().await?;
        Ok(Login {
            token,
            csrf_token: csrf,
            expires_at,
            org_id: chosen.org_id,
            user_id: chosen.user_id,
        })
    }

    /// Beendet die Session serverseitig; `true`, wenn eine aktive Session widerrufen wurde.
    pub async fn logout(&self, token: &str) -> AppResult<bool> {
        let mut tx = self.store.begin_system().await?;
        let revoked = tx
            .auth()
            .revoke_session(&hash_token(token), self.clock.now())
            .await?;
        tx.commit().await?;
        Ok(revoked)
    }

    /// Löst ein Klartext-Token (Session, API-Key, OAuth) zum Aufrufer auf.
    pub async fn principal_for_token(&self, token: &str) -> AppResult<Principal> {
        let mut tx = self.store.begin_system().await?;
        tx.auth()
            .find_principal_by_token_hash(&hash_token(token))
            .await?
            .ok_or_else(|| AppError::unauthenticated("Invalid or expired token."))
    }

    /// Legt einen persönlichen API-Key an (docs/09 §2.2). Die Scopes dürfen die des
    /// aufrufenden Tokens und die Rolle des Users nicht übersteigen.
    pub async fn create_api_key(
        &self,
        ctx: &RequestCtx,
        input: CreateApiKeyInput,
    ) -> AppResult<CreatedApiKey> {
        let user_id = resolve::user::actor_user_id(ctx)
            .filter(|_| matches!(ctx.actor, crate::context::Actor::User { .. }))
            .ok_or_else(|| AppError::forbidden("Only users can create personal API keys."))?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::CreatePersonalApiKey)?;
        if !ctx.scopes.write {
            return Err(AppError::forbidden(
                "Token scope does not allow write operations.",
            ));
        }
        let label = input.label.trim().to_string();
        if label.is_empty() || label.chars().count() > 100 {
            return Err(AppError::invalid_request("label must be 1-100 characters")
                .with_details(json!({ "field": "label" })));
        }
        let mut scopes = vec!["read".to_string()];
        for s in &input.scopes {
            match s.as_str() {
                "read" => {}
                "write" if ctx.scopes.write => scopes.push("write".into()),
                "admin" if ctx.scopes.admin && snap.actor.is_admin() => scopes.push("admin".into()),
                "write" | "admin" => {
                    return Err(AppError::forbidden(
                        "The key cannot have more scopes than your own token",
                    )
                    .with_details(json!({ "scope": s })));
                }
                other => {
                    return Err(
                        AppError::invalid_request(format!("unknown scope `{other}`"))
                            .with_details(json!({ "field": "scopes" })),
                    );
                }
            }
        }
        scopes.dedup();
        let team_ids = match &input.teams {
            Some(refs) => {
                let mut ids = Vec::new();
                for r in refs {
                    ids.push(
                        resolve::team::resolve_in(&mut *tx, &snap, r, Visibility::Content)
                            .await?
                            .id,
                    );
                }
                Some(ids)
            }
            None => None,
        };
        let now = self.clock.now();
        if input.expires_at.is_some_and(|e| e <= now) {
            return Err(AppError::invalid_request("expiresAt must be in the future")
                .with_details(json!({ "field": "expiresAt" })));
        }
        let key = format!("{API_KEY_PREFIX}{}", random_hex());
        let id = ApiKeyId(self.ids.new_uuid());
        tx.auth()
            .insert_api_key(&NewApiKey {
                id,
                user_id,
                label,
                key_prefix: key.chars().take(12).collect(),
                key_hash: hash_token(&key),
                scopes,
                team_ids,
                expires_at: input.expires_at,
                now,
            })
            .await?;
        let record = tx
            .auth()
            .get_api_key(id)
            .await?
            .ok_or_else(|| AppError::internal("api key vanished after insert"))?;
        tx.commit().await?;
        Ok(CreatedApiKey { record, key })
    }

    /// Eigene API-Keys (neueste zuerst, inkl. widerrufener).
    pub async fn list_api_keys(&self, ctx: &RequestCtx) -> AppResult<Vec<ApiKeyRecord>> {
        let user_id = resolve::user::actor_user_id(ctx)
            .ok_or_else(|| AppError::forbidden("Only users own API keys."))?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        tx.auth().list_api_keys(user_id).await
    }

    /// Widerruft einen Key: der Eigentümer oder ein Admin/Owner. Wirkt sofort.
    pub async fn revoke_api_key(&self, ctx: &RequestCtx, id: ApiKeyId) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let key = tx.auth().get_api_key(id).await?.ok_or_else(|| {
            AppError::not_found("Could not find referenced ApiKey.")
                .with_details(json!({ "type": "ApiKey" }))
        })?;
        let own = resolve::user::actor_user_id(ctx) == Some(key.user_id);
        let admin = matches!(ctx.actor, crate::context::Actor::System)
            || matches!(ctx.actor, crate::context::Actor::User { role, .. } if matches!(role, UserRole::Owner | UserRole::Admin));
        if !own && !admin {
            return Err(AppError::not_found("Could not find referenced ApiKey.")
                .with_details(json!({ "type": "ApiKey" })));
        }
        if !ctx.scopes.write {
            return Err(AppError::forbidden(
                "Token scope does not allow write operations.",
            ));
        }
        tx.auth().revoke_api_key(id, self.clock.now()).await?;
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_sha256_hex_lowercase_over_the_full_token() {
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn tokens_have_enough_entropy_and_are_unique() {
        let a = random_hex();
        assert_eq!(a.len(), 96);
        assert_ne!(a, random_hex());
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
