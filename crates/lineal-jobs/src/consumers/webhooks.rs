//! Outbox-Konsument `webhooks` (WP16, docs/09 §4): plant je Event die Zustellungen
//! (`App::plan_webhook_deliveries`) und versendet fällige Zustellungen samt Retries.
//!
//! Versand: HMAC-SHA256-Signatur im Header `Lineal-Signature` über den rohen Body, Ziel nur nach
//! erfolgreicher SSRF-Prüfung (DNS-Auflösung, alle Adressen öffentlich), Verbindung an die geprüfte
//! Adresse gebunden, keine Redirects, 5 s Timeout, Antwort auf 16 KiB begrenzt.
//! Retries nach 1 min / 1 h / 6 h plant `App::record_webhook_attempt`; fällige Wiederholungen
//! versendet [`WebhookDeliveryJob`] (periodisch) und jeder Konsumentenlauf.

use crate::{EventConsumer, PeriodicJob, Schedule};
use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use lineal_app::ports::repos::StoredEvent;
use lineal_app::ports::repos::webhook::DueDelivery;
use lineal_app::usecases::webhook::{
    UrlPolicy, build_body, parse_target_url, resolve_target, sign,
};
use lineal_app::{App, AppResult};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use url::Url;

/// Timeout eines Zustellversuchs (docs/09 §4.5).
pub const DELIVERY_TIMEOUT: Duration = Duration::from_secs(5);
/// Obergrenze der gelesenen Antwort.
pub const MAX_RESPONSE_BYTES: usize = 16 * 1024;
/// Zustellungen je Durchlauf.
const BATCH: i64 = 25;
/// Aufbewahrung des Zustellprotokolls.
const LOG_RETENTION_DAYS: i64 = 30;

/// Ein Zustellversuch (nach der SSRF-Prüfung).
#[derive(Debug, Clone)]
pub struct WebhookRequest {
    pub url: Url,
    /// Geprüfte Adressen; die Verbindung wird daran gebunden.
    pub addrs: Vec<SocketAddr>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Ergebnis eines Versuchs.
#[derive(Debug, Clone)]
pub struct TransportResult {
    pub status: Option<u16>,
    pub error: Option<String>,
}

/// HTTP-Transport (austauschbar für Tests).
#[async_trait]
pub trait WebhookTransport: Send + Sync {
    async fn send(&self, req: WebhookRequest) -> TransportResult;
}

/// Produktiver Transport (reqwest + rustls).
pub struct ReqwestTransport;

#[async_trait]
impl WebhookTransport for ReqwestTransport {
    async fn send(&self, req: WebhookRequest) -> TransportResult {
        let fail = |e: String| TransportResult {
            status: None,
            error: Some(e),
        };
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(DELIVERY_TIMEOUT)
            .connect_timeout(Duration::from_secs(3))
            .user_agent("Lineal-Webhook")
            .no_proxy();
        if let Some(host) = req.url.host_str()
            && !host.starts_with('[')
            && host.parse::<std::net::IpAddr>().is_err()
        {
            builder = builder.resolve_to_addrs(host, &req.addrs);
        }
        let client = match builder.build() {
            Ok(c) => c,
            Err(e) => return fail(format!("client: {e}")),
        };
        let mut request = client.post(req.url.clone()).body(req.body);
        for (k, v) in &req.headers {
            request = request.header(k, v);
        }
        let mut response = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                return fail(if e.is_timeout() {
                    "timeout".into()
                } else if e.is_connect() {
                    "connection failed".into()
                } else {
                    "request failed".into()
                });
            }
        };
        let status = response.status().as_u16();
        // Antwort begrenzt lesen (Verbindung sauber beenden), Inhalt wird verworfen.
        let mut read = 0usize;
        while read < MAX_RESPONSE_BYTES {
            match response.chunk().await {
                Ok(Some(c)) => read += c.len(),
                _ => break,
            }
        }
        TransportResult {
            status: Some(status),
            error: None,
        }
    }
}

/// Versendet fällige Zustellungen. Liefert die Anzahl der Versuche.
pub async fn deliver_due(
    app: &App,
    transport: &dyn WebhookTransport,
    now: DateTime<Utc>,
    limit: i64,
) -> AppResult<usize> {
    let due = app.claim_due_webhook_deliveries(now, limit).await?;
    let policy = UrlPolicy::for_app(app);
    let n = due.len();
    for d in due {
        if let Err(e) = deliver_one(app, transport, policy, &d).await {
            tracing::warn!(delivery = %d.id, error = %e, "webhook delivery bookkeeping failed");
        }
    }
    Ok(n)
}

async fn deliver_one(
    app: &App,
    transport: &dyn WebhookTransport,
    policy: UrlPolicy,
    d: &DueDelivery,
) -> AppResult<()> {
    let started = Instant::now();
    let sent_at = app.clock.now();
    let outcome = attempt(app, transport, policy, d, sent_at).await;
    let ms = i32::try_from(started.elapsed().as_millis()).unwrap_or(i32::MAX);
    let (success, code, error) = match outcome {
        Ok(TransportResult {
            status: Some(s),
            error: None,
        }) => {
            let ok = (200..300).contains(&s);
            (ok, Some(i32::from(s)), (!ok).then(|| format!("HTTP {s}")))
        }
        Ok(TransportResult { error, .. }) => (false, None, error),
        Err(msg) => (false, None, Some(msg)),
    };
    let disabled = app
        .record_webhook_attempt(d, success, code, ms, error)
        .await?;
    if disabled {
        tracing::warn!(webhook = %d.webhook.id, "webhook disabled after repeated delivery failures");
    }
    Ok(())
}

async fn attempt(
    app: &App,
    transport: &dyn WebhookTransport,
    policy: UrlPolicy,
    d: &DueDelivery,
    now: DateTime<Utc>,
) -> Result<TransportResult, String> {
    // SSRF-Prüfung bei jeder Zustellung, nach der DNS-Auflösung.
    let url = parse_target_url(&d.webhook.url, policy).map_err(|e| e.message)?;
    let addrs = resolve_target(&url, policy).await.map_err(|e| e.message)?;
    let body = build_body(&d.payload, d.webhook.id, now).map_err(|e| e.message)?;
    let (secret, previous) = app
        .webhook_signing_secrets(&d.webhook, now)
        .map_err(|e| e.message)?;
    let mut headers = vec![
        (
            "Content-Type".to_string(),
            "application/json; charset=utf-8".to_string(),
        ),
        ("Lineal-Delivery".to_string(), d.id.to_string()),
        ("Lineal-Event".to_string(), d.resource_type.clone()),
        (
            "Lineal-Timestamp".to_string(),
            now.timestamp_millis().to_string(),
        ),
        ("Lineal-Signature".to_string(), sign(&secret, &body)),
    ];
    if let Some(prev) = previous {
        headers.push(("Lineal-Signature-Previous".to_string(), sign(&prev, &body)));
    }
    Ok(transport
        .send(WebhookRequest {
            url,
            addrs,
            headers,
            body,
        })
        .await)
}

pub struct WebhooksConsumer;

#[async_trait]
impl EventConsumer for WebhooksConsumer {
    fn name(&self) -> &'static str {
        "webhooks"
    }

    async fn handle(&self, app: &App, ev: &StoredEvent) -> AppResult<()> {
        // Planen ist idempotent (eindeutig je Webhook × Event); Fehler lassen das Event liegen.
        app.plan_webhook_deliveries(ev).await?;
        // Versandfehler sind Sache des Retry-Plans, nicht des Events.
        if let Err(e) = deliver_due(app, &ReqwestTransport, app.clock.now(), BATCH).await {
            tracing::warn!(error = %e, "webhook delivery run failed");
        }
        Ok(())
    }
}

/// Periodischer Job: fällige Retries versenden und das Zustellprotokoll nach 30 Tagen löschen.
pub struct WebhookDeliveryJob;

#[async_trait]
impl PeriodicJob for WebhookDeliveryJob {
    fn name(&self) -> &'static str {
        "webhook_delivery"
    }

    fn schedule(&self) -> Schedule {
        Schedule::Every(Duration::from_secs(30))
    }

    async fn run(&self, app: &App, now: DateTime<Utc>) -> AppResult<()> {
        deliver_due(app, &ReqwestTransport, now, BATCH * 4).await?;
        let mut tx = app.store.begin_system().await?;
        tx.webhooks()
            .purge_deliveries(now - ChronoDuration::days(LOG_RETENTION_DAYS))
            .await?;
        tx.commit().await
    }
}
