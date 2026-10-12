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
    /// Inhalt eines Dokuments (adressiert über die Dokument-ID, nie über die Inhalts-ID).
    Document(DocumentId),
    /// Beschreibung eines Issues (`issue.description`); der Zustand liegt in `document_content`
    /// (`owner_type = 'issue'`), der Markdown-Text bleibt in der Beschreibung.
    Issue(IssueId),
    /// Markdown-Beschreibung eines Projekts (`project.content`).
    Project(ProjectId),
    /// Markdown-Beschreibung einer Initiative (`initiative.content`).
    Initiative(InitiativeId),
}

impl CollabTarget {
    /// Besitzerart der Inhaltszeile und Besitzer-ID (für Beschreibungen).
    fn owner(self) -> Option<(&'static str, Uuid)> {
        match self {
            Self::Document(_) => None,
            Self::Issue(i) => Some(("issue", i.0)),
            Self::Project(p) => Some(("project", p.0)),
            Self::Initiative(i) => Some(("initiative", i.0)),
        }
    }
}

/// Ergebnis der Zugriffsprüfung beim Beitritt bzw. bei der periodischen Re-Validierung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollabAccess {
    pub content_id: Uuid,
    pub title: String,
    /// Bearbeitungsrecht (sonst nur Lesen: Updates des Clients werden verworfen).
    pub can_write: bool,
    /// Anzeigename des Akteurs (für Cursor-Anzeige; der Server setzt ihn, nicht der Client).
    pub user_name: String,
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

/// Aktueller Stand des Besitzerobjekts einer Beschreibung.
struct OwnerView {
    title: String,
    markdown: String,
    version: i32,
    can_write: bool,
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
        let write_scope = matches!(ctx.actor, Actor::System) || ctx.scopes.write;
        let user_name = match actor_user_id(ctx) {
            Some(u) => tx.users().get(u).await?.map(|u| u.name),
            None => None,
        }
        .unwrap_or_else(|| "Unknown".to_owned());
        match target {
            CollabTarget::Document(id) => {
                let (doc, parent, scope) =
                    load_visible(tx, snap, ctx, &id.0.to_string(), false).await?;
                let can_write = write_scope
                    && require_write(snap, ctx, parent, &scope).is_ok()
                    && require_writable_parent(&scope).is_ok();
                Ok(CollabAccess {
                    content_id: doc.content_id,
                    title: doc.title,
                    can_write,
                    user_name,
                })
            }
            _ => {
                let o = self.owner_in(tx, snap, ctx, target).await?;
                Ok(CollabAccess {
                    content_id: Uuid::nil(),
                    title: o.title,
                    can_write: write_scope && o.can_write,
                    user_name,
                })
            }
        }
    }

    /// Beschreibung eines Issues/Projekts/einer Initiative: Sichtbarkeit wie beim Lesen des
    /// Objekts (nicht sichtbar → `not_found`), Schreibrecht wie beim Ändern des Objekts.
    async fn owner_in(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        ctx: &RequestCtx,
        target: CollabTarget,
    ) -> AppResult<OwnerView> {
        match target {
            CollabTarget::Document(_) => Err(AppError::internal("not an owner target")),
            CollabTarget::Issue(id) => {
                let i = crate::resolve::issue::resolve_in(tx, snap, ctx, &id.0.to_string()).await?;
                if i.trashed {
                    return Err(crate::resolve::issue::issue_not_found(&id.0.to_string()));
                }
                let can_write = i.archived_at.is_none()
                    && snap
                        .require_team(ctx, Action::UpdateIssue, i.team_id)
                        .is_ok();
                Ok(OwnerView {
                    title: i.title,
                    markdown: i.description.unwrap_or_default(),
                    version: i.version,
                    can_write,
                })
            }
            CollabTarget::Project(id) => {
                let r = crate::resolve::project::resolve_in(tx, snap, &id.0.to_string()).await?;
                let p = r.project;
                if p.trashed {
                    return Err(AppError::not_found("Could not find project"));
                }
                let can_write = p.archived_at.is_none()
                    && crate::usecases::project::require_manage(
                        snap,
                        ctx,
                        Action::ManageProject,
                        &r.teams,
                    )
                    .is_ok();
                Ok(OwnerView {
                    title: p.name,
                    markdown: p.content.unwrap_or_default(),
                    version: p.version,
                    can_write,
                })
            }
            CollabTarget::Initiative(id) => {
                if snap
                    .require_workspace(ctx, Action::ViewInitiatives)
                    .is_err()
                {
                    return Err(AppError::not_found("Could not find initiative"));
                }
                let i = crate::resolve::initiative::resolve_in(tx, snap, &id.0.to_string()).await?;
                if i.trashed {
                    return Err(AppError::not_found("Could not find initiative"));
                }
                let can_write = i.archived_at.is_none()
                    && snap
                        .require_workspace(ctx, Action::ManageInitiatives)
                        .is_ok();
                Ok(OwnerView {
                    title: i.name,
                    markdown: i.content.unwrap_or_default(),
                    version: i.version,
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
        let load = self.collab_load_in(&mut *tx, &snap, ctx, target).await?;
        tx.commit().await?;
        Ok(load)
    }

    async fn collab_load_in(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        ctx: &RequestCtx,
        target: CollabTarget,
    ) -> AppResult<CollabLoad> {
        let mut load: CollabLoad = match target.owner() {
            None => {
                let access = self.collab_access_in(tx, snap, ctx, target).await?;
                tx.documents()
                    .content_state(access.content_id, false)
                    .await?
                    .ok_or_else(|| AppError::not_found("Could not find document content."))?
                    .into()
            }
            Some((owner_type, owner_id)) => {
                let o = self.owner_in(tx, snap, ctx, target).await?;
                let cs = tx
                    .documents()
                    .content_state_of_owner(owner_type, owner_id, &o.markdown, false)
                    .await?;
                CollabLoad {
                    content_id: cs.content_id,
                    // Der Text steht am Besitzer (Beschreibung), die Zeile hält nur den Zustand.
                    markdown: o.markdown,
                    state: cs.state,
                    epoch: cs.epoch,
                    version: o.version,
                }
            }
        };
        // Zustand, der nicht (mehr) zum Text passt (Schreiben am Raum vorbei), wird verworfen.
        if let Some(state) = load.state.as_deref() {
            let fresh = crdt::doc_from_state(state).is_some_and(|d| {
                crdt::check_limits(&d).is_ok() && crdt::sanitized_markdown_of(&d) == load.markdown
            });
            if !fresh {
                load.state = None;
                load.epoch = None;
            }
        }
        Ok(load)
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
        let written = match target.owner() {
            None => {
                let access = self.collab_access_in(&mut *tx, &snap, ctx, target).await?;
                tx.documents()
                    .set_content_state(access.content_id, Some(version), Some(state), Some(epoch))
                    .await?
            }
            Some((owner_type, owner_id)) => {
                let o = self.owner_in(&mut *tx, &snap, ctx, target).await?;
                if o.version != version {
                    false
                } else {
                    let cs = tx
                        .documents()
                        .content_state_of_owner(owner_type, owner_id, &o.markdown, true)
                        .await?;
                    tx.documents()
                        .set_content_state(cs.content_id, None, Some(state), Some(epoch))
                        .await?
                }
            }
        };
        tx.commit().await?;
        Ok(written)
    }

    /// Altbestand: Inline-Kommentare ohne Positionsanker bekommen Anker aus `quotedText` (erstes
    /// Vorkommen im Text), sobald der Raum geladen wird; ohne Treffer bleibt der Fallback `quotedText`.
    pub async fn collab_backfill_anchors(
        &self,
        ctx: &RequestCtx,
        target: CollabTarget,
        doc: &yrs::Doc,
    ) -> AppResult<usize> {
        let CollabTarget::Document(doc_id) = target else {
            return Ok(0);
        };
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        // Lesezugriff genügt nicht: Anker schreiben nur Berechtigte (Sichtbarkeit wird geprüft).
        let access = self.collab_access_in(&mut *tx, &snap, ctx, target).await?;
        if !access.can_write {
            return Ok(0);
        }
        let pending = tx.comments().inline_without_anchor(doc_id).await?;
        let mut n = 0;
        for (id, quoted) in pending {
            if let Some((a, b)) = crdt::anchors_for_quote(doc, &quoted) {
                tx.comments().set_anchor(id, &a, &b).await?;
                n += 1;
            }
        }
        tx.commit().await?;
        Ok(n)
    }

    /// Aktuelle Inhaltsrevision (leichte Abfrage für das Erkennen externer Änderungen).
    pub async fn collab_version(&self, ctx: &RequestCtx, target: CollabTarget) -> AppResult<i32> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let version = match target.owner() {
            None => {
                let access = self.collab_access_in(&mut *tx, &snap, ctx, target).await?;
                tx.documents()
                    .content_state(access.content_id, false)
                    .await?
                    .ok_or_else(|| AppError::not_found("Could not find document content."))?
                    .version
            }
            Some(_) => self.owner_in(&mut *tx, &snap, ctx, target).await?.version,
        };
        tx.commit().await?;
        Ok(version)
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
        let CollabTarget::Document(doc_id) = target else {
            return self
                .collab_persist_owner(ctx, target, markdown, state, epoch, expected_version)
                .await;
        };
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

    /// Speichern der Beschreibung eines Issues/Projekts/einer Initiative über denselben Weg wie
    /// ein normales Ändern (Bereinigung, Version, Verlauf, Ereignisse, Erwähnungen). Die Prüfung der
    /// Revision und das Schreiben laufen in getrennten Transaktionen: eine API-Änderung dazwischen
    /// wird vom Raum beim nächsten Abgleich zusammengeführt (letzter Schreiber gewinnt den Text).
    async fn collab_persist_owner(
        &self,
        ctx: &RequestCtx,
        target: CollabTarget,
        markdown: &str,
        state: &str,
        epoch: Uuid,
        expected_version: i32,
    ) -> AppResult<PersistOutcome> {
        let Some((owner_type, owner_id)) = target.owner() else {
            return Err(AppError::internal("not an owner target"));
        };
        let (owner, load, can_write) = {
            let mut tx = self.store.begin(ctx.org_id).await?;
            let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
            let access = self.collab_access_in(&mut *tx, &snap, ctx, target).await?;
            let o = self.owner_in(&mut *tx, &snap, ctx, target).await?;
            let load = self.collab_load_in(&mut *tx, &snap, ctx, target).await?;
            tx.commit().await?;
            (o, load, access.can_write)
        };
        if !can_write {
            return Err(AppError::forbidden("Not allowed to edit this content."));
        }
        if owner.version != expected_version {
            return Ok(PersistOutcome::Stale(load));
        }
        let clean = clean_markdown(markdown)?;
        let version = if clean == owner.markdown {
            owner.version
        } else {
            match target {
                CollabTarget::Issue(id) => {
                    self.update_issue(
                        ctx,
                        crate::usecases::issue::UpdateIssueInput {
                            id: id.0.to_string(),
                            description: Some(crate::usecases::issue::DescriptionEdit::Set(clean)),
                            source: crate::ports::repos::issue::HistorySource::User,
                            ..Default::default()
                        },
                    )
                    .await?
                    .issue
                    .version
                }
                CollabTarget::Project(id) => {
                    self.save_project(
                        ctx,
                        crate::usecases::project::SaveProjectInput {
                            id: Some(id.0.to_string()),
                            description: Some(clean),
                            ..Default::default()
                        },
                    )
                    .await?
                    .project
                    .version
                }
                CollabTarget::Initiative(id) => {
                    self.save_initiative(
                        ctx,
                        crate::usecases::initiative::SaveInitiativeInput {
                            id: Some(id.0.to_string()),
                            description: Some(clean),
                            ..Default::default()
                        },
                    )
                    .await?
                    .initiative
                    .version
                }
                CollabTarget::Document(_) => {
                    return Err(AppError::internal("document target in owner path"));
                }
            }
        };
        // Zustand festhalten (eigene Transaktion: der Text liegt am Besitzer).
        let mut tx = self.store.begin(ctx.org_id).await?;
        let cs = tx
            .documents()
            .content_state_of_owner(owner_type, owner_id, &load.markdown, true)
            .await?;
        tx.documents()
            .set_content_state(cs.content_id, None, Some(state), Some(epoch))
            .await?;
        tx.commit().await?;
        Ok(if version == owner.version {
            PersistOutcome::Unchanged { version }
        } else {
            PersistOutcome::Saved { version }
        })
    }
}
