//! usecases – Webhooks (WP16, docs/09 §4): CRUD, Secret-Verschlüsselung, Signatur, SSRF-Schutz,
//! Retry-Plan, Planung der Zustellungen aus Outbox-Events.
//!
//! Der HTTP-Versand selbst liegt im Konsumenten `lineal-jobs/src/consumers/webhooks.rs`; er nutzt
//! die reinen Hilfen dieses Moduls (`sign`, `UrlPolicy`, `resolve_target`, `retry_delay`) und die
//! Use-Cases `claim_due_webhook_deliveries` / `record_webhook_attempt`.

use super::audit::AuditEvent;
use crate::app::App;
use crate::context::{Actor, RequestCtx};
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::repos::StoredEvent;
use crate::ports::repos::webhook::{
    DeliveryOutcome, DeliveryRecord, DueDelivery, NewDelivery, NewWebhook, WebhookPatch,
    WebhookRecord,
};
use crate::resolve;
use crate::resolve::team::Visibility;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use lineal_domain::enums::TeamVisibility;
use lineal_domain::events::{DomainEvent, EventOrigin};
use lineal_domain::ids::{TeamId, WebhookId};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use url::{Host, Url};
use uuid::Uuid;

/// Abonnierbare Ressourcentypen (docs/09 §4.2; deckt sich mit dem CHECK der Tabelle).
pub const RESOURCE_TYPES: [&str; 13] = [
    "Issue",
    "Comment",
    "Attachment",
    "IssueLabel",
    "Reaction",
    "Project",
    "ProjectUpdate",
    "Cycle",
    "Document",
    "Initiative",
    "InitiativeUpdate",
    "User",
    "IssueSLA",
];

/// Maximale Anzahl Webhooks je Workspace.
pub const MAX_WEBHOOKS: usize = 100;
/// Frist, in der nach einer Rotation zusätzlich `Lineal-Signature-Previous` gesendet wird.
pub const PREVIOUS_SECRET_GRACE_HOURS: i64 = 24;
/// Nach so vielen aufgegebenen Zustellungen in Folge (ohne Erfolg in 24 h) wird deaktiviert.
pub const DISABLE_AFTER_FAILURES: i32 = 5;

// ---------------------------------------------------------------------------
// Secret: erzeugen, verschlüsseln, signieren
// ---------------------------------------------------------------------------

/// 32 Byte Zufall als Hex (2 × UUIDv4 aus dem OS-RNG).
pub fn generate_secret() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn secret_key(session_secret: &str) -> Key {
    let mut h = Sha256::new();
    h.update(b"lineal-webhook-secret-v1");
    h.update(session_secret.as_bytes());
    Key::clone_from_slice(&h.finalize())
}

/// Verschlüsselt `plaintext` (ChaCha20-Poly1305, Schlüssel aus `session_secret`) → `v1:<base64>`.
pub fn seal_secret(session_secret: &str, plaintext: &str) -> AppResult<String> {
    let cipher = ChaCha20Poly1305::new(&secret_key(session_secret));
    let nonce_bytes = Uuid::new_v4();
    let nonce = Nonce::from_slice(&nonce_bytes.as_bytes()[..12]);
    let ct = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|_| AppError::internal("could not encrypt the webhook secret"))?;
    let mut raw = nonce_bytes.as_bytes()[..12].to_vec();
    raw.extend(ct);
    Ok(format!("v1:{}", B64.encode(raw)))
}

/// Entschlüsselt ein mit [`seal_secret`] erzeugtes Secret.
pub fn open_secret(session_secret: &str, sealed: &str) -> AppResult<String> {
    let bad = || AppError::internal("could not decrypt the webhook secret");
    let raw = sealed.strip_prefix("v1:").ok_or_else(bad)?;
    let raw = B64.decode(raw).map_err(|_| bad())?;
    if raw.len() < 12 + 16 {
        return Err(bad());
    }
    let (nonce, ct) = raw.split_at(12);
    let cipher = ChaCha20Poly1305::new(&secret_key(session_secret));
    let pt = cipher
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|_| bad())?;
    String::from_utf8(pt).map_err(|_| bad())
}

/// `hex(HMAC-SHA256(secret, body))` (docs/09 §4.4).
pub fn sign(secret: &str, body: &[u8]) -> String {
    // HMAC akzeptiert Schlüssel beliebiger Länge.
    let Ok(mut mac) = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes()) else {
        return String::new();
    };
    mac.update(body);
    format!("{:x}", mac.finalize().into_bytes())
}

// ---------------------------------------------------------------------------
// SSRF-Schutz
// ---------------------------------------------------------------------------

/// Regeln für Webhook-Ziele. `STRICT` = Produktion: nur https, keine nicht-öffentlichen IPs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UrlPolicy {
    /// `http://` erlauben (nur Dev-Modus).
    pub allow_http: bool,
    /// Loopback-Ziele erlauben (nur Dev-Modus; alle übrigen privaten Bereiche bleiben gesperrt).
    pub allow_loopback: bool,
}

impl UrlPolicy {
    pub const STRICT: UrlPolicy = UrlPolicy {
        allow_http: false,
        allow_loopback: false,
    };
    pub const DEV: UrlPolicy = UrlPolicy {
        allow_http: true,
        allow_loopback: true,
    };

    /// Dev-Modus, wenn die Basis-URL des Servers `http://` ist.
    pub fn for_app(app: &App) -> Self {
        if app
            .config
            .base_url
            .trim()
            .to_ascii_lowercase()
            .starts_with("http://")
        {
            Self::DEV
        } else {
            Self::STRICT
        }
    }
}

fn blocked_v4(ip: Ipv4Addr, policy: UrlPolicy) -> bool {
    let o = ip.octets();
    if ip.is_loopback() {
        return !policy.allow_loopback;
    }
    ip.is_unspecified()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10 (CGNAT)
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // 198.18.0.0/15
        || o[0] >= 240 // reserviert
        || o[0] == 0
}

fn blocked_v6(ip: Ipv6Addr, policy: UrlPolicy) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return blocked_v4(v4, policy);
    }
    if ip.is_loopback() {
        return !policy.allow_loopback;
    }
    let s = ip.segments();
    // NAT64 (64:ff9b::/96) und IPv4-kompatible Adressen tragen eine IPv4-Adresse.
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6].iter().all(|x| *x == 0) {
        let v4 = Ipv4Addr::new(
            (s[6] >> 8) as u8,
            (s[6] & 0xff) as u8,
            (s[7] >> 8) as u8,
            (s[7] & 0xff) as u8,
        );
        return blocked_v4(v4, policy);
    }
    ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || (s[0] & 0xffc0) == 0xfec0 // site-local
        || (s[0] == 0x2001 && s[1] == 0x0db8) // Dokumentation
        || s[..6].iter().all(|x| *x == 0) // ::a.b.c.d (IPv4-kompatibel, veraltet)
}

/// Ob die IP ein verbotenes Ziel ist (privat, Loopback, Link-local, …).
pub fn ip_is_blocked(ip: IpAddr, policy: UrlPolicy) -> bool {
    match ip {
        IpAddr::V4(v4) => blocked_v4(v4, policy),
        IpAddr::V6(v6) => blocked_v6(v6, policy),
    }
}

fn invalid_url(msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg).with_details(json!({ "field": "url" }))
}

/// Syntaktische Prüfung eines Ziels (Schema, Host, Userinfo, IP-Literale, offensichtliche
/// interne Namen). Die DNS-Auflösung prüft [`resolve_target`].
pub fn parse_target_url(raw: &str, policy: UrlPolicy) -> AppResult<Url> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 2000 {
        return Err(invalid_url("url must be 1-2000 characters"));
    }
    let url = Url::parse(raw).map_err(|_| invalid_url("url is not a valid URL"))?;
    match url.scheme() {
        "https" => {}
        "http" if policy.allow_http => {}
        _ => return Err(invalid_url("url must use https")),
    }
    // Nur Standardports: 443 (Dev zusätzlich 80 und frei wählbare Ports für lokale Empfänger).
    if !policy.allow_http && url.port().is_some_and(|p| p != 443) {
        return Err(invalid_url("only port 443 is allowed"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid_url("url must not contain credentials"));
    }
    if url.fragment().is_some() {
        return Err(invalid_url("url must not contain a fragment"));
    }
    match url.host() {
        None => return Err(invalid_url("url needs a host")),
        Some(Host::Ipv4(ip)) => {
            if ip_is_blocked(IpAddr::V4(ip), policy) {
                return Err(invalid_url("the target address is not allowed"));
            }
        }
        Some(Host::Ipv6(ip)) => {
            if ip_is_blocked(IpAddr::V6(ip), policy) {
                return Err(invalid_url("the target address is not allowed"));
            }
        }
        Some(Host::Domain(d)) => {
            let d = d.trim_end_matches('.').to_ascii_lowercase();
            let internal = d == "localhost"
                || d.ends_with(".localhost")
                || d.ends_with(".local")
                || d.ends_with(".internal")
                || d.ends_with(".localdomain")
                || !d.contains('.') && !policy.allow_loopback;
            if internal
                && !(policy.allow_loopback && (d == "localhost" || d.ends_with(".localhost")))
            {
                return Err(invalid_url("the target host is not allowed"));
            }
        }
    }
    Ok(url)
}

/// Löst den Host auf und verlangt, dass **alle** Adressen erlaubt sind. Liefert die geprüften
/// Adressen, an die der Aufrufer die Verbindung bindet (Schutz gegen DNS-Rebinding).
pub async fn resolve_target(url: &Url, policy: UrlPolicy) -> AppResult<Vec<SocketAddr>> {
    let host = url
        .host_str()
        .ok_or_else(|| invalid_url("url needs a host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| invalid_url("url needs a port"))?;
    let lookup = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::lookup_host((host.trim_start_matches('[').trim_end_matches(']'), port)),
    )
    .await
    .map_err(|_| invalid_url("the host could not be resolved in time"))?
    .map_err(|_| invalid_url("the host could not be resolved"))?;
    let addrs: Vec<SocketAddr> = lookup.collect();
    check_resolved(&addrs, policy)?;
    Ok(addrs)
}

/// Prüft aufgelöste Adressen gegen die Policy (rein, für Tests).
pub fn check_resolved(addrs: &[SocketAddr], policy: UrlPolicy) -> AppResult<()> {
    if addrs.is_empty() {
        return Err(invalid_url("the host did not resolve to any address"));
    }
    if addrs.iter().any(|a| ip_is_blocked(a.ip(), policy)) {
        return Err(invalid_url("the target address is not allowed"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Retry-Plan
// ---------------------------------------------------------------------------

/// Wartezeiten der Retries nach dem 1., 2. und 3. fehlgeschlagenen Versuch (docs/09 §4.5).
pub const RETRY_DELAYS_SECS: [i64; 3] = [60, 3600, 6 * 3600];

/// Verzögerung bis zum nächsten Versuch nach `attempts_done` Versuchen (ohne Jitter);
/// `None`, wenn aufgegeben wird.
pub fn retry_delay(attempts_done: i32) -> Option<Duration> {
    let idx = usize::try_from(attempts_done.checked_sub(1)?).ok()?;
    RETRY_DELAYS_SECS.get(idx).map(|s| Duration::seconds(*s))
}

/// Zeitpunkt des nächsten Versuchs mit ±10 % Jitter (`jitter_seed` bestimmt den Jitter).
pub fn next_attempt_at(
    now: DateTime<Utc>,
    attempts_done: i32,
    jitter_seed: u64,
) -> Option<DateTime<Utc>> {
    let base = retry_delay(attempts_done)?;
    // Faktor in [-0.1, +0.1] in Tausendstel-Schritten.
    let permille = i64::try_from(jitter_seed % 201).unwrap_or(0) - 100;
    let ms = base.num_milliseconds();
    let jitter = ms.saturating_mul(permille) / 1000;
    Some(now + Duration::milliseconds(ms.saturating_add(jitter)))
}

// ---------------------------------------------------------------------------
// Event → Ressourcentyp
// ---------------------------------------------------------------------------

/// Abbildung eines Domain-Events auf einen Webhook-Ressourcentyp.
#[derive(Debug, Clone, PartialEq)]
pub struct MappedEvent {
    pub resource_type: &'static str,
    pub action: &'static str,
    /// ID der Entität (Issue, Comment, …); bei `IssueSLA` die des Issues.
    pub entity_id: Uuid,
    /// Geänderte Felder (nur Issue-Updates).
    pub changed: Vec<String>,
    /// Team laut Event (Fallback, wenn die Entität nicht mehr existiert).
    pub team_hint: Option<TeamId>,
}

/// `None` für Events ohne Webhook-Ressourcentyp (Team, Status, Notification, …).
pub fn map_event(ev: &DomainEvent) -> Option<MappedEvent> {
    use DomainEvent as E;
    let m = |t, a, id: Uuid| MappedEvent {
        resource_type: t,
        action: a,
        entity_id: id,
        changed: Vec::new(),
        team_hint: None,
    };
    Some(match ev {
        E::IssueCreated {
            issue_id, team_id, ..
        } => MappedEvent {
            team_hint: Some(*team_id),
            ..m("Issue", "create", issue_id.0)
        },
        E::IssueUpdated {
            issue_id,
            team_id,
            changed,
            ..
        } => MappedEvent {
            team_hint: Some(*team_id),
            changed: changed.clone(),
            ..m("Issue", "update", issue_id.0)
        },
        E::IssueRemoved { issue_id, team_id } => MappedEvent {
            team_hint: Some(*team_id),
            ..m("Issue", "remove", issue_id.0)
        },
        E::CommentCreated { comment_id, .. } => m("Comment", "create", comment_id.0),
        E::CommentUpdated { comment_id, .. } => m("Comment", "update", comment_id.0),
        E::CommentRemoved { comment_id } => m("Comment", "remove", comment_id.0),
        E::IssueLabelCreated { label_id } => m("IssueLabel", "create", label_id.0),
        E::IssueLabelUpdated { label_id } => m("IssueLabel", "update", label_id.0),
        E::IssueLabelRemoved { label_id } => m("IssueLabel", "remove", label_id.0),
        E::ReactionCreated { reaction_id } => m("Reaction", "create", reaction_id.0),
        E::ReactionRemoved { reaction_id } => m("Reaction", "remove", reaction_id.0),
        E::AttachmentCreated { attachment_id } => m("Attachment", "create", attachment_id.0),
        E::AttachmentUpdated { attachment_id } => m("Attachment", "update", attachment_id.0),
        E::AttachmentRemoved { attachment_id } => m("Attachment", "remove", attachment_id.0),
        E::ProjectCreated { project_id } => m("Project", "create", project_id.0),
        E::ProjectUpdated { project_id, .. } => m("Project", "update", project_id.0),
        E::ProjectRemoved { project_id } => m("Project", "remove", project_id.0),
        E::ProjectUpdateCreated { update_id, .. } => m("ProjectUpdate", "create", update_id.0),
        E::ProjectUpdateUpdated { update_id, .. } => m("ProjectUpdate", "update", update_id.0),
        E::ProjectUpdateRemoved { update_id, .. } => m("ProjectUpdate", "remove", update_id.0),
        E::CycleCreated { cycle_id, team_id } => MappedEvent {
            team_hint: Some(*team_id),
            ..m("Cycle", "create", cycle_id.0)
        },
        E::CycleUpdated { cycle_id, team_id } => MappedEvent {
            team_hint: Some(*team_id),
            ..m("Cycle", "update", cycle_id.0)
        },
        E::CycleRemoved { cycle_id, team_id } => MappedEvent {
            team_hint: Some(*team_id),
            ..m("Cycle", "remove", cycle_id.0)
        },
        E::DocumentCreated { document_id } => m("Document", "create", document_id.0),
        E::DocumentUpdated { document_id } => m("Document", "update", document_id.0),
        E::DocumentRemoved { document_id } => m("Document", "remove", document_id.0),
        E::InitiativeCreated { initiative_id } => m("Initiative", "create", initiative_id.0),
        E::InitiativeUpdated { initiative_id, .. } => m("Initiative", "update", initiative_id.0),
        E::InitiativeRemoved { initiative_id } => m("Initiative", "remove", initiative_id.0),
        E::InitiativeUpdateCreated { update_id, .. } => {
            m("InitiativeUpdate", "create", update_id.0)
        }
        E::InitiativeUpdateUpdated { update_id, .. } => {
            m("InitiativeUpdate", "update", update_id.0)
        }
        E::InitiativeUpdateRemoved { update_id, .. } => {
            m("InitiativeUpdate", "remove", update_id.0)
        }
        E::UserCreated { user_id } => m("User", "create", user_id.0),
        E::UserUpdated { user_id } => m("User", "update", user_id.0),
        E::IssueSlaSet { issue_id } => m("IssueSLA", "set", issue_id.0),
        E::IssueSlaHighRisk { issue_id } => m("IssueSLA", "highRisk", issue_id.0),
        E::IssueSlaBreached { issue_id } => m("IssueSLA", "breached", issue_id.0),
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Sichtbarkeit und Payload
// ---------------------------------------------------------------------------

/// Entscheidet, ob `hook` ein Event für die Teams der Entität erhält, und liefert die Teams, die in
/// `data` sichtbar bleiben (docs/09 §4.1, Dispatcher-Pseudocode).
///
/// - Team-Webhook: nur, wenn sein Team zu den Teams der Entität gehört; sichtbar bleibt nur dieses.
/// - `allPublicTeams`: nur öffentliche Teams; hat die Entität Teams, aber keines ist öffentlich,
///   gibt es **keine** Zustellung. Workspace-weite Entitäten (ohne Teams) gehen an `allPublicTeams`.
pub fn visible_teams_for_hook(
    hook: &WebhookRecord,
    entity_teams: &[TeamId],
    visibility_of: &dyn Fn(TeamId) -> Option<TeamVisibility>,
) -> Option<BTreeSet<TeamId>> {
    if let Some(t) = hook.team_id {
        return entity_teams.contains(&t).then(|| BTreeSet::from([t]));
    }
    if !hook.all_public_teams {
        return None;
    }
    if entity_teams.is_empty() {
        return Some(BTreeSet::new());
    }
    let public: BTreeSet<TeamId> = entity_teams
        .iter()
        .copied()
        .filter(|t| visibility_of(*t) == Some(TeamVisibility::Public))
        .collect();
    (!public.is_empty()).then_some(public)
}

/// Entfernt Teams, die der Empfänger nicht sehen darf, aus der serialisierten Entität
/// (`teamIds`, `teams`, `leadTeamId`/`leadTeam`, `teamId`/`team`).
pub fn strip_hidden_teams(data: &mut Value, hidden: &BTreeSet<String>) {
    let Some(obj) = data.as_object_mut() else {
        return;
    };
    let is_hidden = |v: &Value| v.as_str().is_some_and(|s| hidden.contains(s));
    if let Some(Value::Array(a)) = obj.get_mut("teamIds") {
        a.retain(|v| !is_hidden(v));
    }
    if let Some(Value::Array(a)) = obj.get_mut("teams") {
        a.retain(|t| !t.get("id").is_some_and(&is_hidden));
    }
    for (id_key, obj_key) in [("leadTeamId", "leadTeam"), ("teamId", "team")] {
        if obj.get(id_key).is_some_and(&is_hidden) {
            obj.insert(id_key.into(), Value::Null);
            obj.insert(obj_key.into(), Value::Null);
        }
    }
}

/// Baut das Payload (ohne `webhookTimestamp`/`webhookId`).
#[allow(clippy::too_many_arguments)]
pub fn build_payload(
    mapped: &MappedEvent,
    actor: Option<Value>,
    created_at: DateTime<Utc>,
    data: Value,
    updated_from: Option<Value>,
    url: &str,
    org_id: Uuid,
) -> Value {
    let mut o = Map::new();
    o.insert("action".into(), json!(mapped.action));
    o.insert("type".into(), json!(mapped.resource_type));
    o.insert("actor".into(), actor.unwrap_or(Value::Null));
    o.insert(
        "createdAt".into(),
        json!(created_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
    );
    o.insert("data".into(), data);
    if mapped.action == "update" || mapped.action == "highRisk" || mapped.action == "breached" {
        if let Some(u) = updated_from {
            o.insert("updatedFrom".into(), u);
        }
    }
    o.insert("url".into(), json!(url));
    o.insert("organizationId".into(), json!(org_id.to_string()));
    Value::Object(o)
}

/// Vollständiger Body eines Versuchs: Payload plus aktueller `webhookTimestamp` (ms) und `webhookId`.
pub fn build_body(
    payload: &Value,
    webhook_id: WebhookId,
    now: DateTime<Utc>,
) -> AppResult<Vec<u8>> {
    let mut v = payload.clone();
    let Some(o) = v.as_object_mut() else {
        return Err(AppError::internal("webhook payload is not an object"));
    };
    o.insert("webhookTimestamp".into(), json!(now.timestamp_millis()));
    o.insert("webhookId".into(), json!(webhook_id.0.to_string()));
    serde_json::to_vec(&v).map_err(|e| AppError::internal(format!("payload serialization: {e}")))
}

// ---------------------------------------------------------------------------
// Use-Cases
// ---------------------------------------------------------------------------

/// Eingabe für `create_webhook`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CreateWebhookInput {
    pub label: Option<String>,
    pub url: String,
    /// Team-Referenz; genau eines von `team`/`all_public_teams`.
    pub team: Option<String>,
    pub all_public_teams: bool,
    pub resource_types: Vec<String>,
}

/// Eingabe für `update_webhook`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateWebhookInput {
    pub label: Option<Option<String>>,
    pub url: Option<String>,
    pub enabled: Option<bool>,
    pub resource_types: Option<Vec<String>>,
}

/// Webhook mit dem Klartext-Secret (nur bei Anlage und Rotation gesetzt).
#[derive(Debug, Clone, PartialEq)]
pub struct WebhookView {
    pub webhook: WebhookRecord,
    pub secret: Option<String>,
}

fn webhook_not_found() -> AppError {
    AppError::not_found("Could not find referenced Webhook.")
        .with_details(json!({ "type": "Webhook" }))
}

fn validate_label(label: &Option<String>) -> AppResult<Option<String>> {
    match label.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(l) if l.chars().count() > 100 => Err(AppError::invalid_request(
            "label must be at most 100 characters",
        )
        .with_details(json!({ "field": "label" }))),
        Some(l) => Ok(Some(l.to_string())),
    }
}

fn validate_resource_types(types: &[String]) -> AppResult<Vec<String>> {
    if types.is_empty() {
        return Err(AppError::invalid_request("resourceTypes must not be empty")
            .with_details(json!({ "field": "resourceTypes" })));
    }
    let mut out: Vec<String> = Vec::new();
    for t in types {
        if !RESOURCE_TYPES.contains(&t.as_str()) {
            return Err(
                AppError::invalid_request(format!("unknown resource type `{t}`"))
                    .with_details(json!({ "field": "resourceTypes", "value": t })),
            );
        }
        if !out.contains(t) {
            out.push(t.clone());
        }
    }
    Ok(out)
}

impl App {
    fn webhook_policy(&self) -> UrlPolicy {
        UrlPolicy::for_app(self)
    }

    /// Prüft das Ziel (Syntax + DNS-Auflösung + IP-Bereiche).
    async fn check_webhook_target(&self, raw: &str) -> AppResult<String> {
        let policy = self.webhook_policy();
        let url = parse_target_url(raw, policy)?;
        resolve_target(&url, policy).await?;
        Ok(url.to_string())
    }

    /// Recht auf einen vorhandenen Webhook: Team-Webhook → Team-Recht (nicht sichtbar → `not_found`),
    /// Workspace-Webhook → `manageWebhooks`.
    fn authorize_webhook(
        snap: &PolicySnapshot,
        ctx: &RequestCtx,
        w: &WebhookRecord,
    ) -> AppResult<()> {
        match w.team_id {
            // Webhooks nicht sichtbarer Teams (privat, Nicht-Mitglied) sind auch für Admins unsichtbar.
            Some(t) if !snap.visible_team_ids().contains(&t) => Err(webhook_not_found()),
            Some(t) => snap
                .require_team(ctx, Action::CreateTeamWebhook, t)
                .map(|_| ()),
            None => snap.require_workspace(ctx, Action::ManageWebhooks),
        }
    }

    /// Legt einen Webhook an. Workspace-Webhooks (`allPublicTeams`) brauchen `manageWebhooks`
    /// (Admin-Scope), Team-Webhooks das Team-Recht `CreateTeamWebhook`; beides setzt Scope `write` voraus.
    pub async fn create_webhook(
        &self,
        ctx: &RequestCtx,
        input: CreateWebhookInput,
    ) -> AppResult<WebhookView> {
        let creator = match ctx.actor {
            Actor::User { id, .. } | Actor::App { id } => id,
            Actor::System => {
                return Err(AppError::forbidden("Webhooks need a user or app actor."));
            }
        };
        if input.all_public_teams == input.team.is_some() {
            return Err(AppError::invalid_request(
                "exactly one of `team` and `allPublicTeams` must be set",
            )
            .with_details(json!({ "field": "team" })));
        }
        let label = validate_label(&input.label)?;
        let resource_types = validate_resource_types(&input.resource_types)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team_id = match &input.team {
            Some(r) => {
                let team =
                    resolve::team::resolve_in(&mut *tx, &snap, r, Visibility::Content).await?;
                snap.require_team(ctx, Action::CreateTeamWebhook, team.id)?;
                Some(team.id)
            }
            None => {
                snap.require_workspace(ctx, Action::ManageWebhooks)?;
                None
            }
        };
        if tx.webhooks().list_webhooks().await?.len() >= MAX_WEBHOOKS {
            return Err(AppError::limit_exceeded("Too many webhooks.")
                .with_details(json!({ "limit": "maxWebhooks" })));
        }
        let url = self.check_webhook_target(&input.url).await?;
        // Nur der Host landet im Audit-Log: Pfad und Query können Zugangsdaten enthalten.
        let audit_host = url::Url::parse(&url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned));
        let audit_team_scoped = team_id.is_some();
        let secret = generate_secret();
        let now = self.clock.now();
        let id = WebhookId(self.ids.new_uuid());
        tx.webhooks()
            .insert_webhook(&NewWebhook {
                id,
                label,
                url,
                secret_enc: seal_secret(&self.config.session_secret, &secret)?,
                team_id,
                all_public_teams: input.all_public_teams,
                resource_types,
                creator_id: creator,
                oauth_application_id: None,
                now,
            })
            .await?;
        let webhook = tx
            .webhooks()
            .get_webhook(id)
            .await?
            .ok_or_else(|| AppError::internal("webhook vanished after insert"))?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new(
                "webhook.created",
                "webhook",
                Some(id.0),
                json!({ "host": audit_host, "team_scoped": audit_team_scoped }),
            ),
        )
        .await?;
        tx.commit().await?;
        Ok(WebhookView {
            webhook,
            secret: Some(secret),
        })
    }

    /// Die für den Aufrufer verwaltbaren Webhooks (Team-Webhooks nur mit Team-Recht).
    pub async fn list_webhooks(&self, ctx: &RequestCtx) -> AppResult<Vec<WebhookRecord>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let all = tx.webhooks().list_webhooks().await?;
        Ok(all
            .into_iter()
            .filter(|w| Self::authorize_webhook(&snap, ctx, w).is_ok())
            .collect())
    }

    /// Ein Webhook (Verwaltungssicht).
    pub async fn get_webhook(&self, ctx: &RequestCtx, id: WebhookId) -> AppResult<WebhookRecord> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let w = tx
            .webhooks()
            .get_webhook(id)
            .await?
            .ok_or_else(webhook_not_found)?;
        Self::authorize_webhook(&snap, ctx, &w).map_err(|e| hide_forbidden(e, &w, &snap))?;
        Ok(w)
    }

    /// Ändert Label, URL, Aktivierung oder Ressourcentypen. Reaktivieren setzt den Fehlerzähler zurück.
    pub async fn update_webhook(
        &self,
        ctx: &RequestCtx,
        id: WebhookId,
        input: UpdateWebhookInput,
    ) -> AppResult<WebhookRecord> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let w = tx
            .webhooks()
            .get_webhook(id)
            .await?
            .ok_or_else(webhook_not_found)?;
        Self::authorize_webhook(&snap, ctx, &w).map_err(|e| hide_forbidden(e, &w, &snap))?;
        let mut patch = WebhookPatch::default();
        if let Some(l) = &input.label {
            patch.label = Some(validate_label(l)?);
        }
        if let Some(u) = &input.url {
            patch.url = Some(self.check_webhook_target(u).await?);
        }
        patch.enabled = input.enabled;
        if let Some(t) = &input.resource_types {
            patch.resource_types = Some(validate_resource_types(t)?);
        }
        let updated = tx
            .webhooks()
            .update_webhook(id, &patch, self.clock.now())
            .await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Erzeugt ein neues Secret; das alte wird 24 h lang zusätzlich als `Lineal-Signature-Previous` gesendet.
    pub async fn rotate_webhook_secret(
        &self,
        ctx: &RequestCtx,
        id: WebhookId,
    ) -> AppResult<WebhookView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let w = tx
            .webhooks()
            .get_webhook(id)
            .await?
            .ok_or_else(webhook_not_found)?;
        Self::authorize_webhook(&snap, ctx, &w).map_err(|e| hide_forbidden(e, &w, &snap))?;
        let secret = generate_secret();
        let sealed = seal_secret(&self.config.session_secret, &secret)?;
        let updated = tx
            .webhooks()
            .update_webhook(
                id,
                &WebhookPatch {
                    rotate_secret: Some((sealed, w.secret_enc.clone())),
                    ..Default::default()
                },
                self.clock.now(),
            )
            .await?;
        tx.commit().await?;
        Ok(WebhookView {
            webhook: updated,
            secret: Some(secret),
        })
    }

    /// Löscht (archiviert) den Webhook.
    pub async fn delete_webhook(&self, ctx: &RequestCtx, id: WebhookId) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let w = tx
            .webhooks()
            .get_webhook(id)
            .await?
            .ok_or_else(webhook_not_found)?;
        Self::authorize_webhook(&snap, ctx, &w).map_err(|e| hide_forbidden(e, &w, &snap))?;
        tx.webhooks().archive_webhook(id, self.clock.now()).await?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new("webhook.deleted", "webhook", Some(id.0), json!({})),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Die letzten Zustellungen (max. 100) eines Webhooks.
    pub async fn list_webhook_deliveries(
        &self,
        ctx: &RequestCtx,
        id: WebhookId,
    ) -> AppResult<Vec<DeliveryRecord>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let w = tx
            .webhooks()
            .get_webhook(id)
            .await?
            .ok_or_else(webhook_not_found)?;
        Self::authorize_webhook(&snap, ctx, &w).map_err(|e| hide_forbidden(e, &w, &snap))?;
        tx.webhooks().list_deliveries(id, 100).await
    }

    /// „Erneut senden“: setzt eine Zustellung auf sofort fällig.
    pub async fn resend_webhook_delivery(
        &self,
        ctx: &RequestCtx,
        id: WebhookId,
        delivery: Uuid,
    ) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let w = tx
            .webhooks()
            .get_webhook(id)
            .await?
            .ok_or_else(webhook_not_found)?;
        Self::authorize_webhook(&snap, ctx, &w).map_err(|e| hide_forbidden(e, &w, &snap))?;
        let known = tx
            .webhooks()
            .list_deliveries(id, 100)
            .await?
            .iter()
            .any(|d| d.id == delivery);
        if !known
            || !tx
                .webhooks()
                .requeue_delivery(delivery, self.clock.now())
                .await?
        {
            return Err(
                AppError::not_found("Could not find referenced WebhookDelivery.")
                    .with_details(json!({ "type": "WebhookDelivery" })),
            );
        }
        tx.commit().await?;
        Ok(())
    }

    // --- Zustellung -------------------------------------------------------

    /// Plant die Zustellungen zu einem Outbox-Event (idempotent über `(Webhook, Event)`).
    /// Events mit `origin = import` werden übersprungen. Liefert die Anzahl neu geplanter Zustellungen.
    pub async fn plan_webhook_deliveries(&self, ev: &StoredEvent) -> AppResult<usize> {
        if ev.origin == EventOrigin::Import {
            return Ok(0);
        }
        let Some(mapped) = map_event(&ev.event) else {
            return Ok(0);
        };
        let mut tx = self.store.begin(ev.org_id).await?;
        let hooks = tx
            .webhooks()
            .list_dispatch_webhooks(mapped.resource_type)
            .await?;
        if hooks.is_empty() {
            return Ok(0);
        }
        let entity = tx
            .webhooks()
            .load_webhook_entity(mapped.resource_type, mapped.entity_id, &mapped.changed)
            .await?;
        let (teams, data, updated_from, url_path) = match entity {
            Some(e) => (e.team_ids, e.data, e.updated_from, e.url_path),
            // Entität weg (endgültig gelöscht): nur mit Team-Hinweis zustellbar.
            None => match mapped.team_hint {
                Some(t) => (
                    vec![t],
                    json!({ "id": mapped.entity_id.to_string() }),
                    None,
                    String::new(),
                ),
                None => {
                    tracing::debug!(event = %ev.id, "webhook: entity gone and no team hint, skipped");
                    return Ok(0);
                }
            },
        };
        let all_teams = tx.teams().list_all().await?;
        let visibility = |t: TeamId| {
            all_teams
                .iter()
                .find(|x| x.id == t && !x.trashed && x.retired_at.is_none())
                .map(|x| x.visibility)
        };
        let org = tx
            .orgs()
            .get()
            .await?
            .ok_or_else(|| AppError::internal("workspace vanished"))?;
        let actor = match ev.actor_id {
            Some(a) => tx.users().get(a).await?.map(|u| {
                json!({
                    "id": u.id.to_string(),
                    "type": if u.role == lineal_domain::enums::UserRole::App { "integration" } else { "user" },
                    "name": u.name,
                })
            }),
            None => Some(json!({ "type": "system" })),
        };
        let base = self.config.base_url.trim_end_matches('/');
        let url = if url_path.is_empty() {
            format!("{base}/{}", org.url_key)
        } else {
            format!("{base}/{}/{url_path}", org.url_key)
        };
        let now = self.clock.now();
        let mut planned = 0;
        for hook in hooks {
            // Der Ersteller muss die Daten des Teams weiterhin sehen dürfen (aktiv, bei nicht
            // öffentlichen Teams Mitglied bzw. Admin).
            if let Some(t) = hook.team_id {
                let creator = tx.users().get(hook.creator_id).await?;
                let ok = match creator {
                    // Stillgelegte/gelöschte Teams liefern nichts (visibility = None).
                    Some(u) if u.active && u.status == "active" && visibility(t).is_some() => {
                        visibility(t) == Some(TeamVisibility::Public)
                            || tx.teams().get_membership(t, u.id).await?.is_some()
                    }
                    _ => false,
                };
                if !ok {
                    continue;
                }
            }
            let Some(visible) = visible_teams_for_hook(&hook, &teams, &visibility) else {
                continue;
            };
            let mut hook_data = data.clone();
            let hidden: BTreeSet<String> = teams
                .iter()
                .filter(|t| !visible.contains(t))
                .map(|t| t.0.to_string())
                .collect();
            strip_hidden_teams(&mut hook_data, &hidden);
            let payload = build_payload(
                &mapped,
                actor.clone(),
                ev.created_at,
                hook_data,
                updated_from.clone(),
                &url,
                ev.org_id.0,
            );
            let inserted = tx
                .webhooks()
                .insert_delivery(&NewDelivery {
                    id: self.ids.new_uuid(),
                    webhook_id: hook.id,
                    event_id: ev.id.0,
                    resource_type: mapped.resource_type.to_string(),
                    action: mapped.action.to_string(),
                    payload,
                    now,
                })
                .await?;
            if inserted {
                planned += 1;
            }
        }
        tx.commit().await?;
        Ok(planned)
    }

    /// Beansprucht fällige Zustellungen (Lease 10 min, damit parallele Worker sie nicht doppelt senden).
    pub async fn claim_due_webhook_deliveries(
        &self,
        now: DateTime<Utc>,
        limit: i64,
    ) -> AppResult<Vec<DueDelivery>> {
        let mut tx = self.store.begin_system().await?;
        let due = tx
            .webhooks()
            .claim_due_deliveries(now, limit, Duration::minutes(10))
            .await?;
        tx.commit().await?;
        Ok(due)
    }

    /// Entschlüsselte Signatur-Secrets: aktuelles und (24 h nach Rotation) vorheriges.
    pub fn webhook_signing_secrets(
        &self,
        w: &WebhookRecord,
        now: DateTime<Utc>,
    ) -> AppResult<(String, Option<String>)> {
        let current = open_secret(&self.config.session_secret, &w.secret_enc)?;
        let previous = match (&w.previous_secret_enc, w.secret_rotated_at) {
            (Some(p), Some(at)) if now - at < Duration::hours(PREVIOUS_SECRET_GRACE_HOURS) => {
                Some(open_secret(&self.config.session_secret, p)?)
            }
            _ => None,
        };
        Ok((current, previous))
    }

    /// Protokolliert einen Versuch und plant ggf. den nächsten (Retry-Plan 1 min / 1 h / 6 h).
    /// Liefert `true`, wenn der Webhook dabei wegen Fehlschlägen deaktiviert wurde.
    pub async fn record_webhook_attempt(
        &self,
        due: &DueDelivery,
        success: bool,
        response_code: Option<i32>,
        duration_ms: i32,
        error: Option<String>,
    ) -> AppResult<bool> {
        let now = self.clock.now();
        let seed = u64::from_le_bytes(
            due.id.as_bytes()[8..16]
                .try_into()
                .map_err(|_| AppError::internal("uuid length"))?,
        ) ^ u64::try_from(due.attempt).unwrap_or(0);
        let next = if success {
            None
        } else {
            next_attempt_at(now, due.attempt.saturating_add(1), seed)
        };
        let outcome = DeliveryOutcome {
            success,
            response_code,
            duration_ms,
            error,
            next_attempt_at: next,
        };
        let mut tx = self.store.begin(due.org_id).await?;
        let disabled = tx
            .webhooks()
            .record_delivery_attempt(due.id, due.webhook.id, &outcome, now)
            .await?;
        tx.commit().await?;
        Ok(disabled)
    }
}

/// Ein Team-Webhook, dessen Team der Aufrufer nicht sehen darf, ist `not_found`;
/// `forbidden` bleibt nur für sichtbare Objekte erhalten.
fn hide_forbidden(e: AppError, w: &WebhookRecord, snap: &PolicySnapshot) -> AppError {
    match w.team_id {
        Some(t) if !snap.visible_team_ids().contains(&t) => webhook_not_found(),
        _ => e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lineal_domain::ids::IssueId;

    #[test]
    fn signature_matches_rfc4231_vector() {
        // RFC 4231, Test Case 2.
        assert_eq!(
            sign("Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn signature_is_verifiable_over_the_raw_body() {
        let payload = json!({ "action": "create", "type": "Issue", "data": { "id": "x" } });
        let body = build_body(&payload, WebhookId::new_v7(), Utc::now()).unwrap();
        let secret = generate_secret();
        assert_eq!(secret.len(), 64);
        let sig = sign(&secret, &body);
        assert_eq!(sig.len(), 64);
        assert_eq!(sig, sign(&secret, &body));
        let mut tampered = body.clone();
        tampered.push(b' ');
        assert_ne!(sig, sign(&secret, &tampered));
        assert_ne!(sig, sign("other-secret", &body));
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        assert!(parsed["webhookTimestamp"].is_i64());
        assert!(parsed["webhookId"].is_string());
    }

    #[test]
    fn secrets_roundtrip_encrypted() {
        let sealed = seal_secret("server-secret", "topsecret").unwrap();
        assert!(sealed.starts_with("v1:"));
        assert!(!sealed.contains("topsecret"));
        assert_eq!(open_secret("server-secret", &sealed).unwrap(), "topsecret");
        assert!(open_secret("other", &sealed).is_err());
        assert_ne!(sealed, seal_secret("server-secret", "topsecret").unwrap());
    }

    #[test]
    fn retry_plan_is_one_minute_one_hour_six_hours_then_gives_up() {
        assert_eq!(retry_delay(1), Some(Duration::minutes(1)));
        assert_eq!(retry_delay(2), Some(Duration::hours(1)));
        assert_eq!(retry_delay(3), Some(Duration::hours(6)));
        assert_eq!(retry_delay(4), None);
        assert_eq!(retry_delay(0), None);
        let now = Utc::now();
        for seed in 0..500u64 {
            for (n, base) in [(1, 60i64), (2, 3600), (3, 21600)] {
                let at = next_attempt_at(now, n, seed).unwrap();
                let secs = (at - now).num_milliseconds() as f64 / 1000.0;
                assert!(
                    secs >= base as f64 * 0.9 - 0.01 && secs <= base as f64 * 1.1 + 0.01,
                    "{secs} for {base}"
                );
            }
            assert!(next_attempt_at(now, 4, seed).is_none());
        }
    }

    #[test]
    fn ssrf_rejects_private_targets() {
        let s = UrlPolicy::STRICT;
        for bad in [
            "https://127.0.0.1/hook",
            "https://10.0.0.5/hook",
            "https://172.16.3.4/hook",
            "https://192.168.1.1/hook",
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]/hook",
            "https://[fe80::1]/hook",
            "https://[fd00::1]/hook",
            "https://[::ffff:127.0.0.1]/hook",
            "https://[::ffff:10.1.2.3]/hook",
            "https://100.64.0.1/hook",
            "https://0.0.0.0/hook",
            "https://localhost/hook",
            "https://db.internal/hook",
            "https://printer.local/hook",
            "https://intranet/hook",
            "http://93.184.216.34/hook",
            "ftp://93.184.216.34/hook",
            "https://user:pw@93.184.216.34/hook",
            "https://93.184.216.34/hook#frag",
            "not a url",
            "",
        ] {
            assert!(parse_target_url(bad, s).is_err(), "{bad} must be rejected");
        }
        assert!(parse_target_url("https://93.184.216.34/hook", s).is_ok());
        assert!(parse_target_url("https://example.com/hook?x=1", s).is_ok());
        assert!(parse_target_url("https://example.com:443/hook", s).is_ok());
        assert!(parse_target_url("https://example.com:8443/hook", s).is_err());
        assert!(parse_target_url("https://[::ffff:7f00:1]/hook", s).is_err());
        assert!(parse_target_url("https://[::ffff:127.0.0.1]:443/hook", s).is_err());
    }

    #[test]
    fn dev_policy_allows_http_loopback_but_still_blocks_other_private_ranges() {
        let d = UrlPolicy::DEV;
        assert!(parse_target_url("http://127.0.0.1:9000/hook", d).is_ok());
        assert!(parse_target_url("http://localhost:9000/hook", d).is_ok());
        assert!(parse_target_url("http://10.0.0.5/hook", d).is_err());
        assert!(parse_target_url("http://169.254.169.254/", d).is_err());
    }

    #[test]
    fn dns_results_on_private_ips_are_rejected() {
        let s = UrlPolicy::STRICT;
        let private: Vec<SocketAddr> = vec!["10.0.0.5:443".parse().unwrap()];
        assert!(check_resolved(&private, s).is_err());
        // Ein privater Treffer unter mehreren Adressen genügt zur Ablehnung.
        let mixed: Vec<SocketAddr> = vec![
            "93.184.216.34:443".parse().unwrap(),
            "[::1]:443".parse().unwrap(),
        ];
        assert!(check_resolved(&mixed, s).is_err());
        let metadata: Vec<SocketAddr> = vec!["169.254.169.254:80".parse().unwrap()];
        assert!(check_resolved(&metadata, s).is_err());
        assert!(check_resolved(&[], s).is_err());
        let public: Vec<SocketAddr> = vec!["93.184.216.34:443".parse().unwrap()];
        assert!(check_resolved(&public, s).is_ok());
    }

    #[tokio::test]
    async fn resolve_target_blocks_localhost_by_dns() {
        let url = Url::parse("https://localhost/hook").unwrap();
        assert!(resolve_target(&url, UrlPolicy::STRICT).await.is_err());
        let ip = Url::parse("https://127.0.0.1/hook").unwrap();
        assert!(resolve_target(&ip, UrlPolicy::STRICT).await.is_err());
        assert!(resolve_target(&ip, UrlPolicy::DEV).await.is_ok());
    }

    fn hook(team: Option<TeamId>, public: bool) -> WebhookRecord {
        WebhookRecord {
            id: WebhookId::new_v7(),
            org_id: lineal_domain::ids::OrgId::new_v7(),
            label: None,
            url: "https://example.com".into(),
            secret_enc: String::new(),
            previous_secret_enc: None,
            secret_rotated_at: None,
            enabled: true,
            team_id: team,
            all_public_teams: public,
            resource_types: vec!["Issue".into()],
            creator_id: lineal_domain::ids::UserId::new_v7(),
            oauth_application_id: None,
            consecutive_failures: 0,
            disabled_reason: None,
            last_success_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn visibility_filter_never_leaks_private_teams_to_workspace_hooks() {
        let (eng, des) = (TeamId::new_v7(), TeamId::new_v7());
        let vis = |t: TeamId| {
            Some(if t == des {
                TeamVisibility::Private
            } else {
                TeamVisibility::Public
            })
        };
        let ws = hook(None, true);
        assert_eq!(
            visible_teams_for_hook(&ws, &[eng], &vis),
            Some(BTreeSet::from([eng]))
        );
        assert_eq!(visible_teams_for_hook(&ws, &[des], &vis), None);
        // Multi-Team-Projekt: nur das öffentliche Team bleibt sichtbar.
        assert_eq!(
            visible_teams_for_hook(&ws, &[eng, des], &vis),
            Some(BTreeSet::from([eng]))
        );
        // Workspace-weite Entität (Initiative, User) geht an Workspace-Webhooks.
        assert_eq!(
            visible_teams_for_hook(&ws, &[], &vis),
            Some(BTreeSet::new())
        );
        // Team-Webhook: nur Daten seines Teams, nie workspace-weite Entitäten.
        let th = hook(Some(eng), false);
        assert_eq!(
            visible_teams_for_hook(&th, &[eng, des], &vis),
            Some(BTreeSet::from([eng]))
        );
        assert_eq!(visible_teams_for_hook(&th, &[des], &vis), None);
        assert_eq!(visible_teams_for_hook(&th, &[], &vis), None);
        // Team-Webhook auf ein privates Team erhält dessen Daten (Ersteller war Mitglied).
        let ph = hook(Some(des), false);
        assert_eq!(
            visible_teams_for_hook(&ph, &[des], &vis),
            Some(BTreeSet::from([des]))
        );
        // Gelöschtes/unbekanntes Team gilt als nicht öffentlich.
        assert_eq!(
            visible_teams_for_hook(&ws, &[TeamId::new_v7()], &|_| None),
            None
        );
    }

    #[test]
    fn hidden_teams_are_stripped_from_data() {
        let (eng, des) = (TeamId::new_v7(), TeamId::new_v7());
        let mut data = json!({
            "id": "p1",
            "teamIds": [eng.0.to_string(), des.0.to_string()],
            "teams": [{ "id": eng.0.to_string() }, { "id": des.0.to_string() }],
            "leadTeamId": des.0.to_string(),
            "leadTeam": { "id": des.0.to_string() },
        });
        strip_hidden_teams(&mut data, &BTreeSet::from([des.0.to_string()]));
        assert_eq!(data["teamIds"], json!([eng.0.to_string()]));
        assert_eq!(data["teams"], json!([{ "id": eng.0.to_string() }]));
        assert!(data["leadTeamId"].is_null() && data["leadTeam"].is_null());
    }

    #[test]
    fn events_map_to_resource_types() {
        let id = IssueId::new_v7();
        let team = TeamId::new_v7();
        let m = map_event(&DomainEvent::IssueUpdated {
            previous: None,
            issue_id: id,
            team_id: team,
            changed: vec!["stateId".into()],
            mentioned_user_ids: vec![],
        })
        .unwrap();
        assert_eq!((m.resource_type, m.action), ("Issue", "update"));
        assert_eq!(m.changed, vec!["stateId".to_string()]);
        assert_eq!(m.team_hint, Some(team));
        let s = map_event(&DomainEvent::IssueSlaBreached { issue_id: id }).unwrap();
        assert_eq!((s.resource_type, s.action), ("IssueSLA", "breached"));
        assert!(map_event(&DomainEvent::TeamUpdated { team_id: team }).is_none());
        // Jeder gemappte Typ ist abonnierbar.
        assert!(RESOURCE_TYPES.contains(&m.resource_type));
    }

    #[test]
    fn resource_types_are_validated() {
        assert!(validate_resource_types(&[]).is_err());
        assert!(validate_resource_types(&["Team".into()]).is_err());
        assert_eq!(
            validate_resource_types(&["Issue".into(), "Issue".into(), "Comment".into()]).unwrap(),
            vec!["Issue".to_string(), "Comment".to_string()]
        );
    }
}
