//! Fake-Quelle aus JSON-Fixtures (erfundene Daten) – damit Tests nie gegen das echte Linear laufen.
//!
//! Fixture-Format:
//!
//! ```json
//! { "workspace": { "id": "…", "name": "Acme", "urlKey": "acme", "viewerEmail": "alice@acme.test" },
//!   "entities": { "User": [ … ], "Team": [ … ], "Issue": [ … ] },
//!   "files": { "https://uploads.linear.app/…": { "contentType": "image/png", "hex": "89504e47…" } } }
//! ```
//!
//! Die Objekte haben das normalisierte Linear-Format (siehe `port`). Seiten werden über den
//! Listenindex als Cursor geliefert; `updatedAfter` filtert nach `updatedAt`.

use crate::net::UrlPolicy;
use crate::port::{
    Downloaded, ImportSource, PageQuery, SourceEntity, SourceError, SourcePage, SourceResult,
    SourceWorkspace,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Fake-Quelle.
pub struct FakeSource {
    workspace: SourceWorkspace,
    data: Mutex<BTreeMap<&'static str, Vec<Value>>>,
    files: HashMap<String, (String, Vec<u8>)>,
    policy: UrlPolicy,
    page_calls: Mutex<HashMap<&'static str, usize>>,
    downloads: Mutex<Vec<String>>,
    probes: Mutex<Vec<String>>,
    /// Wenn gesetzt: jede Anfrage schlägt mit diesem Fehler fehl (z. B. `Auth`).
    fail_with: Mutex<Option<SourceError>>,
    /// Entität, Anzahl erfolgreicher Seitenabrufe, danach Fehler (Absturz mitten im Lauf).
    fail_after: Mutex<Option<(&'static str, usize, SourceError)>>,
    kind: &'static str,
}

impl FakeSource {
    /// Aus einem JSON-Fixture (Format siehe Moduldoku).
    pub fn from_json(v: &Value) -> Result<Self, String> {
        let ws = &v["workspace"];
        let text = |x: &Value| x.as_str().map(str::to_owned);
        let workspace = SourceWorkspace {
            id: text(&ws["id"]).ok_or("workspace.id missing")?,
            name: text(&ws["name"]).unwrap_or_default(),
            url_key: text(&ws["urlKey"]).unwrap_or_default(),
            viewer_email: text(&ws["viewerEmail"]),
        };
        let mut data: BTreeMap<&'static str, Vec<Value>> = BTreeMap::new();
        for e in SourceEntity::ALL {
            let items = v["entities"][e.as_str()]
                .as_array()
                .cloned()
                .unwrap_or_default();
            data.insert(e.as_str(), items);
        }
        let mut files = HashMap::new();
        if let Some(m) = v["files"].as_object() {
            for (url, f) in m {
                let ct = f["contentType"]
                    .as_str()
                    .unwrap_or("application/octet-stream")
                    .to_owned();
                let bytes = match (f["hex"].as_str(), f["text"].as_str()) {
                    (Some(h), _) => hex::decode(h).map_err(|e| e.to_string())?,
                    (None, Some(t)) => t.as_bytes().to_vec(),
                    _ => Vec::new(),
                };
                files.insert(url.clone(), (ct, bytes));
            }
        }
        let mut policy = UrlPolicy::linear();
        policy.allow_local_for_tests = false;
        Ok(Self {
            workspace,
            data: Mutex::new(data),
            files,
            policy,
            page_calls: Mutex::new(HashMap::new()),
            downloads: Mutex::new(Vec::new()),
            probes: Mutex::new(Vec::new()),
            fail_with: Mutex::new(None),
            fail_after: Mutex::new(None),
            kind: "linear",
        })
    }

    /// Als CSV-Quelle ausgeben (Lauf-Quelle `linearCsv`).
    #[must_use]
    pub fn with_kind(mut self, kind: &'static str) -> Self {
        self.kind = kind;
        self
    }

    /// Alle Objekte einer Entität ersetzen.
    pub fn set_entity(&self, entity: SourceEntity, items: Vec<Value>) {
        lock(&self.data).insert(entity.as_str(), items);
    }

    /// Objekte einer Entität (Kopie).
    pub fn entity(&self, entity: SourceEntity) -> Vec<Value> {
        lock(&self.data)
            .get(entity.as_str())
            .cloned()
            .unwrap_or_default()
    }

    /// Setzt Felder eines Objekts (Delta-Tests); `patch` ist ein JSON-Objekt.
    pub fn patch(&self, entity: SourceEntity, id: &str, patch: &Value) -> bool {
        let mut data = lock(&self.data);
        let Some(list) = data.get_mut(entity.as_str()) else {
            return false;
        };
        for item in list.iter_mut() {
            if item["id"].as_str() == Some(id) {
                if let (Some(obj), Some(p)) = (item.as_object_mut(), patch.as_object()) {
                    for (k, v) in p {
                        obj.insert(k.clone(), v.clone());
                    }
                }
                return true;
            }
        }
        false
    }

    /// Lässt alle weiteren Anfragen fehlschlagen.
    pub fn fail_with(&self, err: Option<SourceError>) {
        *lock(&self.fail_with) = err;
    }

    /// Lässt Seitenabrufe der Entität nach `pages` erfolgreichen Abrufen fehlschlagen.
    pub fn fail_after(&self, entity: SourceEntity, pages: usize, err: SourceError) {
        *lock(&self.fail_after) = Some((entity.as_str(), pages, err));
    }

    /// Hebt `fail_after` auf.
    pub fn fail_after_clear(&self) {
        *lock(&self.fail_after) = None;
    }

    /// Anzahl gelesener Seiten je Entität.
    pub fn page_calls(&self, entity: SourceEntity) -> usize {
        lock(&self.page_calls)
            .get(entity.as_str())
            .copied()
            .unwrap_or(0)
    }

    /// URLs, die heruntergeladen wurden.
    pub fn downloads(&self) -> Vec<String> {
        lock(&self.downloads).clone()
    }

    /// URLs, deren Größe erfragt wurde (`probe`).
    pub fn probes(&self) -> Vec<String> {
        lock(&self.probes).clone()
    }

    /// Download-Policy für Tests ersetzen.
    #[must_use]
    pub fn with_policy(mut self, policy: UrlPolicy) -> Self {
        self.policy = policy;
        self
    }
}

#[async_trait]
impl ImportSource for FakeSource {
    fn kind(&self) -> &'static str {
        self.kind
    }

    async fn workspace(&self) -> SourceResult<SourceWorkspace> {
        if let Some(e) = lock(&self.fail_with).clone() {
            return Err(e);
        }
        Ok(self.workspace.clone())
    }

    async fn page(&self, q: &PageQuery) -> SourceResult<SourcePage> {
        if let Some(e) = lock(&self.fail_with).clone() {
            return Err(e);
        }
        if let Some((ent, pages, err)) = lock(&self.fail_after).clone()
            && ent == q.entity.as_str()
            && lock(&self.page_calls).get(ent).copied().unwrap_or(0) >= pages
        {
            return Err(err);
        }
        *lock(&self.page_calls).entry(q.entity.as_str()).or_default() += 1;
        let all = lock(&self.data)
            .get(q.entity.as_str())
            .cloned()
            .unwrap_or_default();
        let items: Vec<Value> = match q.updated_after {
            Some(after) => all
                .into_iter()
                .filter(|v| {
                    v["updatedAt"]
                        .as_str()
                        .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                        .is_none_or(|t| t > after)
                })
                .collect(),
            None => all,
        };
        let start: usize = q
            .cursor
            .as_deref()
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        let limit = q.limit.clamp(1, 100) as usize;
        let end = (start + limit).min(items.len());
        let page: Vec<Value> = items
            .get(start..end)
            .map(<[Value]>::to_vec)
            .unwrap_or_default();
        let next = (end < items.len()).then(|| end.to_string());
        Ok(SourcePage {
            items: page,
            next_cursor: next,
        })
    }

    async fn probe(&self, url: &str) -> SourceResult<Option<u64>> {
        if let Some(e) = lock(&self.fail_with).clone() {
            return Err(e);
        }
        self.policy.check_url(url)?;
        lock(&self.probes).push(url.to_owned());
        Ok(self.files.get(url).map(|(_, b)| b.len() as u64))
    }

    async fn download(&self, url: &str) -> SourceResult<Downloaded> {
        if let Some(e) = lock(&self.fail_with).clone() {
            return Err(e);
        }
        // Dieselben Regeln wie beim echten Adapter (https, Allowlist, keine IP-Literale).
        self.policy.check_url(url)?;
        lock(&self.downloads).push(url.to_owned());
        let (ct, bytes) = self
            .files
            .get(url)
            .cloned()
            .ok_or_else(|| SourceError::Transport("file not found".into()))?;
        if bytes.len() as u64 > self.policy.max_bytes {
            return Err(SourceError::Rejected("file is too large".into()));
        }
        let base = ct
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !self.policy.allowed_content_types.contains(&base) {
            return Err(SourceError::Rejected("content type is not allowed".into()));
        }
        Ok(Downloaded {
            bytes,
            content_type: base,
        })
    }
}
