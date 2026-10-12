//! usecases – Dokumente (docs/06 §1): genau ein Elternobjekt, Reparent, Sichtbarkeit über das
//! Elternobjekt, Patch, Snapshot-Historie, Papierkorb, Markdown-Bereinigung.
//!
//! Jede Schreiboperation läuft in einer Transaktion inkl. Historie und Outbox-Event. Nicht
//! sichtbare Dokumente (privates Team, unsichtbares Projekt …) sind überall `not_found`.

mod collab;
mod rich;

pub use collab::{CollabAccess, CollabLoad, CollabTarget, PersistOutcome};

use crate::app::App;
use crate::context::{Actor, RequestCtx};
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::NewEvent;
use crate::ports::repos::document::{
    DocParent, Document, DocumentQuery, HistoryEntry, HistoryKind, NewDocument, NewHistory,
    ParentScope,
};
use crate::ports::repos::user::{OrderBy, PageCursor};
use crate::resolve;
use crate::resolve::team::Visibility;
use crate::resolve::user::actor_user_id;
use chrono::{DateTime, Utc};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::*;
use lineal_domain::rules::markdown;
use lineal_domain::rules::patch::{self, PatchFailure, PatchOp};
use serde_json::{Value, json};
use uuid::Uuid;

/// Maximale Länge des Titels (docs/06 §1.2).
pub const MAX_TITLE_CHARS: usize = 255;
/// Ruhe in Minuten, nach der eine Änderung einen neuen Snapshot beginnt (docs/06 §1.9).
pub const SNAPSHOT_QUIET_MINUTES: i64 = 10;
/// Höchstalter eines Snapshots in Minuten, bis zu dem weitere Änderungen eingebündelt werden.
pub const SNAPSHOT_MAX_MINUTES: i64 = 60;

/// Ob eine Änderung zum Zeitpunkt `now` in den letzten Snapshot `latest` eingebündelt wird: nur in
/// einen `edited`-Snapshot, wenn die vorige Änderung weniger als [`SNAPSHOT_QUIET_MINUTES`] und der
/// Snapshot weniger als [`SNAPSHOT_MAX_MINUTES`] zurückliegt.
pub fn snapshot_can_be_bundled(
    latest: &HistoryEntry,
    previous_change: DateTime<Utc>,
    now: DateTime<Utc>,
) -> bool {
    latest.kind == HistoryKind::Edited
        && now - previous_change < chrono::Duration::minutes(SNAPSHOT_QUIET_MINUTES)
        && now - latest.created_at < chrono::Duration::minutes(SNAPSHOT_MAX_MINUTES)
}

/// Das Elternobjekt eines Dokuments mit Anzeigewerten.
#[derive(Debug, Clone, PartialEq)]
pub enum ParentInfo {
    Project {
        id: ProjectId,
        identifier: String,
        name: String,
    },
    Issue {
        id: IssueId,
        identifier: String,
        title: String,
    },
    Initiative {
        id: InitiativeId,
        identifier: String,
        name: String,
    },
    Cycle {
        id: CycleId,
        number: i32,
        team_id: TeamId,
        team_key: String,
        team_name: String,
    },
    Team {
        id: TeamId,
        key: String,
        name: String,
    },
}

/// Ein Dokument mit Anzeigewerten.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentView {
    pub doc: Document,
    pub parent: Option<ParentInfo>,
    pub creator: Option<(UserId, String)>,
    pub updated_by: Option<(UserId, String)>,
    /// Kanonische App-URL (docs/06 §1.6).
    pub url: String,
    /// Warnungen der Template-Anwendung (`save_document` mit Template).
    pub warnings: Vec<String>,
}

/// Seite einer Dokumentliste.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentPage {
    pub items: Vec<DocumentView>,
    pub next: Option<PageCursor>,
}

/// Elternparameter (genau einer darf gesetzt sein, `team` zählt bei `cycle` nicht mit).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParentRefs {
    pub project: Option<String>,
    pub issue: Option<String>,
    pub initiative: Option<String>,
    pub cycle: Option<String>,
    pub team: Option<String>,
}

impl ParentRefs {
    fn given(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.project.is_some() {
            v.push("project");
        }
        if self.issue.is_some() {
            v.push("issue");
        }
        if self.initiative.is_some() {
            v.push("initiative");
        }
        if self.cycle.is_some() {
            v.push("cycle");
        }
        if self.team.is_some() {
            v.push("team");
        }
        v
    }

    /// Anzahl der Elternparameter (`team` ist bei `cycle` nur Auflösungshilfe).
    fn parent_count(&self) -> usize {
        let n = self.given().len();
        if self.cycle.is_some() && self.team.is_some() {
            n - 1
        } else {
            n
        }
    }

    /// Ob überhaupt ein Elternparameter gesetzt ist.
    pub fn any(&self) -> bool {
        !self.given().is_empty()
    }
}

/// Eingabe von `save_document` / `documentCreate` / `documentUpdate`.
#[derive(Debug, Clone, Default)]
pub struct SaveDocumentInput {
    /// Referenz des bestehenden Dokuments (Update); ohne `id` wird angelegt.
    pub id: Option<String>,
    pub title: Option<String>,
    /// Vollständiger Inhalt (Markdown).
    pub content: Option<String>,
    /// Teiländerungen (nur Update, schließt `content` aus).
    pub patch: Option<Vec<PatchOp>>,
    /// Leerer String entfernt Icon/Farbe.
    pub icon: Option<String>,
    pub color: Option<String>,
    pub parent: ParentRefs,
    pub sort_order: Option<f64>,
    /// Optimistische Sperre (nur öffentliche API): erwartete Inhalts-Revision.
    pub expected_version: Option<i32>,
    /// Dokument-Template (nur beim Anlegen).
    pub template: Option<String>,
}

/// Filter von `list_documents`.
#[derive(Debug, Clone, Default)]
pub struct ListDocumentsInput {
    pub query: Option<String>,
    pub creator: Option<String>,
    pub project: Option<String>,
    pub initiative: Option<String>,
    pub team: Option<String>,
    pub issue: Option<String>,
    pub cycle: Option<String>,
    pub include_archived: bool,
    /// Papierkorb-Ansicht („Recently deleted“).
    pub trashed: bool,
    pub created_after: Option<DateTime<Utc>>,
    pub updated_after: Option<DateTime<Utc>>,
    pub order_by: OrderBy,
    pub limit: i64,
    pub cursor: Option<PageCursor>,
}

/// Seite der Historie.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryPage {
    pub items: Vec<HistoryEntry>,
    /// Weiterblättern: Position des letzten Eintrags.
    pub next: Option<(DateTime<Utc>, Uuid)>,
}

fn not_found(reference: &str) -> AppError {
    AppError::not_found("Could not find referenced Document.")
        .with_details(json!({ "reference": reference, "type": "Document" }))
}

fn invalid(field: &str, msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg).with_details(json!({ "field": field }))
}

fn patch_error(e: patch::PatchError) -> AppError {
    let details = json!({
        "field": "content", "opIndex": e.op_index, "op": e.op,
        "anchor": e.anchor, "matches": e.matches,
    });
    match e.reason {
        PatchFailure::AnchorNotFound | PatchFailure::AnchorAmbiguous => {
            AppError::conflict(e.to_string()).with_details(details)
        }
        PatchFailure::LimitExceeded => {
            AppError::limit_exceeded(e.to_string()).with_details(details)
        }
        PatchFailure::NoOps | PatchFailure::TooManyOps | PatchFailure::EmptyArgument => {
            AppError::invalid_request(e.to_string()).with_details(details)
        }
    }
}

/// Titel-Slug der URL (docs/06 §1.6): Kleinbuchstaben, Umlaute transliteriert, Rest zu `-`,
/// zusammengefasst, an den Rändern entfernt, auf 60 Zeichen gekürzt; leer ergibt `untitled`.
pub fn title_slug(title: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true;
    for c in title.chars().flat_map(char::to_lowercase) {
        let mapped: &str = match c {
            'ä' => "ae",
            'ö' => "oe",
            'ü' => "ue",
            'ß' => "ss",
            _ => "",
        };
        if !mapped.is_empty() {
            out.push_str(mapped);
            last_dash = false;
        } else if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed: String = out.trim_matches('-').chars().take(60).collect();
    let trimmed = trimmed.trim_matches('-').to_owned();
    if trimmed.is_empty() {
        "untitled".into()
    } else {
        trimmed
    }
}

/// `slugId` aus UUID-freier Referenz: `slugId`, `<titel>-<slugId>` oder komplette URL.
pub fn slug_id_of(reference: &str) -> Option<String> {
    let r = reference.trim();
    let r = r.split(['?', '#']).next().unwrap_or(r);
    let seg = r.trim_end_matches('/').rsplit('/').next().unwrap_or(r);
    let tail = seg.rsplit('-').next().unwrap_or(seg);
    (tail.len() == 12
        && tail
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()))
    .then(|| tail.to_owned())
}

pub(crate) fn validate_title(title: &str) -> AppResult<String> {
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(invalid(
            "title",
            format!("title must not exceed {MAX_TITLE_CHARS} characters"),
        ));
    }
    Ok(title.to_owned())
}

/// Icon: `^[A-Za-z0-9_:-]{1,64}$`; leer = entfernen.
pub(crate) fn validate_icon(icon: &str) -> AppResult<Option<String>> {
    if icon.is_empty() {
        return Ok(None);
    }
    if icon.len() > 64
        || !icon
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '-'))
    {
        return Err(invalid("icon", "icon must match ^[A-Za-z0-9_:-]{1,64}$"));
    }
    Ok(Some(icon.to_owned()))
}

/// Farbe: `^#[0-9A-Fa-f]{6}$`; leer = entfernen.
pub(crate) fn validate_color(color: &str) -> AppResult<Option<String>> {
    if color.is_empty() {
        return Ok(None);
    }
    let ok = color.len() == 7
        && color.starts_with('#')
        && color[1..].chars().all(|c| c.is_ascii_hexdigit());
    if !ok {
        return Err(invalid("color", "color must be a hex color like #5E6AD2"));
    }
    Ok(Some(color.to_owned()))
}

/// Bereinigt Markdown und prüft die Länge.
pub(crate) fn clean_markdown(raw: &str) -> AppResult<String> {
    if raw.chars().count() > patch::MAX_TEXT_CHARS {
        return Err(
            AppError::limit_exceeded("content exceeds 500000 characters")
                .with_details(json!({ "field": "content" })),
        );
    }
    Ok(markdown::sanitize(raw))
}

async fn emit(tx: &mut dyn Tx, ctx: &RequestCtx, event: DomainEvent) -> AppResult<()> {
    tx.outbox().push(NewEvent::from_ctx(event, ctx)).await?;
    Ok(())
}

/// Ob der Akteur das Elternobjekt sehen darf (docs/06 §1.7).
async fn can_see(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    parent: DocParent,
    scope: &ParentScope,
) -> AppResult<bool> {
    let visible = snap.visible_team_ids();
    Ok(match parent {
        DocParent::Project(_) | DocParent::Cycle { .. } | DocParent::Team(_) => {
            scope.teams.iter().any(|t| visible.contains(t))
        }
        DocParent::Issue(i) => {
            if scope.teams.iter().any(|t| visible.contains(t)) {
                true
            } else {
                match crate::resolve::issue::share_recipient(snap, ctx) {
                    Some(u) => tx.documents().issue_shared_with(i, u).await?,
                    None => false,
                }
            }
        }
        DocParent::Initiative(_) => {
            snap.require_workspace(ctx, Action::ViewInitiatives).is_ok()
                && scope.teams.iter().all(|t| visible.contains(t))
        }
    })
}

/// Schreibrecht für Dokumente des Elternobjekts (`forbidden`; Sichtbarkeit ist vorher geprüft).
fn require_write(
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    parent: DocParent,
    scope: &ParentScope,
) -> AppResult<()> {
    let visible = snap.visible_team_ids();
    match parent {
        DocParent::Initiative(_) => {
            snap.require_workspace(ctx, Action::ViewInitiatives)?;
            if !matches!(ctx.actor, Actor::System) && !ctx.scopes.write {
                return Err(
                    AppError::forbidden("Token scope does not allow write operations.")
                        .with_details(json!({ "reason": "insufficient_scope", "scope": "write" })),
                );
            }
            for t in &scope.teams {
                snap.require_team(ctx, Action::ManageDocuments, *t)?;
            }
            Ok(())
        }
        _ => {
            let mut last: Option<AppError> = None;
            for t in scope.teams.iter().filter(|t| visible.contains(t)) {
                match snap.require_team(ctx, Action::ManageDocuments, *t) {
                    Ok(_) => return Ok(()),
                    Err(e) => last = Some(e),
                }
            }
            // Nur über Issue-Sharing sichtbar: lesen ja, schreiben nein.
            Err(last.unwrap_or_else(|| {
                AppError::forbidden("Not allowed to edit documents of this parent.")
                    .with_details(json!({ "reason": "role" }))
            }))
        }
    }
}

fn require_writable_parent(scope: &ParentScope) -> AppResult<()> {
    if scope.state.archived || scope.state.trashed {
        return Err(AppError::invalid_request("parent is archived or trashed")
            .with_details(json!({ "reason": "parent_archived_or_trashed" })));
    }
    Ok(())
}

/// Löst die Elternparameter zu genau einem Elternobjekt auf (nur sichtbare).
async fn resolve_parent(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    refs: &ParentRefs,
) -> AppResult<DocParent> {
    if refs.parent_count() != 1 {
        return Err(AppError::invalid_request(
            "Provide exactly one of project, issue, initiative, cycle, or team",
        )
        .with_details(json!({ "parameters": refs.given() })));
    }
    if let Some(r) = &refs.project {
        let p = resolve::project::resolve_in(tx, snap, r).await?;
        return Ok(DocParent::Project(p.project.id));
    }
    if let Some(r) = &refs.issue {
        let i = resolve::issue::resolve_in(tx, snap, ctx, r).await?;
        if i.trashed {
            return Err(resolve::issue::issue_not_found(r));
        }
        return Ok(DocParent::Issue(i.id));
    }
    if let Some(r) = &refs.initiative {
        if snap
            .require_workspace(ctx, Action::ViewInitiatives)
            .is_err()
        {
            return Err(AppError::not_found(format!(
                "Could not find initiative \"{r}\""
            )));
        }
        let i = resolve::initiative::resolve_in(tx, snap, r).await?;
        return Ok(DocParent::Initiative(i.id));
    }
    if let Some(r) = &refs.cycle {
        let team = match &refs.team {
            Some(t) => Some(resolve::team::resolve_in(tx, snap, t, Visibility::Content).await?),
            None => None,
        };
        let c = resolve::cycle::resolve_in(tx, snap, team.as_ref(), r)
            .await
            .map_err(|e| {
                if team.is_some() && e.kind == crate::error::ErrorKind::NotFound {
                    AppError::invalid_request("`team` does not match the cycle")
                        .with_details(json!({ "parameters": ["cycle", "team"] }))
                } else {
                    e
                }
            })?;
        return Ok(DocParent::Cycle {
            cycle: c.id,
            team: c.team_id,
        });
    }
    if let Some(r) = &refs.team {
        let t = resolve::team::resolve_in(tx, snap, r, Visibility::Content).await?;
        return Ok(DocParent::Team(t.id));
    }
    Err(AppError::internal("no parent given"))
}

/// Lädt ein Dokument per UUID, `slugId` oder URL (nicht sichtbar → `not_found`).
async fn load_visible(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    reference: &str,
    want_trashed: bool,
) -> AppResult<(Document, DocParent, ParentScope)> {
    let r = reference.trim();
    let doc = if let Ok(u) = Uuid::parse_str(r) {
        tx.documents().get(DocumentId(u)).await?
    } else if let Some(slug) = slug_id_of(r) {
        tx.documents().find_by_slug_id(&slug).await?
    } else {
        None
    };
    let doc = doc
        .filter(|d| d.trashed == want_trashed)
        .ok_or_else(|| not_found(reference))?;
    let parent = doc.parent().ok_or_else(|| not_found(reference))?;
    let scope = tx
        .documents()
        .parent_scope(parent)
        .await?
        .ok_or_else(|| not_found(reference))?;
    if !can_see(tx, snap, ctx, parent, &scope).await? {
        return Err(not_found(reference));
    }
    Ok((doc, parent, scope))
}

impl App {
    async fn document_view(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        doc: Document,
        url_key: &str,
        warnings: Vec<String>,
    ) -> AppResult<DocumentView> {
        self.document_view_opts(tx, snap, doc, url_key, warnings, true)
            .await
    }

    /// `link_issues = false` verzichtet auf das Verlinken von Issue-Kennungen (Listen: gekürzte
    /// Auszüge, keine Abfrage je Kennung).
    async fn document_view_opts(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        doc: Document,
        url_key: &str,
        warnings: Vec<String>,
        link_issues: bool,
    ) -> AppResult<DocumentView> {
        let parent = match doc.parent() {
            Some(DocParent::Project(p)) => tx.projects().get(p).await?.map(|p| {
                let key = snap
                    .tree
                    .get(p.lead_team_id)
                    .map(|t| t.key.clone())
                    .unwrap_or_default();
                ParentInfo::Project {
                    id: p.id,
                    identifier: format!("P-{key}-{}", p.number),
                    name: p.name,
                }
            }),
            Some(DocParent::Issue(i)) => {
                tx.issues().get(i, false).await?.map(|i| ParentInfo::Issue {
                    id: i.id,
                    identifier: i.identifier,
                    title: i.title,
                })
            }
            Some(DocParent::Initiative(i)) => {
                tx.initiatives()
                    .get(i)
                    .await?
                    .map(|n| ParentInfo::Initiative {
                        id: n.id,
                        identifier: n.identifier(),
                        name: n.name,
                    })
            }
            Some(DocParent::Cycle { cycle, .. }) => tx.cycles().get(cycle).await?.map(|c| {
                let (key, name) = snap
                    .tree
                    .get(c.team_id)
                    .map(|t| (t.key.clone(), t.name.clone()))
                    .unwrap_or_default();
                ParentInfo::Cycle {
                    id: c.id,
                    number: c.number,
                    team_id: c.team_id,
                    team_key: key,
                    team_name: name,
                }
            }),
            Some(DocParent::Team(t)) => snap.tree.get(t).map(|t| ParentInfo::Team {
                id: t.id,
                key: t.key.clone(),
                name: t.name.clone(),
            }),
            None => None,
        };
        let creator = match doc.creator_id {
            Some(u) => tx.users().get(u).await?.map(|u| (u.id, u.name)),
            None => None,
        };
        let updated_by = match doc.updated_by_id {
            Some(u) => tx.users().get(u).await?.map(|u| (u.id, u.name)),
            None => None,
        };
        let url = format!(
            "{}/{}/document/{}-{}",
            self.config.base_url.trim_end_matches('/'),
            url_key,
            title_slug(&doc.title),
            doc.slug_id
        );
        let mut doc = doc;
        if link_issues && !doc.content.is_empty() {
            doc.content = rich::link_visible_issues(tx, snap, url_key, &doc.content).await?;
        }
        Ok(DocumentView {
            doc,
            parent,
            creator,
            updated_by,
            url,
            warnings,
        })
    }

    async fn url_key(&self, tx: &mut dyn Tx) -> AppResult<String> {
        Ok(tx
            .orgs()
            .get()
            .await?
            .map(|o| o.url_key)
            .unwrap_or_default())
    }

    /// `get_document`: UUID, `slugId` oder URL; nicht sichtbar → `not_found`.
    pub async fn get_document(&self, ctx: &RequestCtx, reference: &str) -> AppResult<DocumentView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let (doc, _, _) = load_visible(&mut *tx, &snap, ctx, reference, false).await?;
        let url_key = self.url_key(&mut *tx).await?;
        let view = self
            .document_view(&mut *tx, &snap, doc, &url_key, vec![])
            .await?;
        tx.commit().await?;
        Ok(view)
    }

    /// `list_documents` / Suche: nur Dokumente sichtbarer Elternobjekte.
    pub async fn list_documents(
        &self,
        ctx: &RequestCtx,
        input: ListDocumentsInput,
    ) -> AppResult<DocumentPage> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let limit = input.limit.clamp(1, 250);
        let tx_ref: &mut dyn Tx = &mut *tx;
        let mut q = DocumentQuery {
            visible_teams: snap.visible_team_ids().into_iter().collect(),
            actor: actor_user_id(ctx),
            initiatives_visible: snap.require_workspace(ctx, Action::ViewInitiatives).is_ok(),
            text: input
                .query
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            trashed: input.trashed,
            include_archived: input.include_archived,
            created_after: input.created_after,
            updated_after: input.updated_after,
            order_by: input.order_by,
            cursor: input.cursor,
            fetch: limit + 1,
            ..Default::default()
        };
        if let Some(r) = &input.creator {
            q.creator = Some(resolve::user::resolve_in(tx_ref, &snap, ctx, r).await?.id);
        }
        if let Some(r) = &input.project {
            q.project = Some(
                resolve::project::resolve_in(tx_ref, &snap, r)
                    .await?
                    .project
                    .id,
            );
        }
        if let Some(r) = &input.initiative {
            if !q.initiatives_visible {
                return Err(AppError::not_found(format!(
                    "Could not find initiative \"{r}\""
                )));
            }
            q.initiative = Some(resolve::initiative::resolve_in(tx_ref, &snap, r).await?.id);
        }
        let team = match &input.team {
            Some(r) => {
                Some(resolve::team::resolve_in(tx_ref, &snap, r, Visibility::Content).await?)
            }
            None => None,
        };
        q.team = team.as_ref().map(|t| t.id);
        if let Some(r) = &input.issue {
            q.issue = Some(resolve::issue::resolve_in(tx_ref, &snap, ctx, r).await?.id);
        }
        if let Some(r) = &input.cycle {
            q.cycle = Some(
                resolve::cycle::resolve_in(tx_ref, &snap, team.as_ref(), r)
                    .await?
                    .id,
            );
        }
        let mut rows = tx.documents().list(&q).await?;
        let next = if rows.len() as i64 > limit {
            rows.truncate(limit as usize);
            rows.last().map(|d| PageCursor {
                key: match input.order_by {
                    OrderBy::CreatedAt => d.created_at,
                    OrderBy::UpdatedAt => d.updated_at,
                },
                id: d.id.0,
            })
        } else {
            None
        };
        let url_key = self.url_key(&mut *tx).await?;
        let mut items = Vec::with_capacity(rows.len());
        for d in rows {
            items.push(
                self.document_view_opts(&mut *tx, &snap, d, &url_key, vec![], false)
                    .await?,
            );
        }
        tx.commit().await?;
        Ok(DocumentPage { items, next })
    }

    /// `save_document`: legt an (ohne `id`) oder ändert (mit `id`, inkl. Reparent und Patch).
    pub async fn save_document(
        &self,
        ctx: &RequestCtx,
        input: SaveDocumentInput,
    ) -> AppResult<DocumentView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let view = self.save_document_in(&mut *tx, ctx, input).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Wie [`App::save_document`] in einer vorhandenen Transaktion (Projekt-Templates).
    pub async fn save_document_in(
        &self,
        tx: &mut dyn Tx,
        ctx: &RequestCtx,
        input: SaveDocumentInput,
    ) -> AppResult<DocumentView> {
        self.save_document_impl(tx, ctx, input, false).await
    }

    /// Wie [`App::save_document`] für automatisches Speichern des Editors: Inhaltsänderungen werden
    /// gebündelt (docs/06 §1.9) – ein neuer Snapshot entsteht erst nach [`SNAPSHOT_QUIET_MINUTES`]
    /// Ruhe oder [`SNAPSHOT_MAX_MINUTES`] seit dem letzten Snapshot; sonst wird der letzte
    /// `edited`-Snapshot fortgeschrieben (Akteure vereinigt). Ausdrückliches Speichern über
    /// `save_document` erzeugt immer einen Snapshot.
    pub async fn save_document_autosave(
        &self,
        ctx: &RequestCtx,
        input: SaveDocumentInput,
    ) -> AppResult<DocumentView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let view = self.save_document_impl(&mut *tx, ctx, input, true).await?;
        tx.commit().await?;
        Ok(view)
    }

    async fn save_document_impl(
        &self,
        tx: &mut dyn Tx,
        ctx: &RequestCtx,
        input: SaveDocumentInput,
        bundle: bool,
    ) -> AppResult<DocumentView> {
        self.save_document_with(tx, ctx, input, bundle, &collab::StateMode::FromMarkdown)
            .await
    }

    async fn save_document_with(
        &self,
        tx: &mut dyn Tx,
        ctx: &RequestCtx,
        input: SaveDocumentInput,
        bundle: bool,
        state: &collab::StateMode,
    ) -> AppResult<DocumentView> {
        let snap = PolicySnapshot::load(tx, ctx).await?;
        let now = self.clock.now();
        if input.content.is_some() && input.patch.is_some() {
            return Err(invalid(
                "patch",
                "`patch` cannot be combined with `content`",
            ));
        }
        let (doc, warnings) = match &input.id {
            None => self.create_document_in(tx, &snap, ctx, &input, now).await?,
            Some(r) => (
                self.update_document_in(tx, &snap, ctx, r, &input, now, bundle, state)
                    .await?,
                vec![],
            ),
        };
        let url_key = self.url_key(tx).await?;
        self.document_view(tx, &snap, doc, &url_key, warnings).await
    }

    async fn create_document_in(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        ctx: &RequestCtx,
        input: &SaveDocumentInput,
        now: DateTime<Utc>,
    ) -> AppResult<(Document, Vec<String>)> {
        if input.patch.is_some() {
            return Err(invalid("patch", "`patch` is only valid when updating"));
        }
        if input.expected_version.is_some() {
            return Err(invalid(
                "expectedVersion",
                "`expectedVersion` is only valid when updating",
            ));
        }
        let parent = resolve_parent(tx, snap, ctx, &input.parent).await?;
        let scope = tx
            .documents()
            .parent_scope(parent)
            .await?
            .ok_or_else(|| AppError::not_found("Could not find referenced parent."))?;
        if !can_see(tx, snap, ctx, parent, &scope).await? {
            return Err(AppError::not_found("Could not find referenced parent."));
        }
        require_write(snap, ctx, parent, &scope)?;
        require_writable_parent(&scope)?;

        // Template (optional): Titel/Inhalt/Icon/Farbe vorbelegen.
        let mut warnings = Vec::new();
        let (mut title, mut content, mut icon, mut color) = (
            input.title.clone(),
            input.content.clone(),
            input.icon.clone(),
            input.color.clone(),
        );
        let mut template_id = None;
        if let Some(t) = &input.template {
            let merged = super::template::merge_document(
                tx,
                snap,
                now,
                &scope.teams,
                t,
                title.clone(),
                content.clone(),
                icon.clone(),
                color.clone(),
            )
            .await?;
            title = merged.title;
            content = merged.content;
            icon = merged.icon;
            color = merged.color;
            template_id = Some(merged.template_id);
            warnings = merged.warnings;
        }
        let title = validate_title(
            title
                .as_deref()
                .ok_or_else(|| invalid("title", "title is required when creating a document"))?,
        )?;
        let icon = icon.as_deref().map(validate_icon).transpose()?.flatten();
        let color = color.as_deref().map(validate_color).transpose()?.flatten();
        let markdown = clean_markdown(content.as_deref().unwrap_or(""))?;
        let sort_order = match input.sort_order {
            Some(s) if s.is_finite() => s,
            Some(_) => return Err(invalid("sortOrder", "sortOrder must be a finite number")),
            None => tx
                .documents()
                .max_sort_order(parent)
                .await?
                .map_or(1.0, |m| m + 1.0),
        };
        let creator = actor_user_id(ctx);
        let doc = tx
            .documents()
            .insert(&NewDocument {
                id: DocumentId(self.ids.new_uuid()),
                content_id: self.ids.new_uuid(),
                title: title.clone(),
                markdown: markdown.clone(),
                icon,
                color,
                parent,
                creator_id: creator,
                sort_order,
                last_applied_template_id: template_id,
                now,
            })
            .await?;
        tx.documents()
            .history_insert(&NewHistory {
                id: self.ids.new_uuid(),
                document_content_id: doc.content_id,
                markdown,
                title: Some(title),
                actor_ids: creator.into_iter().collect(),
                kind: HistoryKind::Created,
                restored_from_id: None,
                now,
                created_at: None,
            })
            .await?;
        if let Some(u) = creator
            && matches!(ctx.actor, Actor::User { .. })
        {
            tx.documents()
                .subscribe(self.ids.new_uuid(), doc.id, u, "creator", now)
                .await?;
        }
        emit(
            tx,
            ctx,
            DomainEvent::DocumentCreated {
                document_id: doc.id,
            },
        )
        .await?;
        self.notify_document_mentions(tx, ctx, &doc, &scope, "", &doc.content, now)
            .await?;
        Ok((doc, warnings))
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_document_in(
        &self,
        tx: &mut dyn Tx,
        snap: &PolicySnapshot,
        ctx: &RequestCtx,
        reference: &str,
        input: &SaveDocumentInput,
        now: DateTime<Utc>,
        bundle: bool,
        state: &collab::StateMode,
    ) -> AppResult<Document> {
        if input.template.is_some() {
            return Err(invalid(
                "template",
                "`template` is only valid when creating a document",
            ));
        }
        let (mut doc, parent, scope) = load_visible(tx, snap, ctx, reference, false).await?;
        let loaded_updated_at = doc.updated_at;
        require_write(snap, ctx, parent, &scope)?;
        require_writable_parent(&scope)?;
        if let Some(v) = input.expected_version
            && v != doc.content_version
        {
            return Err(AppError::conflict("The document was changed concurrently.")
                .with_details(json!({ "currentVersion": doc.content_version })));
        }
        let actor = actor_user_id(ctx);
        let mut meta_changed = false;
        let mut moved = false;

        if input.parent.any() {
            let new_parent = resolve_parent(tx, snap, ctx, &input.parent).await?;
            if new_parent != parent {
                let new_scope = tx
                    .documents()
                    .parent_scope(new_parent)
                    .await?
                    .ok_or_else(|| AppError::not_found("Could not find referenced parent."))?;
                if !can_see(tx, snap, ctx, new_parent, &new_scope).await? {
                    return Err(AppError::not_found("Could not find referenced parent."));
                }
                require_write(snap, ctx, new_parent, &new_scope)?;
                require_writable_parent(&new_scope)?;
                doc.set_parent(new_parent);
                doc.sort_order = match input.sort_order {
                    Some(s) => s,
                    None => tx
                        .documents()
                        .max_sort_order(new_parent)
                        .await?
                        .map_or(1.0, |m| m + 1.0),
                };
                meta_changed = true;
                moved = true;
            }
        }
        if let (Some(s), false) = (input.sort_order, moved) {
            if !s.is_finite() {
                return Err(invalid("sortOrder", "sortOrder must be a finite number"));
            }
            if s != doc.sort_order {
                doc.sort_order = s;
                meta_changed = true;
            }
        }
        if let Some(t) = &input.title {
            let t = validate_title(t)?;
            if t != doc.title {
                doc.title = t;
                meta_changed = true;
            }
        }
        if let Some(i) = &input.icon {
            let i = validate_icon(i)?;
            if i != doc.icon {
                doc.icon = i;
                meta_changed = true;
            }
        }
        if let Some(c) = &input.color {
            let c = validate_color(c)?;
            if c != doc.color {
                doc.color = c;
                meta_changed = true;
            }
        }
        let new_content = match (&input.content, &input.patch) {
            (Some(c), _) => Some(clean_markdown(c)?),
            (None, Some(ops)) => {
                let out = patch::apply(&doc.content, ops).map_err(patch_error)?;
                Some(clean_markdown(&out)?)
            }
            (None, None) => None,
        };
        let content_changed = new_content.as_ref().is_some_and(|c| *c != doc.content);
        if !meta_changed && !content_changed {
            return Ok(doc);
        }
        let mut doc = if meta_changed {
            tx.documents().update(&doc, actor, now).await?
        } else {
            doc
        };
        if let (true, Some(markdown)) = (content_changed, new_content) {
            let previous_change = loaded_updated_at;
            // Zustand vor dem Schreiben sichern (sperrt die Zeile); `set_content` verwirft ihn,
            // danach wird er mitgeführt (API/MCP) bzw. vom Raum geliefert.
            let prev_state = match state {
                collab::StateMode::FromMarkdown => {
                    tx.documents().content_state(doc.content_id, true).await?
                }
                collab::StateMode::Provided { .. } => None,
            };
            let version = tx
                .documents()
                .set_content(doc.id, &markdown, false, actor, now)
                .await?;
            match state {
                collab::StateMode::FromMarkdown => {
                    collab::advance_state(tx, doc.content_id, prev_state, &markdown).await?;
                }
                collab::StateMode::Provided { state, epoch } => {
                    tx.documents()
                        .set_content_state(doc.content_id, None, Some(state), *epoch)
                        .await?;
                }
            }
            let mut actors: Vec<UserId> = actor.into_iter().collect();
            let mut created_at = None;
            if bundle
                && let Some(latest) = tx.documents().history_latest(doc.content_id).await?
                && snapshot_can_be_bundled(&latest, previous_change, now)
            {
                for a in &latest.actor_ids {
                    if !actors.contains(a) {
                        actors.push(*a);
                    }
                }
                created_at = Some(latest.created_at);
                tx.documents()
                    .history_delete(doc.content_id, latest.id)
                    .await?;
            }
            tx.documents()
                .history_insert(&NewHistory {
                    id: self.ids.new_uuid(),
                    document_content_id: doc.content_id,
                    markdown: markdown.clone(),
                    title: Some(doc.title.clone()),
                    actor_ids: actors,
                    kind: HistoryKind::Edited,
                    restored_from_id: None,
                    now,
                    created_at,
                })
                .await?;
            let old_content = std::mem::replace(&mut doc.content, markdown);
            doc.content_version = version;
            doc.updated_at = now;
            doc.updated_by_id = actor;
            self.notify_document_mentions(tx, ctx, &doc, &scope, &old_content, &doc.content, now)
                .await?;
        }
        // Reparent und Titel/Inhalt: `DocumentUpdated` (kein eigener Event-Typ für Moves).
        emit(
            tx,
            ctx,
            DomainEvent::DocumentUpdated {
                document_id: doc.id,
            },
        )
        .await?;
        Ok(doc)
    }

    /// Löscht ein Dokument in den Papierkorb (30 Tage wiederherstellbar).
    pub async fn delete_document(
        &self,
        ctx: &RequestCtx,
        reference: &str,
    ) -> AppResult<DocumentView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let (mut doc, parent, scope) = load_visible(&mut *tx, &snap, ctx, reference, false).await?;
        require_write(&snap, ctx, parent, &scope)?;
        if scope.state.archived {
            return Err(AppError::invalid_request("parent is archived or trashed")
                .with_details(json!({ "reason": "parent_archived_or_trashed" })));
        }
        doc.trashed = true;
        doc.trashed_at = Some(now);
        doc.trashed_with_parent = false;
        let doc = tx.documents().update(&doc, actor_user_id(ctx), now).await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::DocumentRemoved {
                document_id: doc.id,
            },
        )
        .await?;
        let url_key = self.url_key(&mut *tx).await?;
        let view = self
            .document_view(&mut *tx, &snap, doc, &url_key, vec![])
            .await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Stellt ein Dokument aus dem Papierkorb wieder her (Elternobjekt muss existieren und nutzbar sein).
    pub async fn restore_document(
        &self,
        ctx: &RequestCtx,
        reference: &str,
    ) -> AppResult<DocumentView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let (mut doc, parent, scope) = load_visible(&mut *tx, &snap, ctx, reference, true).await?;
        require_write(&snap, ctx, parent, &scope)?;
        if scope.state.trashed || scope.state.archived {
            return Err(AppError::invalid_request("restore parent first")
                .with_details(json!({ "reason": "parent_archived_or_trashed" })));
        }
        doc.trashed = false;
        doc.trashed_at = None;
        doc.trashed_with_parent = false;
        let doc = tx.documents().update(&doc, actor_user_id(ctx), now).await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::DocumentUpdated {
                document_id: doc.id,
            },
        )
        .await?;
        let url_key = self.url_key(&mut *tx).await?;
        let view = self
            .document_view(&mut *tx, &snap, doc, &url_key, vec![])
            .await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Snapshot-Historie eines Dokuments (neueste zuerst); lesbar für alle, die das Dokument sehen.
    pub async fn document_history(
        &self,
        ctx: &RequestCtx,
        reference: &str,
        before: Option<(DateTime<Utc>, Uuid)>,
        limit: i64,
    ) -> AppResult<HistoryPage> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let (doc, _, _) = load_visible(&mut *tx, &snap, ctx, reference, false).await?;
        let limit = limit.clamp(1, 250);
        let mut items = tx
            .documents()
            .history_list(doc.content_id, before, limit + 1)
            .await?;
        let next = if items.len() as i64 > limit {
            items.truncate(limit as usize);
            items.last().map(|h| (h.content_snapshot_at, h.id))
        } else {
            None
        };
        tx.commit().await?;
        Ok(HistoryPage { items, next })
    }

    /// Stellt den Inhalt eines Snapshots als neue aktuelle Version wieder her (Snapshot `restored`).
    pub async fn restore_document_version(
        &self,
        ctx: &RequestCtx,
        reference: &str,
        history_id: &str,
    ) -> AppResult<DocumentView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let (doc, parent, scope) = load_visible(&mut *tx, &snap, ctx, reference, false).await?;
        require_write(&snap, ctx, parent, &scope)?;
        require_writable_parent(&scope)?;
        let hid = Uuid::parse_str(history_id.trim())
            .map_err(|_| AppError::not_found("Could not find referenced document version."))?;
        let entry = tx
            .documents()
            .history_get(doc.content_id, hid)
            .await?
            .ok_or_else(|| AppError::not_found("Could not find referenced document version."))?;
        let actor = actor_user_id(ctx);
        let markdown = clean_markdown(&entry.markdown)?;
        let prev_state = tx.documents().content_state(doc.content_id, true).await?;
        let version = tx
            .documents()
            .set_content(doc.id, &markdown, true, actor, now)
            .await?;
        collab::advance_state(&mut *tx, doc.content_id, prev_state, &markdown).await?;
        tx.documents()
            .history_insert(&NewHistory {
                id: self.ids.new_uuid(),
                document_content_id: doc.content_id,
                markdown: markdown.clone(),
                title: Some(doc.title.clone()),
                actor_ids: actor.into_iter().collect(),
                kind: HistoryKind::Restored,
                restored_from_id: Some(entry.id),
                now,
                created_at: None,
            })
            .await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::DocumentUpdated {
                document_id: doc.id,
            },
        )
        .await?;
        let mut doc = doc;
        doc.content = markdown;
        doc.content_version = version;
        doc.restored_at = Some(now);
        doc.updated_at = now;
        doc.updated_by_id = actor;
        let url_key = self.url_key(&mut *tx).await?;
        let view = self
            .document_view(&mut *tx, &snap, doc, &url_key, vec![])
            .await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Kaskade beim Löschen eines Elternobjekts (Aufruf durch dessen Use-Case, z. B. Issue-Papierkorb).
    pub async fn trash_documents_of(
        &self,
        tx: &mut dyn Tx,
        parent: DocParent,
    ) -> AppResult<Vec<DocumentId>> {
        tx.documents().cascade_trash(parent, self.clock.now()).await
    }

    /// Gegenstück: stellt die mit dem Elternobjekt mitgelöschten Dokumente wieder her.
    pub async fn restore_documents_of(
        &self,
        tx: &mut dyn Tx,
        parent: DocParent,
    ) -> AppResult<Vec<DocumentId>> {
        tx.documents().cascade_restore(parent).await
    }
}

/// Dokument-JSON-Hilfe für Adapter: Wert des Elternobjekts als `(kind, id)`.
pub fn parent_kind_id(parent: &ParentInfo) -> (&'static str, Value) {
    match parent {
        ParentInfo::Project { id, .. } => ("project", json!(id.to_string())),
        ParentInfo::Issue { id, .. } => ("issue", json!(id.to_string())),
        ParentInfo::Initiative { id, .. } => ("initiative", json!(id.to_string())),
        ParentInfo::Cycle { id, .. } => ("cycle", json!(id.to_string())),
        ParentInfo::Team { id, .. } => ("team", json!(id.to_string())),
    }
}
