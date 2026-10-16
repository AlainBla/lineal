//! Phasen 5–7: Kommentare/Reaktionen, Anhänge und eingebettete Dateien, Dokumente, Status-Updates.

use super::node::{N, clean_text, clean_title, jts, row};
use super::{HResult, ImportEngine, MAX_COMMENT, MAX_DOCUMENT, Outcome, Session, Step};
use crate::port::SourceEntity;
use futures_util::StreamExt;
use lineal_app::error::{AppError, AppResult};
use lineal_app::ports::repos::import::ImportTable;
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{
    AttachmentId, CommentId, DocumentId, InitiativeId, InitiativeUpdateId, OrgId, ProjectId,
    ProjectUpdateId,
};
use lineal_domain::rules::markdown::is_safe_url;
use regex::Regex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::LazyLock;
use uuid::Uuid;

const HEALTH: &[&str] = &["onTrack", "atRisk", "offTrack"];
const UPLOAD_HOST: &str = "uploads.linear.app";

static UPLOAD_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"https://uploads\.linear\.app/[^\s)\]">'<]+"#).expect("static regex is valid")
});

/// Eine in den Blob-Store übernommene Datei.
#[derive(Debug, Clone)]
pub(crate) struct Stored {
    pub key: String,
    pub size: u64,
    pub content_type: String,
    pub filename: String,
}

/// Nur `https://uploads.linear.app/…` (exakter Host) gilt als herunterladbarer Upload.
fn is_upload_url(url: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .is_some_and(|u| u.scheme() == "https" && u.host_str() == Some(UPLOAD_HOST))
}

fn filename_of(url: &str) -> String {
    let last = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .rsplit('/')
        .next()
        .unwrap_or("file");
    let clean: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    if clean.is_empty() {
        "file".into()
    } else {
        clean
    }
}

/// Lädt eine Datei herunter (Policy der Quelle) und legt sie im Blob-Store ab.
async fn store_file(eng: &ImportEngine<'_>, org: OrgId, url: &str) -> Result<Stored, String> {
    let d = eng.source.download(url).await.map_err(|e| e.to_string())?;
    let key = format!(
        "{}/{}/{}",
        org.0,
        eng.app.ids.new_uuid(),
        eng.app.ids.new_uuid()
    );
    let size = d.bytes.len() as u64;
    eng.sink
        .put(&key, &d.content_type, d.bytes)
        .await
        .map_err(|e| e.message)?;
    Ok(Stored {
        key,
        size,
        content_type: d.content_type,
        filename: filename_of(url),
    })
}

impl Session<'_, '_> {
    // --------------------------------------------------------------------------------------
    // Kommentare
    // --------------------------------------------------------------------------------------

    pub(crate) async fn step_comments(&mut self, cursor: Option<String>) -> HResult<()> {
        self.comments_step(Step::Comments, cursor).await
    }

    /// Kommentare an Projekt-Updates: erst nach den Updates (Phase 7), deshalb ein eigener Schritt.
    pub(crate) async fn step_update_comments(&mut self, cursor: Option<String>) -> HResult<()> {
        self.comments_step(Step::UpdateComments, cursor).await
    }

    async fn comments_step(&mut self, step: Step, cursor: Option<String>) -> HResult<()> {
        let on_updates = step == Step::UpdateComments;
        let mut cursor = cursor;
        loop {
            let page = self.fetch(SourceEntity::Comments, cursor).await?;
            self.begin_unit().await?;
            for v in &page.items {
                let n = N(v);
                // Jeder Kommentar gehört genau einem der beiden Schritte.
                let for_update = n.ref_id("projectUpdate").is_some();
                if for_update != on_updates {
                    continue;
                }
                let r = self.comment(n, on_updates).await;
                self.item_result("Comment", n.id(), r)?;
            }
            let next = page.next_cursor;
            if next.is_none() {
                self.resolve_deferred_parents().await?;
            }
            self.end_unit(Some(self.cp(step, next.clone()))).await?;
            match next {
                Some(c) => cursor = Some(c),
                None => return Ok(()),
            }
        }
    }

    /// Merkt ein Paar (Antwort, Elternkommentar) vor; steht im Lauf-Status und überlebt Neustarts.
    fn defer_parent(&mut self, child: &str, parent: &str) {
        if !self.run.stats.is_object() {
            self.run.stats = json!({});
        }
        if let Some(obj) = self.run.stats.as_object_mut() {
            let list = obj.entry("deferredParents").or_insert_with(|| json!([]));
            if let Some(a) = list.as_array_mut() {
                a.push(json!([child, parent]));
            }
        }
    }

    /// Trägt vorgemerkte Elternbezüge nach, sobald alle Kommentare gelesen sind. Bleibt der Eltern-
    /// kommentar auch dann unbekannt (gelöscht, anderes Team/Projekt, nicht ausgewählt), bleibt die
    /// Antwort ein Kommentar auf oberster Ebene; der Bericht nennt die Ursache.
    async fn resolve_deferred_parents(&mut self) -> AppResult<()> {
        let pairs: Vec<(String, String)> = self.run.stats["deferredParents"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| Some((p[0].as_str()?.to_owned(), p[1].as_str()?.to_owned())))
            .collect();
        if let Some(obj) = self.run.stats.as_object_mut() {
            obj.remove("deferredParents");
        }
        let mut fixed = 0u64;
        for (child, parent) in pairs {
            let (Some(c), Some(p)) = (
                self.target("Comment", &child).await?,
                self.target("Comment", &parent).await?,
            ) else {
                self.note(
                    "warning",
                    "Comment",
                    Some(&child),
                    "the parent comment is not part of the import (deleted in Linear, on another object, or outside the selected teams); imported as a top-level comment",
                );
                continue;
            };
            let r = row(vec![("parent_id", json!(p)), ("quoted_text", Value::Null)]);
            self.tx()?
                .imports()
                .update_row(ImportTable::Comment, c, &Value::Object(r))
                .await?;
            fixed += 1;
        }
        if fixed > 0 {
            self.note(
                "info",
                "Comment",
                None,
                format!("{fixed} reply comment(s) were attached to their parent comment after all comments were read"),
            );
        }
        Ok(())
    }

    async fn comment(&mut self, n: N<'_>, on_update: bool) -> AppResult<()> {
        let src = n
            .id()
            .ok_or_else(|| AppError::invalid_request("comment without id"))?
            .to_owned();
        if n.ref_id("initiativeUpdate").is_some() {
            self.counters("Comment").skipped += 1;
            self.note(
                "info",
                "Comment",
                Some(&src),
                "comments on initiative updates are not imported",
            );
            return Ok(());
        }
        let (issue, update) = if on_update {
            (
                None,
                self.target_opt("ProjectUpdate", n.ref_id("projectUpdate"))
                    .await?,
            )
        } else {
            (self.target_opt("Issue", n.ref_id("issue")).await?, None)
        };
        if issue.is_none() && update.is_none() {
            self.counters("Comment").skipped += 1;
            if on_update {
                self.note(
                    "info",
                    "Comment",
                    Some(&src),
                    "the project update of the comment was not imported; skipped",
                );
            }
            return Ok(());
        }
        // Linear liefert Antworten oft vor ihrem Elternkommentar (neueste zuerst, Seitengrenzen).
        // Ein noch nicht importierter Elternkommentar wird vorgemerkt und am Ende des Schritts
        // nachgetragen (`resolve_deferred_parents`).
        let mut deferred_parent = None;
        let parent = match n.ref_id("parent") {
            Some(p) => {
                let t = self.target("Comment", p).await?;
                if t.is_none() {
                    deferred_parent = Some(p.to_owned());
                }
                t
            }
            None => None,
        };
        let now = self.now;
        let created = n.ts("createdAt").unwrap_or(now).min(now);
        let body = self
            .md("Comment", &src, n.str("body"), MAX_COMMENT)
            .filter(|b| !b.trim().is_empty())
            .unwrap_or_else(|| "(empty)".to_owned());
        let user = self.target_opt("User", n.ref_id("user")).await?;
        let top = parent.is_none() && deferred_parent.is_none();
        let r = row(vec![
            ("body", json!(body)),
            ("user_id", json!(user)),
            ("issue_id", json!(issue)),
            ("project_update_id", json!(update)),
            ("parent_id", json!(parent)),
            (
                "quoted_text",
                json!(
                    top.then(|| n.text("quotedText").map(|q| clean_text(q, 5000, true)))
                        .flatten()
                ),
            ),
            ("edited_at", jts(n.ts("editedAt"))),
            (
                "resolved_at",
                jts(top.then(|| n.ts("resolvedAt")).flatten()),
            ),
            ("created_at", jts(Some(created))),
            (
                "updated_at",
                jts(n.ts("updatedAt").map(|t| t.min(now)).or(Some(created))),
            ),
        ]);
        let up = self
            .upsert("Comment", n, ImportTable::Comment, r, false)
            .await?;
        if let Some(p) = deferred_parent {
            self.defer_parent(&src, &p);
        }
        if !up.wrote() {
            return Ok(());
        }
        if up.outcome == Outcome::Updated {
            self.tx()?
                .imports()
                .delete_rows_where(ImportTable::Reaction, "comment_id", up.id)
                .await?;
        }
        self.reactions(n, "comment_id", up.id, created).await?;
        let ev = if up.outcome == Outcome::Created {
            DomainEvent::CommentCreated {
                comment_id: CommentId(up.id),
                mentioned_user_ids: vec![],
            }
        } else {
            DomainEvent::CommentUpdated {
                comment_id: CommentId(up.id),
                mentioned_user_ids: vec![],
            }
        };
        self.event(ev).await?;
        Ok(())
    }

    /// Reaktionen eines Quellobjekts (`reactions: [{ emoji, user, createdAt }]`) an das Ziel
    /// (`column` = `issue_id`/`comment_id`) schreiben. Reaktionen unbekannter Nutzer entfallen;
    /// doppelte (Nutzer, Emoji) schützt der Unique-Index.
    pub(crate) async fn reactions(
        &mut self,
        n: N<'_>,
        column: &'static str,
        target: Uuid,
        fallback: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<()> {
        let now = self.now;
        for r in n.array("reactions").iter().take(500) {
            let rn = N(r);
            let Some(emoji) = rn
                .text("emoji")
                .map(|e| clean_text(e, 64, false))
                .filter(|e| !e.is_empty())
            else {
                continue;
            };
            let Some(uid) = self.target_opt("User", rn.ref_id("user")).await? else {
                continue;
            };
            let rc = rn.ts("createdAt").unwrap_or(fallback).min(now);
            let rrow = row(vec![
                ("emoji", json!(emoji)),
                ("user_id", json!(uid)),
                (column, json!(target)),
                ("created_at", jts(Some(rc))),
                ("updated_at", jts(Some(rc))),
            ]);
            if self
                .tx()?
                .imports()
                .insert_row_ignore(ImportTable::Reaction, &Value::Object(rrow))
                .await?
            {
                self.counters("Reaction").created += 1;
            }
        }
        Ok(())
    }

    // --------------------------------------------------------------------------------------
    // Anhänge (Phase 6)
    // --------------------------------------------------------------------------------------

    /// Lädt mehrere Dateien mit begrenzter Parallelität (docs/13 §4.1: 4 gleichzeitige Downloads)
    /// und legt sie im Blob-Store ab. Borgt nur die (synchronisierte) Engine, nicht die Sitzung.
    async fn store_files(
        eng: &ImportEngine<'_>,
        org: OrgId,
        urls: Vec<String>,
    ) -> HashMap<String, Result<Stored, String>> {
        let conc = eng.config.download_concurrency.clamp(1, 4);
        futures_util::stream::iter(urls)
            .map(|u| async move {
                let r = store_file(eng, org, &u).await;
                (u, r)
            })
            .buffer_unordered(conc)
            .collect()
            .await
    }

    /// Dry-Run: zählt die geplanten Downloads (`plan.downloadFiles`, `downloadBytes`,
    /// `downloadSizeUnknown`, `downloadRejected`), ohne den Inhalt zu laden. Jede URL zählt einmal.
    async fn probe_files(&mut self, urls: Vec<String>) {
        let fresh: Vec<String> = urls
            .into_iter()
            .filter(|u| self.dl_seen.insert(u.clone()))
            .collect();
        let eng = self.eng;
        let results: Vec<(String, crate::port::SourceResult<Option<u64>>)> =
            futures_util::stream::iter(fresh)
                .map(|u| async move {
                    let r = eng.source.probe(&u).await;
                    (u, r)
                })
                .buffer_unordered(4)
                .collect()
                .await;
        for (url, r) in results {
            match r {
                Ok(size) => {
                    self.plan_add_n("downloadFiles", 1);
                    match size {
                        Some(b) => self.plan_add_n("downloadBytes", b),
                        None => self.plan_add_n("downloadSizeUnknown", 1),
                    }
                }
                Err(e) => {
                    self.plan_add_n("downloadRejected", 1);
                    let host = url::Url::parse(&url)
                        .ok()
                        .and_then(|u| u.host_str().map(str::to_owned))
                        .unwrap_or_default();
                    self.note(
                        "warning",
                        "Attachment",
                        None,
                        format!("a file on {host} would not be downloaded ({e})"),
                    );
                }
            }
        }
    }

    pub(crate) async fn step_attachments(&mut self, cursor: Option<String>) -> HResult<()> {
        let mut cursor = cursor;
        loop {
            let page = self.fetch(SourceEntity::Attachments, cursor).await?;
            // Downloads außerhalb der Transaktion; im Dry-Run nie.
            let urls: Vec<String> = page
                .items
                .iter()
                .filter_map(|v| N(v).text("url"))
                .filter(|u| is_upload_url(u))
                .map(str::to_owned)
                .collect();
            let stored = if self.dry {
                // Dry-Run: nur zählen (Anzahl, Größe per HEAD), nichts laden.
                self.probe_files(urls).await;
                HashMap::new()
            } else {
                Self::store_files(self.eng, self.org, urls).await
            };
            self.begin_unit().await?;
            for v in &page.items {
                let n = N(v);
                let r = self.attachment(n, &stored).await;
                self.item_result("Attachment", n.id(), r)?;
            }
            let next = page.next_cursor;
            self.end_unit(Some(self.cp(Step::Attachments, next.clone())))
                .await?;
            match next {
                Some(c) => cursor = Some(c),
                None => return Ok(()),
            }
        }
    }

    async fn attachment(
        &mut self,
        n: N<'_>,
        stored: &HashMap<String, Result<Stored, String>>,
    ) -> AppResult<()> {
        let src = n
            .id()
            .ok_or_else(|| AppError::invalid_request("attachment without id"))?
            .to_owned();
        let Some(issue) = self.target_opt("Issue", n.ref_id("issue")).await? else {
            self.counters("Attachment").skipped += 1;
            return Ok(());
        };
        let Some(url) = n.text("url").filter(|u| u.len() <= 2048) else {
            return Err(AppError::invalid_request("attachment without a usable url"));
        };
        if url.starts_with(&self.eng.app.asset_prefix()) {
            self.counters("Attachment").skipped += 1;
            self.note(
                "warning",
                "Attachment",
                Some(&src),
                "the attachment points to a Lineal asset path; skipped",
            );
            return Ok(());
        }
        if !is_safe_url(url) {
            self.counters("Attachment").skipped += 1;
            self.note(
                "warning",
                "Attachment",
                Some(&src),
                "the attachment URL scheme is not allowed; skipped",
            );
            return Ok(());
        }
        let now = self.now;
        let created = n.ts("createdAt").unwrap_or(now).min(now);
        let creator = self.target_opt("User", n.ref_id("creator")).await?;
        let title = clean_title(n.str("title"), 255, &clean_text(url, 255, false));
        let metadata =
            n.0.get("metadata")
                .filter(|m| m.is_object() && m.to_string().len() <= 10_000)
                .cloned()
                .unwrap_or_else(|| json!({}));
        let mut r = row(vec![
            ("issue_id", json!(issue)),
            ("url", json!(url)),
            ("title", json!(title)),
            (
                "subtitle",
                json!(n.text("subtitle").map(|s| clean_text(s, 500, false))),
            ),
            (
                "source_type",
                json!(n.text("sourceType").map(|s| clean_text(s, 64, false))),
            ),
            ("metadata", metadata),
            ("creator_id", json!(creator)),
            ("created_at", jts(Some(created))),
            (
                "updated_at",
                jts(n.ts("updatedAt").map(|t| t.min(now)).or(Some(created))),
            ),
        ]);
        if is_upload_url(url) {
            match stored.get(url) {
                Some(Ok(s)) => {
                    r.insert(
                        "url".into(),
                        json!(format!("{}{}", self.eng.app.asset_prefix(), s.key)),
                    );
                    r.insert("storage_key".into(), json!(s.key));
                    r.insert("content_type".into(), json!(s.content_type));
                    r.insert("size".into(), json!(s.size));
                    r.insert("source_type".into(), Value::Null);
                    r.insert(
                        "title".into(),
                        json!(clean_title(n.str("title"), 255, &s.filename)),
                    );
                }
                Some(Err(msg)) => {
                    self.note(
                        "warning",
                        "Attachment",
                        Some(&src),
                        format!("file could not be downloaded; kept as a link ({msg})"),
                    );
                }
                None => {
                    if self.dry {
                        self.note(
                            "info",
                            "Attachment",
                            Some(&src),
                            "the file would be downloaded (not done in a dry run)",
                        );
                    }
                }
            }
        }
        let up = self
            .upsert("Attachment", n, ImportTable::Attachment, r, false)
            .await?;
        if up.outcome == Outcome::Created {
            self.event(DomainEvent::AttachmentCreated {
                attachment_id: AttachmentId(up.id),
            })
            .await?;
        }
        Ok(())
    }

    /// Eingebettete Uploads in Issue-Beschreibungen und Kommentaren: herunterladen, im Blob-Store
    /// ablegen, URL umschreiben, als Anhang des Issues führen (damit die Datei lesbar bleibt).
    pub(crate) async fn step_embedded_files(&mut self, cursor: Option<String>) -> HResult<()> {
        if self.dry {
            return self.count_embedded_files().await;
        }
        // Cursor: `<tabelle>:<uuid>`; Tabelle 0 = Issues, 1 = Kommentare.
        let (mut stage, mut after): (usize, Option<Uuid>) =
            match cursor.as_deref().and_then(|c| c.split_once(':')) {
                Some((s, id)) => (s.parse().unwrap_or(0), id.parse().ok()),
                None => (0, None),
            };
        let targets = [
            (ImportTable::Issue, "description", "id", "Issue"),
            (ImportTable::Comment, "body", "issue_id", "Comment"),
        ];
        while stage < targets.len() {
            let (table, col, owner_col, ty) = targets[stage];
            loop {
                // Lesen in einer kurzen eigenen Transaktion, Downloads ohne offene Transaktion.
                let mut rtx = self.eng.app.store.begin(self.org).await?;
                let rows = rtx
                    .imports()
                    .find_rows_containing(table, col, owner_col, UPLOAD_HOST, after, 25)
                    .await?;
                drop(rtx);
                if rows.is_empty() {
                    break;
                }
                let mut urls: Vec<String> = rows
                    .iter()
                    .flat_map(|(_, _, body)| {
                        UPLOAD_URL.find_iter(body).map(|m| m.as_str().to_owned())
                    })
                    .filter(|u| is_upload_url(u))
                    .collect();
                urls.sort();
                urls.dedup();
                let stored = Self::store_files(self.eng, self.org, urls).await;
                self.begin_unit().await?;
                let now = self.now;
                for (id, owner, body) in &rows {
                    let mut text = body.clone();
                    for (url, res) in &stored {
                        if !body.contains(url.as_str()) {
                            continue;
                        }
                        match res {
                            Ok(s) => {
                                let asset = format!("{}{}", self.eng.app.asset_prefix(), s.key);
                                text = text.replace(url.as_str(), &asset);
                                let att_id = self.new_id();
                                let r = row(vec![
                                    ("id", json!(att_id)),
                                    ("issue_id", json!(owner)),
                                    ("url", json!(asset)),
                                    ("title", json!(clean_title(Some(&s.filename), 255, "file"))),
                                    ("content_type", json!(s.content_type)),
                                    ("size", json!(s.size)),
                                    ("storage_key", json!(s.key)),
                                    ("created_at", jts(Some(now))),
                                    ("updated_at", jts(Some(now))),
                                ]);
                                if self
                                    .tx()?
                                    .imports()
                                    .insert_row_ignore(ImportTable::Attachment, &Value::Object(r))
                                    .await?
                                {
                                    self.event(DomainEvent::AttachmentCreated {
                                        attachment_id: AttachmentId(att_id),
                                    })
                                    .await?;
                                }
                                self.counters("EmbeddedFile").created += 1;
                            }
                            Err(msg) => {
                                self.counters("EmbeddedFile").errors += 1;
                                self.note(
                                    "warning",
                                    ty,
                                    Some(&id.to_string()),
                                    format!("embedded file could not be downloaded ({msg})"),
                                );
                            }
                        }
                    }
                    if text != *body {
                        self.tx()?
                            .imports()
                            .set_row_text(table, col, *id, &text)
                            .await?;
                    }
                }
                after = rows.last().map(|(id, _, _)| *id);
                let cp = json!({"phase": 6, "step": "EmbeddedFiles", "cursor": format!("{stage}:{}", after.map(|a| a.to_string()).unwrap_or_default())});
                self.end_unit(Some(cp)).await?;
            }
            stage += 1;
            after = None;
        }
        Ok(())
    }

    /// Dry-Run von Phase 6: sucht eingebettete Uploads in den (in der Dry-Run-Transaktion
    /// geschriebenen) Issue-Beschreibungen und Kommentaren und zählt sie.
    async fn count_embedded_files(&mut self) -> HResult<()> {
        self.begin_unit().await?;
        for (table, col, owner_col) in [
            (ImportTable::Issue, "description", "id"),
            (ImportTable::Comment, "body", "issue_id"),
        ] {
            let mut after: Option<Uuid> = None;
            loop {
                let rows = self
                    .tx()?
                    .imports()
                    .find_rows_containing(table, col, owner_col, UPLOAD_HOST, after, 100)
                    .await?;
                if rows.is_empty() {
                    break;
                }
                let urls: Vec<String> = rows
                    .iter()
                    .flat_map(|(_, _, body)| {
                        UPLOAD_URL.find_iter(body).map(|m| m.as_str().to_owned())
                    })
                    .filter(|u| is_upload_url(u))
                    .collect();
                self.probe_files(urls).await;
                after = rows.last().map(|(id, _, _)| *id);
            }
        }
        Ok(())
    }

    // --------------------------------------------------------------------------------------
    // Dokumente, Status-Updates (Phase 7)
    // --------------------------------------------------------------------------------------

    pub(crate) async fn step_documents(&mut self, cursor: Option<String>) -> HResult<()> {
        let mut cursor = cursor;
        loop {
            let page = self.fetch(SourceEntity::Documents, cursor).await?;
            self.begin_unit().await?;
            for v in &page.items {
                let n = N(v);
                let r = self.document(n).await;
                self.item_result("Document", n.id(), r)?;
            }
            let next = page.next_cursor;
            self.end_unit(Some(self.cp(Step::Documents, next.clone())))
                .await?;
            match next {
                Some(c) => cursor = Some(c),
                None => return Ok(()),
            }
        }
    }

    async fn document(&mut self, n: N<'_>) -> AppResult<()> {
        let src = n
            .id()
            .ok_or_else(|| AppError::invalid_request("document without id"))?
            .to_owned();
        let project = self.target_opt("Project", n.ref_id("project")).await?;
        let initiative = self
            .target_opt("Initiative", n.ref_id("initiative"))
            .await?;
        let issue = self.target_opt("Issue", n.ref_id("issue")).await?;
        let (project, initiative, issue, team) = {
            let found = [project, initiative, issue].iter().flatten().count();
            if found == 1 {
                (project, initiative, issue, None)
            } else if found > 1 {
                // Mehrere Eltern gibt es im Modell nicht: Projekt vor Issue vor Initiative.
                self.note(
                    "warning",
                    "Document",
                    Some(&src),
                    "the document has several parents; only one is kept (project, issue, initiative)",
                );
                if project.is_some() {
                    (project, None, None, None)
                } else if issue.is_some() {
                    (None, None, issue, None)
                } else {
                    (None, initiative, None, None)
                }
            } else {
                // Kein importiertes Elternobjekt. Sichtbarkeit darf nie wachsen: Ein Dokument, das in
                // Linear an einem (nicht importierten, womöglich privaten) Projekt, Issue, Cycle oder
                // einer Initiative hing, bleibt draußen. Nur ein Dokument, das in Linear selbst ein
                // Team-Dokument ist (kein anderes Elternobjekt), geht an genau dieses Team, sofern
                // es importiert wurde (gleiche Sichtbarkeit wie in Linear), nie an den Workspace
                // oder ein beliebiges anderes Team.
                let had_parent = ["project", "initiative", "issue", "cycle"]
                    .iter()
                    .any(|k| n.ref_id(k).is_some());
                let own_team = if had_parent {
                    None
                } else {
                    self.document_team(n).await?
                };
                match own_team {
                    Some(t) => {
                        self.note(
                            "info",
                            "Document",
                            Some(&src),
                            "the document has no parent object; attached to its own imported team",
                        );
                        (None, None, None, Some(t))
                    }
                    None => {
                        self.counters("Document").skipped += 1;
                        self.note(
                            "warning",
                            "Document",
                            Some(&src),
                            if had_parent {
                                "the parent of the document (project, issue, cycle or initiative) was not imported; the document was skipped so that it does not become visible to more people than in Linear"
                            } else {
                                "the document has no parent object and its team was not imported; skipped"
                            },
                        );
                        return Ok(());
                    }
                }
            }
        };
        let now = self.now;
        let created = n.ts("createdAt").unwrap_or(now).min(now);
        let updated = n.ts("updatedAt").map(|t| t.min(now)).unwrap_or(created);
        let creator = self.target_opt("User", n.ref_id("creator")).await?;
        let content = self
            .md("Document", &src, n.str("content"), MAX_DOCUMENT)
            .unwrap_or_default();
        let mut r = row(vec![
            (
                "title",
                json!(clean_text(n.str("title").unwrap_or(""), 255, false)),
            ),
            ("content", json!(content)),
            ("icon", json!(super::node::icon_or_none(n.str("icon")))),
            (
                "color",
                json!(
                    n.str("color")
                        .map(|c| super::node::color_or(Some(c), "#5E6AD2"))
                ),
            ),
            ("project_id", json!(project)),
            ("initiative_id", json!(initiative)),
            ("issue_id", json!(issue)),
            ("team_id", json!(team)),
            ("creator_id", json!(creator)),
            ("updated_by_id", json!(creator)),
            ("archived_at", jts(n.ts("archivedAt"))),
            ("created_at", jts(Some(created))),
            ("updated_at", jts(Some(updated))),
        ]);
        if let Some(slug) = n
            .str("slugId")
            .map(str::to_ascii_lowercase)
            .filter(|s| s.len() == 12 && s.chars().all(|c| c.is_ascii_hexdigit()))
        {
            r.insert("slug_id".into(), json!(slug));
        }
        let up = self
            .upsert("Document", n, ImportTable::Document, r, false)
            .await?;
        if !up.wrote() {
            return Ok(());
        }
        if up.outcome == Outcome::Updated {
            self.tx()?
                .imports()
                .delete_rows_where(ImportTable::DocumentContent, "owner_id", up.id)
                .await?;
        }
        let cid = self.new_id();
        let c = row(vec![
            ("id", json!(cid)),
            ("owner_type", json!("document")),
            ("owner_id", json!(up.id)),
            ("markdown", json!(content)),
            ("created_at", jts(Some(created))),
            ("updated_at", jts(Some(updated))),
        ]);
        self.tx()?
            .imports()
            .insert_row(ImportTable::DocumentContent, &Value::Object(c))
            .await?;
        if up.outcome == Outcome::Created {
            self.event(DomainEvent::DocumentCreated {
                document_id: DocumentId(up.id),
            })
            .await?;
        } else {
            self.event(DomainEvent::DocumentUpdated {
                document_id: DocumentId(up.id),
            })
            .await?;
        }
        Ok(())
    }

    /// Das eigene, importierte Team eines Dokuments (nur wenn es in der Auswahl liegt).
    async fn document_team(&mut self, n: N<'_>) -> AppResult<Option<Uuid>> {
        match n.ref_id("team").filter(|t| self.selected.contains(*t)) {
            Some(t) => {
                let t = t.to_owned();
                self.target("Team", &t).await
            }
            None => Ok(None),
        }
    }

    pub(crate) async fn step_project_updates(&mut self, cursor: Option<String>) -> HResult<()> {
        self.updates_step(
            SourceEntity::ProjectUpdates,
            Step::ProjectUpdates,
            cursor,
            "ProjectUpdate",
        )
        .await
    }

    pub(crate) async fn step_initiative_updates(&mut self, cursor: Option<String>) -> HResult<()> {
        self.updates_step(
            SourceEntity::InitiativeUpdates,
            Step::InitiativeUpdates,
            cursor,
            "InitiativeUpdate",
        )
        .await
    }

    async fn updates_step(
        &mut self,
        entity: SourceEntity,
        step: Step,
        cursor: Option<String>,
        ty: &'static str,
    ) -> HResult<()> {
        let mut cursor = cursor;
        loop {
            let page = self.fetch(entity, cursor).await?;
            self.begin_unit().await?;
            for v in &page.items {
                let n = N(v);
                let r = self.status_update(n, ty).await;
                self.item_result(ty, n.id(), r)?;
            }
            let next = page.next_cursor;
            self.end_unit(Some(self.cp(step, next.clone()))).await?;
            match next {
                Some(c) => cursor = Some(c),
                None => return Ok(()),
            }
        }
    }

    async fn status_update(&mut self, n: N<'_>, ty: &'static str) -> AppResult<()> {
        let src = n
            .id()
            .ok_or_else(|| AppError::invalid_request("update without id"))?
            .to_owned();
        let is_project = ty == "ProjectUpdate";
        let parent = if is_project {
            self.target_opt("Project", n.ref_id("project")).await?
        } else {
            self.target_opt("Initiative", n.ref_id("initiative"))
                .await?
        };
        let Some(parent) = parent else {
            self.counters(ty).skipped += 1;
            return Ok(());
        };
        let author = match self.target_opt("User", n.ref_id("user")).await? {
            Some(u) => u,
            None => match self.run.started_by {
                Some(u) => u,
                None => {
                    self.counters(ty).skipped += 1;
                    self.note(
                        "warning",
                        ty,
                        Some(&src),
                        "the author was not imported; update skipped",
                    );
                    return Ok(());
                }
            },
        };
        let now = self.now;
        let created = n.ts("createdAt").unwrap_or(now).min(now);
        let body = self
            .md(ty, &src, n.str("body"), MAX_DOCUMENT)
            .unwrap_or_default();
        let r = row(vec![
            (
                if is_project {
                    "project_id"
                } else {
                    "initiative_id"
                },
                json!(parent),
            ),
            ("user_id", json!(author)),
            ("body", json!(body)),
            (
                "health",
                json!(super::node::one_of(n.str("health"), HEALTH).unwrap_or("onTrack")),
            ),
            ("edited_at", jts(n.ts("editedAt"))),
            ("created_at", jts(Some(created))),
            (
                "updated_at",
                jts(n.ts("updatedAt").map(|t| t.min(now)).or(Some(created))),
            ),
        ]);
        let table = if is_project {
            ImportTable::ProjectUpdate
        } else {
            ImportTable::InitiativeUpdate
        };
        let up = self.upsert(ty, n, table, r, false).await?;
        if up.outcome == Outcome::Created {
            let ev = if is_project {
                DomainEvent::ProjectUpdateCreated {
                    update_id: ProjectUpdateId(up.id),
                    project_id: ProjectId(parent),
                }
            } else {
                DomainEvent::InitiativeUpdateCreated {
                    update_id: InitiativeUpdateId(up.id),
                    initiative_id: InitiativeId(parent),
                }
            };
            self.event(ev).await?;
        }
        Ok(())
    }
}
