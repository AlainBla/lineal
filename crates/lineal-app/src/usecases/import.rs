//! usecases – Import aus Linear: Verwaltung der Läufe (docs/13 §4, §6, §8).
//!
//! Alle Einstiege laufen durch `policy::require_workspace(.., Action::ManageImport)` (nur
//! Workspace-Admins/Owner, Scope `admin`). Der eigentliche Lauf (Phasen, Quelle, Schreiben) steht
//! im Crate `lineal-import`; hier liegen Lauf-Steuerung und die **Token-Verschlüsselung**, damit
//! GraphQL und CLI denselben abgesicherten Pfad benutzen.
//!
//! Das Token verlässt diese Datei nur als [`Secret`], dessen `Debug`/`Display` nichts preisgeben.

use crate::app::App;
use crate::context::{Actor, RequestCtx};
use crate::error::{AppError, AppResult};
use crate::policy::{Action, require_workspace};
use crate::ports::repos::import::{ImportRun, ImportRunUpdate, NewImportRun};
use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use chrono::{DateTime, Duration, Utc};
use hkdf::Hkdf;
use serde_json::{Value, json};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::fmt;
use uuid::Uuid;

/// Gültigkeit der gespeicherten Zugangsdaten (docs/13 §8: Löschung nach spätestens 7 Tagen).
pub const CREDENTIAL_TTL_DAYS: i64 = 7;
/// Quelle `linear` (Wert in `import_run.source`).
pub const SOURCE_LINEAR: &str = "linear";
/// Quelle `linearCsv`.
pub const SOURCE_LINEAR_CSV: &str = "linearCsv";
const HKDF_INFO: &[u8] = b"lineal-import-credential-v1";
const NONCE_LEN: usize = 12;
const MAX_TOKEN_LEN: usize = 512;

/// Ein Geheimnis (API-Token). `Debug`/`Display` zeigen nie den Inhalt.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Prüft Form und Länge (nur druckbares ASCII ohne Leerzeichen, 8–512 Zeichen).
    pub fn new(raw: &str) -> AppResult<Self> {
        let t = raw.trim();
        let ok = (8..=MAX_TOKEN_LEN).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_graphic());
        if !ok {
            return Err(AppError::invalid_request("the token has an invalid format")
                .with_details(json!({ "field": "token" })));
        }
        Ok(Self(t.to_owned()))
    }

    /// Klartext – nur zum Aufbau des Authorization-Headers verwenden.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

fn cipher(session_secret: &str) -> AppResult<Aes256Gcm> {
    let hk = Hkdf::<Sha256>::new(None, session_secret.as_bytes());
    let mut key = [0u8; 32];
    hk.expand(HKDF_INFO, &mut key)
        .map_err(|_| AppError::internal("key derivation failed"))?;
    Aes256Gcm::new_from_slice(&key).map_err(|_| AppError::internal("key setup failed"))
}

/// AES-256-GCM; Schlüssel per HKDF-SHA256 aus dem Session-Secret, Nonce vorangestellt,
/// Lauf-ID als zusätzlich authentifizierte Daten (Ciphertext ist an den Lauf gebunden).
pub fn encrypt_token(session_secret: &str, run_id: Uuid, token: &Secret) -> AppResult<Vec<u8>> {
    let cipher = cipher(session_secret)?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ct = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: token.expose().as_bytes(),
                aad: run_id.as_bytes(),
            },
        )
        .map_err(|_| AppError::internal("encryption failed"))?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Gegenstück zu [`encrypt_token`].
pub fn decrypt_token(session_secret: &str, run_id: Uuid, blob: &[u8]) -> AppResult<Secret> {
    if blob.len() <= NONCE_LEN {
        return Err(AppError::internal("stored credential is corrupt"));
    }
    let (nonce, ct) = blob.split_at(NONCE_LEN);
    let plain = cipher(session_secret)?
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ct,
                aad: run_id.as_bytes(),
            },
        )
        .map_err(|_| AppError::internal("stored credential cannot be decrypted"))?;
    let text =
        String::from_utf8(plain).map_err(|_| AppError::internal("stored credential is corrupt"))?;
    Ok(Secret(text))
}

/// Eingabe `importScan`.
#[derive(Debug, Clone)]
pub struct ImportScanInput {
    /// `linear` (CSV läuft nur über die CLI).
    pub source: String,
    pub token: String,
}

/// Eingabe `importStart`.
#[derive(Debug, Clone, Default)]
pub struct ImportStartInput {
    pub run_id: Uuid,
    /// Linear-Team-Keys; leer = alle gescannten Teams.
    pub teams: Vec<String>,
    /// Linear-User-ID -> Lineal-User-ID (`None` = Platzhalter-User).
    pub user_mapping: BTreeMap<String, Option<Uuid>>,
    /// Linear-Team-Key -> Ziel-Key in Lineal.
    pub key_mapping: BTreeMap<String, String>,
    pub dry_run: bool,
    /// Nur Änderungen seit dem letzten erfolgreichen Lauf (docs/13 §4.2).
    pub delta: bool,
}

/// Aufbereitete Optionen (`import_run.options`), vom Importer gelesen.
pub fn options_json(
    input: &ImportStartInput,
    teams: &[String],
    updated_after: Option<DateTime<Utc>>,
) -> Value {
    json!({
        "teams": teams,
        "userMapping": input.user_mapping,
        "keyMapping": input.key_mapping,
        "dryRun": input.dry_run,
        "delta": input.delta,
        "updatedAfter": updated_after,
    })
}

fn invalid(field: &str, msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg).with_details(json!({ "field": field }))
}

fn actor_id(ctx: &RequestCtx) -> Option<Uuid> {
    match ctx.actor {
        Actor::User { id, .. } | Actor::App { id } => Some(id.0),
        Actor::System => None,
    }
}

fn not_found() -> AppError {
    AppError::not_found("Import run not found.")
}

impl App {
    /// `importScan`: legt einen Lauf im Status `scanning` an und speichert das Token verschlüsselt.
    /// Der Scan selbst (Verbindung prüfen, Mengen zählen) läuft im Import-Job
    /// (`lineal-import`), damit kein HTTP in Request-Handlern passiert.
    pub async fn import_scan(
        &self,
        ctx: &RequestCtx,
        input: ImportScanInput,
    ) -> AppResult<ImportRun> {
        if input.source != SOURCE_LINEAR {
            return Err(invalid(
                "source",
                "only the source `linear` can be scanned with a token",
            ));
        }
        let token = Secret::new(&input.token)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        require_workspace(&mut *tx, ctx, Action::ManageImport).await?;
        let now = self.clock.now();
        let id = self.ids.new_uuid();
        let run = tx
            .imports()
            .create_import_run(&NewImportRun {
                id,
                source: SOURCE_LINEAR.to_owned(),
                status: "scanning".to_owned(),
                options: json!({}),
                started_by: actor_id(ctx),
                now,
            })
            .await?;
        let blob = encrypt_token(&self.config.session_secret, id, &token)?;
        tx.imports()
            .put_import_credential(id, &blob, now + Duration::days(CREDENTIAL_TTL_DAYS))
            .await?;
        tx.commit().await?;
        Ok(run)
    }

    /// Legt einen bereits gescannten Lauf an (CLI: CSV-Import, dessen Quelle lokal vorliegt).
    /// Kein Token, Status `planned`; `scan` entspricht `stats.scan` (mindestens `teams`).
    pub async fn import_register_scanned(
        &self,
        ctx: &RequestCtx,
        source: &str,
        scan: Value,
    ) -> AppResult<ImportRun> {
        if source != SOURCE_LINEAR && source != SOURCE_LINEAR_CSV {
            return Err(invalid("source", "unknown import source"));
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        require_workspace(&mut *tx, ctx, Action::ManageImport).await?;
        let now = self.clock.now();
        let id = self.ids.new_uuid();
        tx.imports()
            .create_import_run(&NewImportRun {
                id,
                source: source.to_owned(),
                status: "planned".to_owned(),
                options: json!({}),
                started_by: actor_id(ctx),
                now,
            })
            .await?;
        tx.imports()
            .update_import_run(
                id,
                &ImportRunUpdate {
                    stats: Some(json!({ "scan": scan })),
                    now: Some(now),
                    ..Default::default()
                },
            )
            .await?;
        let run = tx
            .imports()
            .get_import_run(id)
            .await?
            .ok_or_else(not_found)?;
        tx.commit().await?;
        Ok(run)
    }

    /// `importStart`: validiert Team-Auswahl/Key-Konflikte und stellt den Lauf auf `running`
    /// (der Job oder die CLI führt ihn aus).
    pub async fn import_start(
        &self,
        ctx: &RequestCtx,
        input: ImportStartInput,
    ) -> AppResult<ImportRun> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        require_workspace(&mut *tx, ctx, Action::ManageImport).await?;
        let run = tx
            .imports()
            .get_import_run(input.run_id)
            .await?
            .ok_or_else(not_found)?;
        if run.status != "planned" {
            return Err(AppError::conflict(format!(
                "the run is `{}`, only planned runs can be started",
                run.status
            )));
        }
        let scan_teams: Vec<(String, String)> = run.stats["scan"]["teams"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|t| {
                        Some((t["id"].as_str()?.to_owned(), t["key"].as_str()?.to_owned()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let selected: Vec<String> = if input.teams.is_empty() {
            scan_teams.iter().map(|(_, k)| k.clone()).collect()
        } else {
            input.teams.clone()
        };
        if selected.is_empty() {
            return Err(invalid("teams", "no team to import"));
        }
        let mut conflicts = Vec::new();
        let mut targets: BTreeMap<String, String> = BTreeMap::new();
        for key in &selected {
            let Some((src_id, _)) = scan_teams.iter().find(|(_, k)| k == key) else {
                return Err(invalid("teams", format!("unknown team `{key}`")));
            };
            let target = input
                .key_mapping
                .get(key)
                .cloned()
                .unwrap_or_else(|| key.clone());
            if lineal_domain::identifier::TeamKey::parse(&target).is_err() {
                conflicts.push(json!({ "key": key, "target": target, "reason": "invalid_key" }));
                continue;
            }
            if targets.insert(target.clone(), key.clone()).is_some() {
                conflicts
                    .push(json!({ "key": key, "target": target, "reason": "duplicate_target" }));
                continue;
            }
            let existing = tx.imports().find_team_by_key(&target).await?;
            if let Some(team_id) = existing {
                let mapped = tx
                    .imports()
                    .get_mappings(&run.source, "Team", std::slice::from_ref(src_id))
                    .await?;
                if !mapped.iter().any(|m| m.target_id == team_id) {
                    conflicts.push(json!({ "key": key, "target": target, "reason": "key_exists" }));
                }
            }
        }
        if !conflicts.is_empty() {
            return Err(AppError::conflict("team keys conflict with existing teams")
                .with_details(json!({ "conflicts": conflicts })));
        }
        let now = self.clock.now();
        let updated_after = if input.delta {
            tx.imports()
                .latest_completed_import_run(&run.source)
                .await?
                .and_then(|r| r.started_at)
        } else {
            None
        };
        if input.delta && updated_after.is_none() {
            return Err(invalid(
                "delta",
                "there is no completed import to continue from",
            ));
        }
        tx.imports().delete_report_entries(run.id).await?;
        tx.imports()
            .update_import_run(
                run.id,
                &ImportRunUpdate {
                    status: Some("running".into()),
                    options: Some(options_json(&input, &selected, updated_after)),
                    checkpoint: Some(json!({})),
                    started_at: Some(Some(now)),
                    finished_at: Some(None),
                    error: Some(None),
                    now: Some(now),
                    ..Default::default()
                },
            )
            .await?;
        let run = tx
            .imports()
            .get_import_run(run.id)
            .await?
            .ok_or_else(not_found)?;
        tx.commit().await?;
        Ok(run)
    }

    /// `importPause`: ein laufender Lauf hält nach der aktuellen Seite an.
    pub async fn import_pause(&self, ctx: &RequestCtx, run_id: Uuid) -> AppResult<ImportRun> {
        self.import_transition(ctx, run_id, &["running"], "paused", false)
            .await
    }

    /// `importResume`: pausierte oder fehlgeschlagene Läufe laufen ab dem Checkpoint weiter.
    pub async fn import_resume(&self, ctx: &RequestCtx, run_id: Uuid) -> AppResult<ImportRun> {
        self.import_transition(ctx, run_id, &["paused", "failed"], "running", false)
            .await
    }

    /// `importCancel`: beendet den Lauf endgültig und löscht das gespeicherte Token.
    pub async fn import_cancel(&self, ctx: &RequestCtx, run_id: Uuid) -> AppResult<ImportRun> {
        self.import_transition(
            ctx,
            run_id,
            &["scanning", "planned", "running", "paused", "failed"],
            "canceled",
            true,
        )
        .await
    }

    async fn import_transition(
        &self,
        ctx: &RequestCtx,
        run_id: Uuid,
        from: &[&str],
        to: &str,
        terminal: bool,
    ) -> AppResult<ImportRun> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        require_workspace(&mut *tx, ctx, Action::ManageImport).await?;
        let run = tx
            .imports()
            .get_import_run(run_id)
            .await?
            .ok_or_else(not_found)?;
        if !from.contains(&run.status.as_str()) {
            return Err(AppError::conflict(format!(
                "the run is `{}` and cannot change to `{to}`",
                run.status
            )));
        }
        let now = self.clock.now();
        tx.imports()
            .update_import_run(
                run_id,
                &ImportRunUpdate {
                    status: Some(to.to_owned()),
                    finished_at: terminal.then_some(Some(now)),
                    error: if to == "running" { Some(None) } else { None },
                    now: Some(now),
                    ..Default::default()
                },
            )
            .await?;
        if terminal {
            tx.imports().delete_import_credential(run_id).await?;
            // Sperren der betroffenen Teams aufheben.
            let teams = tx
                .imports()
                .list_mappings(&run.source, "Team", None, 1000)
                .await?;
            let ids: Vec<Uuid> = teams.iter().map(|m| m.target_id).collect();
            tx.imports().set_team_import_lock(&ids, None).await?;
        }
        let run = tx
            .imports()
            .get_import_run(run_id)
            .await?
            .ok_or_else(not_found)?;
        tx.commit().await?;
        Ok(run)
    }

    /// `importRun(id)`.
    pub async fn import_get_run(&self, ctx: &RequestCtx, run_id: Uuid) -> AppResult<ImportRun> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        require_workspace(&mut *tx, ctx, Action::ManageImport).await?;
        tx.imports()
            .get_import_run(run_id)
            .await?
            .ok_or_else(not_found)
    }

    /// `importRuns`: neueste zuerst.
    pub async fn import_list_runs(
        &self,
        ctx: &RequestCtx,
        limit: i64,
    ) -> AppResult<Vec<ImportRun>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        require_workspace(&mut *tx, ctx, Action::ManageImport).await?;
        tx.imports().list_import_runs(limit).await
    }

    /// Bericht eines Laufs (Einträge und Zähler je Schweregrad).
    pub async fn import_report(
        &self,
        ctx: &RequestCtx,
        run_id: Uuid,
        severity: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> AppResult<(
        Vec<crate::ports::repos::import::ReportEntry>,
        Vec<(String, i64)>,
    )> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        require_workspace(&mut *tx, ctx, Action::ManageImport).await?;
        tx.imports()
            .get_import_run(run_id)
            .await?
            .ok_or_else(not_found)?;
        let entries = tx
            .imports()
            .list_report_entries(run_id, severity, limit, offset)
            .await?;
        let counts = tx.imports().report_counts(run_id).await?;
        Ok((entries, counts))
    }

    /// Entschlüsselt das Token eines Laufs (nur für den Importer in Job/CLI; System- oder
    /// Admin-Kontext, kein GraphQL-Pfad).
    pub async fn import_load_token(
        &self,
        org_run: (lineal_domain::ids::OrgId, Uuid),
    ) -> AppResult<Option<Secret>> {
        let mut tx = self.store.begin(org_run.0).await?;
        let blob = tx.imports().get_import_credential(org_run.1).await?;
        match blob {
            Some(b) => Ok(Some(decrypt_token(
                &self.config.session_secret,
                org_run.1,
                &b,
            )?)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_is_redacted_everywhere() {
        let s = Secret::new("lin_api_TESTSECRET1234").unwrap();
        assert!(!format!("{s:?}").contains("TESTSECRET"));
        assert!(!format!("{s}").contains("TESTSECRET"));
        assert!(!format!("{:?}", Some(&s)).contains("TESTSECRET"));
    }

    #[test]
    fn token_roundtrip_is_bound_to_run_and_secret() {
        let run = Uuid::new_v4();
        let t = Secret::new("lin_api_TESTSECRET1234").unwrap();
        let blob = encrypt_token("a-session-secret-of-32-bytes-or-more", run, &t).unwrap();
        assert!(
            !blob.windows(10).any(|w| w == b"TESTSECRET"),
            "ciphertext must not contain the token"
        );
        assert_eq!(
            decrypt_token("a-session-secret-of-32-bytes-or-more", run, &blob)
                .unwrap()
                .expose(),
            t.expose()
        );
        assert!(decrypt_token("another-session-secret-32-bytes-xx", run, &blob).is_err());
        assert!(
            decrypt_token(
                "a-session-secret-of-32-bytes-or-more",
                Uuid::new_v4(),
                &blob
            )
            .is_err()
        );
        // zwei Verschlüsselungen desselben Tokens unterscheiden sich (Zufalls-Nonce)
        let blob2 = encrypt_token("a-session-secret-of-32-bytes-or-more", run, &t).unwrap();
        assert_ne!(blob, blob2);
    }

    #[test]
    fn token_format_is_validated() {
        assert!(Secret::new("").is_err());
        assert!(Secret::new("short").is_err());
        assert!(Secret::new("has space inside the token").is_err());
        assert!(Secret::new(&"x".repeat(600)).is_err());
        assert!(Secret::new("  lin_api_abcdefgh  ").is_ok());
    }
}
