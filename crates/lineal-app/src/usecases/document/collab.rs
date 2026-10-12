//! Echtzeit-Mitbearbeitung (WP36, docs/06 §1.9 Stufe 2): Zugriffsprüfung, Laden und Speichern des
//! CRDT-Zustands eines Inhalts. Der Raum selbst (Verbindungen, Verteilung) lebt in `lineal-sync`;
//! hier liegt alles, was Rechte, Transaktionen, Snapshots und Outbox-Events berührt – damit ein
//! Raum-Speichern denselben Weg geht wie `save_document_autosave`.

use super::*;
use crate::collab::crdt;
use crate::ports::repos::document::ContentState;

/// Was ein Raum bearbeitet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CollabTarget {
    /// Inhalt eines Dokuments (`document_content.id` des Dokuments, adressiert über die Dokument-ID).
    Document(DocumentId),
}

/// Ergebnis der Zugriffsprüfung beim Beitritt bzw. bei der periodischen Re-Validierung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollabAccess {
    pub content_id: Uuid,
    pub title: String,
    /// Bearbeitungsrecht (sonst nur Lesen: Updates des Clients werden verworfen).
    pub can_write: bool,
}

/// Geladener Inhalt eines Raums.
#[derive(Debug, Clone, PartialEq)]
pub struct CollabLoad {
    pub content_id: Uuid,
    pub markdown: String,
    pub state: Option<String>,
    pub epoch: Option<Uuid>,
    pub version: i32,
}

impl From<ContentState> for CollabLoad {
    fn from(c: ContentState) -> Self {
        Self {
            content_id: c.content_id,
            markdown: c.markdown,
            state: c.state,
            epoch: c.epoch,
            version: c.version,
        }
    }
}

/// Ergebnis von [`App::collab_persist`].
#[derive(Debug, Clone, PartialEq)]
pub enum PersistOutcome {
    /// Gespeichert (neue Inhaltsrevision).
    Saved { version: i32 },
    /// Inhalt unverändert (nur der Zustand wurde nachgeführt).
    Unchanged { version: i32 },
    /// Zwischenzeitlich extern geändert (API/MCP/Wiederherstellung): der Raum muss den aktuellen
    /// Stand einarbeiten und erneut speichern.
    Stale(CollabLoad),
}

/// Wie der CRDT-Zustand beim Schreiben von Inhalt behandelt wird.
pub(super) enum StateMode {
    /// API/MCP/Wiederherstellung: Markdown ist maßgeblich, ein vorhandener Zustand wird per
    /// minimaler Änderung nachgeführt.
    FromMarkdown,
    /// Raum-Speichern: Zustand wird mitgeliefert.
    Provided { state: String, epoch: Option<Uuid> },
}

/// Führt einen vor dem Schreiben gesicherten Zustand an den neuen Markdown heran. Nur wenn der
/// Zustand zum vorherigen Markdown passt (sonst war er veraltet und bleibt verworfen).
pub(super) async fn advance_state(
    tx: &mut dyn Tx,
    content_id: Uuid,
    prev: Option<ContentState>,
    new_markdown: &str,
) -> AppResult<()> {
    let Some(prev) = prev else { return Ok(()) };
    let Some(state) = prev.state.as_deref() else {
        return Ok(());
    };
    let Some(doc) = crdt::doc_from_state(state) else {
        return Ok(());
    };
    if crdt::sanitized_markdown_of(&doc) != prev.markdown {
        return Ok(());
    }
    crdt::apply_markdown(&doc, new_markdown);
    if crdt::check_limits(&doc).is_err() {
        // Zu tief/zu groß: Zustand verwerfen, der Raum baut beim nächsten Öffnen neu aus Markdown.
        return Ok(());
    }
    tx.documents()
        .set_content_state(
            content_id,
            None,
            Some(&crdt::state_to_b64(&doc)),
            prev.epoch,
        )
        .await?;
    Ok(())
}

impl App {
    /// Prüft Sichtbarkeit und Bearbeitungsrecht. Nicht sichtbar → `not_found` (verrät nichts).
    pub async fn collab_open(
        &self,
        ctx: &RequestCtx,
        target: CollabTarget,
    ) -> AppResult<CollabAccess> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = self.collab_access_in(&mut *tx, &snap, ctx, target).await?;
        tx.commit().await?;
        Ok(access)
    }

    async fn collab_access_in(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        ctx: &RequestCtx,
        target: CollabTarget,
    ) -> AppResult<CollabAccess> {
        match target {
            CollabTarget::Document(id) => {
                let (doc, parent, scope) =
                    load_visible(tx, snap, ctx, &id.0.to_string(), false).await?;
                let scope_ok = matches!(ctx.actor, Actor::System) || ctx.scopes.write;
                let can_write = scope_ok
                    && require_write(snap, ctx, parent, &scope).is_ok()
                    && require_writable_parent(&scope).is_ok();
                Ok(CollabAccess {
                    content_id: doc.content_id,
                    title: doc.title,
                    can_write,
                })
            }
        }
    }

    /// Lädt Inhalt und CRDT-Zustand (Lesezugriff nötig).
    pub async fn collab_load(
        &self,
        ctx: &RequestCtx,
        target: CollabTarget,
    ) -> AppResult<CollabLoad> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = self.collab_access_in(&mut *tx, &snap, ctx, target).await?;
        let cs = tx
            .documents()
            .content_state(access.content_id, false)
            .await?
            .ok_or_else(|| AppError::not_found("Could not find document content."))?;
        tx.commit().await?;
        Ok(cs.into())
    }

    /// Hinterlegt den frisch erzeugten Zustand samt Epoche (Erstbefüllung aus Markdown), sofern die
    /// Inhaltsrevision noch stimmt. Ändert weder Markdown noch Version und löst keine Ereignisse aus.
    pub async fn collab_seed(
        &self,
        ctx: &RequestCtx,
        target: CollabTarget,
        state: &str,
        epoch: Uuid,
        version: i32,
    ) -> AppResult<bool> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = self.collab_access_in(&mut *tx, &snap, ctx, target).await?;
        let written = tx
            .documents()
            .set_content_state(access.content_id, Some(version), Some(state), Some(epoch))
            .await?;
        tx.commit().await?;
        Ok(written)
    }

    /// Aktuelle Inhaltsrevision (leichte Abfrage für das Erkennen externer Änderungen).
    pub async fn collab_version(&self, ctx: &RequestCtx, target: CollabTarget) -> AppResult<i32> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = self.collab_access_in(&mut *tx, &snap, ctx, target).await?;
        let cs = tx
            .documents()
            .content_state(access.content_id, false)
            .await?
            .ok_or_else(|| AppError::not_found("Could not find document content."))?;
        tx.commit().await?;
        Ok(cs.version)
    }

    /// Speichert den Stand eines Raums wie das automatische Speichern des Editors: bereinigtes
    /// Markdown, Version +1, Snapshot-Bündelung, `DocumentUpdated` und Erwähnungs-Benachrichtigungen
    /// (nur für neu hinzugekommene Erwähnungen) – zusammen mit dem Zustand in einer Transaktion.
    /// Recht und Sichtbarkeit werden mit `ctx` (dem schreibenden User) frisch geprüft.
    /// `expected_version` ist die Revision, auf der der Raum aufbaut; weicht sie ab, wurde extern
    /// geschrieben ([`PersistOutcome::Stale`]).
    pub async fn collab_persist(
        &self,
        ctx: &RequestCtx,
        target: CollabTarget,
        markdown: &str,
        state: &str,
        epoch: Uuid,
        expected_version: i32,
    ) -> AppResult<PersistOutcome> {
        let CollabTarget::Document(doc_id) = target;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let access = self.collab_access_in(&mut *tx, &snap, ctx, target).await?;
        if !access.can_write {
            return Err(AppError::forbidden(
                "Not allowed to edit documents of this parent.",
            ));
        }
        let cs = tx
            .documents()
            .content_state(access.content_id, true)
            .await?
            .ok_or_else(|| AppError::not_found("Could not find document content."))?;
        if cs.version != expected_version {
            tx.commit().await?;
            return Ok(PersistOutcome::Stale(cs.into()));
        }
        let clean = clean_markdown(markdown)?;
        if clean == cs.markdown {
            tx.documents()
                .set_content_state(
                    access.content_id,
                    Some(cs.version),
                    Some(state),
                    Some(epoch),
                )
                .await?;
            tx.commit().await?;
            return Ok(PersistOutcome::Unchanged {
                version: cs.version,
            });
        }
        let input = SaveDocumentInput {
            id: Some(doc_id.0.to_string()),
            content: Some(clean),
            ..Default::default()
        };
        let mode = StateMode::Provided {
            state: state.to_owned(),
            epoch: Some(epoch),
        };
        let view = self
            .save_document_with(&mut *tx, ctx, input, true, &mode)
            .await?;
        tx.commit().await?;
        Ok(PersistOutcome::Saved {
            version: view.doc.content_version,
        })
    }
}
