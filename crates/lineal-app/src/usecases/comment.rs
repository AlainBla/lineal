//! usecases – Kommentare und Reaktionen (docs/03 §6): Threads, Antworten, Inline-Kommentare,
//! Bearbeiten/Löschen, Auflösen, Reaktionen, Erwähnungen.
//!
//! Sichtbarkeit und Rechte laufen immer über das Elternobjekt: Issue → Team; Projekt, Meilenstein,
//! Projekt-Update → alle Teams des Projekts; Initiative und Initiative-Update → Workspace-Recht
//! `ViewInitiatives`; Dokument → sein Parent. Nicht sichtbar → `not_found`.

use crate::app::App;
use crate::context::{Actor, RequestCtx};
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot, TeamTree, compute_visible_team_ids};
use crate::ports::Tx;
use crate::ports::repos::NewEvent;
use crate::ports::repos::comment::{
    Comment, CommentQuery, NewComment, ParentKind, ParentRef, ParentScope, Reaction, ReactionTarget,
};
use crate::ports::repos::user::{OrderBy, PageCursor};
use lineal_domain::enums::UserRole;
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::*;
use lineal_domain::rules::markdown;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use uuid::Uuid;

/// Maximale Länge eines Kommentars in Zeichen (docs/03 §6.1).
pub const MAX_BODY_CHARS: usize = 100_000;
/// Maximale Länge von `quotedText`.
pub const MAX_QUOTED_CHARS: usize = 10_000;
/// Platzhalter für gelöschte Top-Level-Kommentare mit Antworten.
pub const DELETED_PLACEHOLDER: &str = "Kommentar gelöscht";

/// Autor in Antworten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorRef {
    pub id: UserId,
    pub name: String,
    pub display_name: String,
}

/// Aggregierte Reaktionen (`reactionData`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionGroup {
    pub emoji: String,
    pub count: usize,
    pub user_ids: Vec<UserId>,
}

/// Ein Kommentar mit Autor und Reaktionen.
#[derive(Debug, Clone, PartialEq)]
pub struct CommentView {
    pub comment: Comment,
    pub author: Option<AuthorRef>,
    pub reactions: Vec<ReactionGroup>,
    /// Kennung des Issues bei Issue-Kommentaren (für die URL).
    pub issue_identifier: Option<String>,
    /// Zustand des Inline-Ankers (`attached`/`detached`), nur bei Inline-Kommentaren an Dokumenten.
    pub anchor_state: Option<&'static str>,
}

/// Ergebnis einer Schreiboperation.
#[derive(Debug, Clone, PartialEq)]
pub struct CommentMutation {
    pub view: CommentView,
    pub last_sync_id: i64,
    /// Erwähnte User mit Zugriff (bei Update nur neu hinzugekommene).
    pub mentioned_user_ids: Vec<UserId>,
}

/// Ergebnis von Löschoperationen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deleted {
    pub id: Uuid,
    pub last_sync_id: i64,
}

/// Ergebnis einer Reaktion.
#[derive(Debug, Clone, PartialEq)]
pub struct ReactionMutation {
    pub reaction: Reaction,
    pub last_sync_id: i64,
    /// `false`, wenn die Reaktion schon existierte.
    pub created: bool,
}

/// Referenz auf das Elternobjekt eines neuen Kommentars (genau eines).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentSpec {
    Issue(String),
    Project(String),
    Initiative(String),
    Document(String),
    Milestone(String),
    /// Status-Update; ohne Typ wird zuerst Projekt-, dann Initiative-Update geprüft.
    StatusUpdate {
        id: String,
        kind: Option<ParentKind>,
    },
}

/// Eingabe für [`App::create_comment`].
#[derive(Debug, Clone, Default)]
pub struct CreateCommentInput {
    /// Elternobjekt (nur ohne `reply_to`).
    pub parent: Option<ParentSpec>,
    /// Antwort auf einen Kommentar (Thread-Wurzel wird normalisiert).
    pub reply_to: Option<String>,
    pub body: String,
    /// Inline-Kommentar: zitierte Textstelle.
    pub quoted_text: Option<String>,
}

/// Eingabe für [`App::list_comments`].
#[derive(Debug, Clone)]
pub struct ListCommentsInput {
    pub parent: ParentSpec,
    pub order_by: OrderBy,
    pub cursor: Option<PageCursor>,
    /// 1..=250.
    pub limit: i64,
}

/// Eine Seite Kommentare.
#[derive(Debug, Clone, PartialEq)]
pub struct CommentPage {
    pub items: Vec<CommentView>,
    pub next: Option<PageCursor>,
}

/// Ziel einer Reaktion in Eingaben (UUID bzw. Kennung des Ziels).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReactionTargetSpec {
    Comment(String),
    Issue(String),
    ProjectUpdate(String),
    InitiativeUpdate(String),
}

fn not_found(what: &str, reference: &str) -> AppError {
    AppError::not_found(format!("Could not find referenced {what}."))
        .with_details(json!({ "reference": reference, "type": what }))
}

pub(crate) fn actor_user(ctx: &RequestCtx) -> Option<UserId> {
    match ctx.actor {
        Actor::User { id, .. } | Actor::App { id } => Some(id),
        Actor::System => None,
    }
}

/// Prüft `action` auf dem Sichtbarkeitsbereich des Elternobjekts. `write` verlangt zusätzlich den
/// Scope `write` (bei Workspace-Bereichen prüft `ViewInitiatives` ihn nicht selbst).
pub(crate) fn authorize(
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    scope: &ParentScope,
    action: Action,
    write: bool,
) -> AppResult<()> {
    match scope {
        ParentScope::Teams(teams) if teams.len() == 1 => {
            snap.require_team(ctx, action, teams[0])?;
        }
        ParentScope::Teams(teams) => {
            snap.require_teams(ctx, action, teams)?;
        }
        ParentScope::Workspace => {
            snap.require_workspace(ctx, Action::ViewInitiatives)?;
            if write && !matches!(ctx.actor, Actor::System) && !ctx.scopes.write {
                return Err(
                    AppError::forbidden("Token scope does not allow write operations.")
                        .with_details(json!({ "reason": "insufficient_scope", "scope": "write" })),
                );
            }
        }
    }
    Ok(())
}

/// Wie `authorize`, berücksichtigt aber Freigaben: bei Issue-Eltern gilt die zentrale Regel
/// `resolve::issue::authorize_issue` (Team sichtbar oder Issue mit dem Akteur geteilt).
async fn authorize_parent(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    parent: ParentRef,
    scope: &ParentScope,
    action: Action,
    write: bool,
) -> AppResult<()> {
    if let (ParentRef::Issue(issue), ParentScope::Teams(teams)) = (parent, scope)
        && teams.len() == 1
    {
        return crate::resolve::issue::authorize_issue(tx, snap, ctx, issue, teams[0], action)
            .await;
    }
    authorize(snap, ctx, scope, action, write)
}

async fn scope_of(tx: &mut dyn Tx, parent: ParentRef) -> AppResult<ParentScope> {
    tx.comments()
        .parent_scope(parent)
        .await?
        .ok_or_else(team_not_found_for_parent)
}

fn team_not_found_for_parent() -> AppError {
    // Gleiche Antwort wie für nicht sichtbare Objekte (kein Existenz-Leak).
    AppError::not_found("Could not find referenced parent.")
}

async fn resolve_parent(tx: &mut dyn Tx, spec: &ParentSpec) -> AppResult<ParentRef> {
    let (kinds, reference): (Vec<ParentKind>, &str) = match spec {
        ParentSpec::Issue(r) => (vec![ParentKind::Issue], r),
        ParentSpec::Project(r) => (vec![ParentKind::Project], r),
        ParentSpec::Initiative(r) => (vec![ParentKind::Initiative], r),
        ParentSpec::Document(r) => (vec![ParentKind::Document], r),
        ParentSpec::Milestone(r) => (vec![ParentKind::Milestone], r),
        ParentSpec::StatusUpdate { id, kind } => (
            match kind {
                Some(k @ (ParentKind::ProjectUpdate | ParentKind::InitiativeUpdate)) => vec![*k],
                Some(_) => {
                    return Err(AppError::invalid_request(
                        "statusUpdateType must be `project` or `initiative`",
                    ));
                }
                None => vec![ParentKind::ProjectUpdate, ParentKind::InitiativeUpdate],
            },
            id,
        ),
    };
    for kind in kinds {
        if let Some(p) = tx.comments().find_parent(kind, reference).await? {
            return Ok(p);
        }
    }
    let what = match spec {
        ParentSpec::Issue(_) => "Issue",
        ParentSpec::Project(_) => "Project",
        ParentSpec::Initiative(_) => "Initiative",
        ParentSpec::Document(_) => "Document",
        ParentSpec::Milestone(_) => "ProjectMilestone",
        ParentSpec::StatusUpdate { .. } => "StatusUpdate",
    };
    Err(not_found(what, reference))
}

/// Bereinigt den Body und prüft Länge/Leere.
fn clean_body(body: &str) -> AppResult<String> {
    let cleaned = markdown::sanitize(body.trim());
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return Err(AppError::invalid_request("body must not be empty")
            .with_details(json!({ "field": "body" })));
    }
    if cleaned.chars().count() > MAX_BODY_CHARS {
        return Err(AppError::invalid_request(format!(
            "body must not exceed {MAX_BODY_CHARS} characters"
        ))
        .with_details(json!({ "field": "body", "max": MAX_BODY_CHARS })));
    }
    Ok(cleaned.to_owned())
}

/// Die Erwähnungs-Kandidaten im Text: je `@`-Lauf die Namen von lang nach kurz
/// (`@bob.` → `bob.`, `bob`).
fn mention_runs(body: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = body.chars().collect();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' && (i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '@')) {
            let mut j = i + 1;
            while j < chars.len()
                && j - i - 1 < 40
                && (chars[j].is_ascii_alphanumeric() || matches!(chars[j], '.' | '_' | '-'))
            {
                j += 1;
            }
            let mut run: String = chars[i + 1..j].iter().collect::<String>().to_lowercase();
            let mut names = Vec::new();
            while !run.is_empty() {
                names.push(run.clone());
                if run.ends_with(|c: char| !c.is_ascii_alphanumeric()) {
                    run.pop();
                } else {
                    break;
                }
            }
            if !names.is_empty() {
                runs.push(names);
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    runs
}

/// Erwähnte User (`@displayName`), die Zugriff auf das Elternobjekt haben.
pub(crate) async fn mentioned_users(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    scope: &ParentScope,
    body: &str,
) -> AppResult<Vec<UserId>> {
    let runs = mention_runs(body);
    if runs.is_empty() {
        return Ok(vec![]);
    }
    let names: Vec<String> = runs
        .iter()
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let candidates = tx.comments().mention_candidates(&names).await?;
    let by_name: HashMap<&str, &crate::ports::repos::comment::MentionCandidate> = candidates
        .iter()
        .map(|c| (c.display_name.as_str(), c))
        .collect();
    let mut result: Vec<UserId> = Vec::new();
    for run in &runs {
        let Some(c) = run.iter().find_map(|n| by_name.get(n.as_str())) else {
            continue;
        };
        if result.contains(&c.id) {
            continue;
        }
        if user_can_access(tx, &snap.tree, scope, c.id, c.role).await? {
            result.push(c.id);
        }
    }
    Ok(result)
}

async fn user_can_access(
    tx: &mut dyn Tx,
    tree: &TeamTree,
    scope: &ParentScope,
    user: UserId,
    role: UserRole,
) -> AppResult<bool> {
    match scope {
        ParentScope::Workspace => Ok(role != UserRole::Guest),
        ParentScope::Teams(teams) => {
            let memberships = tx.teams().list_memberships_of_user(user).await?;
            let view = crate::policy::ActorView::new(Actor::User { id: user, role }, &memberships);
            let visible = compute_visible_team_ids(&view, tree);
            Ok(teams.iter().all(|t| visible.contains(t)))
        }
    }
}

async fn build_views(tx: &mut dyn Tx, comments: Vec<Comment>) -> AppResult<Vec<CommentView>> {
    let ids: Vec<CommentId> = comments.iter().map(|c| c.id).collect();
    let reactions = tx.comments().reactions_of_comments(&ids).await?;
    let mut grouped: HashMap<CommentId, BTreeMap<String, Vec<UserId>>> = HashMap::new();
    for r in reactions {
        if let ReactionTarget::Comment(cid) = r.target {
            grouped
                .entry(cid)
                .or_default()
                .entry(r.emoji)
                .or_default()
                .push(r.user_id);
        }
    }
    let mut authors: HashMap<UserId, Option<AuthorRef>> = HashMap::new();
    let mut identifiers: HashMap<IssueId, Option<String>> = HashMap::new();
    let mut doc_cache: HashMap<DocumentId, Option<(Option<yrs::Doc>, String)>> = HashMap::new();
    let mut views = Vec::with_capacity(comments.len());
    for c in comments {
        let author = match c.user_id {
            Some(uid) => {
                if let std::collections::hash_map::Entry::Vacant(e) = authors.entry(uid) {
                    let a = tx.users().get(uid).await?.map(|u| AuthorRef {
                        id: u.id,
                        name: u.name,
                        display_name: u.display_name,
                    });
                    e.insert(a);
                }
                authors.get(&uid).cloned().flatten()
            }
            None => None,
        };
        let issue_identifier = match c.parent {
            ParentRef::Issue(i) => {
                if let std::collections::hash_map::Entry::Vacant(e) = identifiers.entry(i) {
                    let ident = tx.comments().issue_identifier(i).await?;
                    e.insert(ident);
                }
                identifiers.get(&i).cloned().flatten()
            }
            _ => None,
        };
        let reactions = grouped
            .remove(&c.id)
            .unwrap_or_default()
            .into_iter()
            .map(|(emoji, user_ids)| ReactionGroup {
                emoji,
                count: user_ids.len(),
                user_ids,
            })
            .collect();
        let anchor_state = match (c.parent, c.parent_id, &c.quoted_text) {
            (ParentRef::Document(d), None, Some(q)) => {
                if let std::collections::hash_map::Entry::Vacant(e) = doc_cache.entry(d) {
                    e.insert(load_anchor_doc(tx, d).await?);
                }
                doc_cache
                    .get(&d)
                    .and_then(Option::as_ref)
                    .map(|(doc, markdown)| {
                        match (&c.anchor_start, &c.anchor_end, doc) {
                            (Some(a), Some(b), Some(doc)) => {
                                crate::collab::crdt::anchor_state(doc, a, b)
                                    .map(crate::collab::crdt::AnchorState::as_str)
                                    .unwrap_or("detached")
                            }
                            // Ohne Positionsanker: `detached`, sobald der Text nicht mehr vorkommt.
                            _ if markdown.contains(q.as_str()) => "attached",
                            _ => "detached",
                        }
                    })
            }
            _ => None,
        };
        views.push(CommentView {
            comment: c,
            author,
            reactions,
            issue_identifier,
            anchor_state,
        });
    }
    Ok(views)
}

/// Kommt der zitierte Text im aktuellen Klartext des Dokuments vor?
async fn quoted_text_occurs(tx: &mut dyn Tx, id: DocumentId, quoted: &str) -> AppResult<bool> {
    let Some((ydoc, markdown)) = load_anchor_doc(tx, id).await? else {
        return Ok(false);
    };
    if markdown.contains(quoted) {
        return Ok(true);
    }
    let ydoc = ydoc.unwrap_or_else(|| crate::collab::crdt::doc_from_markdown(&markdown));
    Ok(crate::collab::crdt::anchors_for_quote(&ydoc, quoted).is_some())
}

/// Dokument-Markdown und (falls vorhanden) CRDT-Zustand für die Auflösung von Inline-Ankern.
async fn load_anchor_doc(
    tx: &mut dyn Tx,
    id: DocumentId,
) -> AppResult<Option<(Option<yrs::Doc>, String)>> {
    let Some(doc) = tx.documents().get(id).await? else {
        return Ok(None);
    };
    let state = tx.documents().content_state(doc.content_id, false).await?;
    let ydoc = state
        .and_then(|s| s.state)
        .and_then(|s| crate::collab::crdt::doc_from_state(&s));
    Ok(Some((ydoc, doc.content)))
}

/// Lädt einen lebenden (nicht archivierten, nicht durch Platzhalter ersetzten) Kommentar und
/// prüft den Zugriff auf sein Elternobjekt. Liefert auch den Bereich.
async fn load_accessible(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    id: &str,
    action: Action,
    write: bool,
    allow_deleted: bool,
) -> AppResult<(Comment, ParentScope)> {
    let cid: CommentId = id.trim().parse().map_err(|_| not_found("Comment", id))?;
    let comment = tx
        .comments()
        .get(cid)
        .await?
        .filter(|c| c.archived_at.is_none() && (allow_deleted || c.deleted_at.is_none()))
        .ok_or_else(|| not_found("Comment", id))?;
    let scope = tx
        .comments()
        .parent_scope(comment.parent)
        .await?
        .ok_or_else(|| not_found("Comment", id))?;
    authorize_parent(tx, snap, ctx, comment.parent, &scope, action, write)
        .await
        .map_err(|e| {
            if e.kind == crate::error::ErrorKind::NotFound {
                not_found("Comment", id)
            } else {
                e
            }
        })?;
    Ok((comment, scope))
}

fn is_admin(ctx: &RequestCtx) -> bool {
    matches!(
        ctx.actor,
        Actor::User {
            role: UserRole::Owner | UserRole::Admin,
            ..
        } | Actor::System
    )
}

impl App {
    /// Kommentare eines Elternobjekts, flach (Wurzeln und Antworten).
    pub async fn list_comments(
        &self,
        ctx: &RequestCtx,
        input: ListCommentsInput,
    ) -> AppResult<CommentPage> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let parent = resolve_parent(&mut *tx, &input.parent).await?;
        let scope = scope_of(&mut *tx, parent).await?;
        authorize_parent(
            &mut *tx,
            &snap,
            ctx,
            parent,
            &scope,
            Action::ViewTeam,
            false,
        )
        .await?;
        let limit = input.limit.clamp(1, 250);
        let mut rows = tx
            .comments()
            .list(&CommentQuery {
                parent,
                order_by: input.order_by,
                descending: true,
                cursor: input.cursor,
                fetch: limit + 1,
            })
            .await?;
        let has_more = rows.len() as i64 > limit;
        rows.truncate(limit as usize);
        let next = if has_more {
            rows.last().map(|c| PageCursor {
                key: match input.order_by {
                    OrderBy::CreatedAt => c.created_at,
                    OrderBy::UpdatedAt => c.updated_at,
                },
                id: c.id.0,
            })
        } else {
            None
        };
        let items = build_views(&mut *tx, rows).await?;
        Ok(CommentPage { items, next })
    }

    /// Ein einzelner Kommentar (Lesezugriff über das Elternobjekt).
    pub async fn get_comment(&self, ctx: &RequestCtx, id: &str) -> AppResult<CommentView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let (comment, _) =
            load_accessible(&mut *tx, &snap, ctx, id, Action::ViewTeam, false, true).await?;
        let mut views = build_views(&mut *tx, vec![comment]).await?;
        views
            .pop()
            .ok_or_else(|| AppError::internal("comment view missing"))
    }

    /// Neuer Thread, Antwort oder Inline-Kommentar.
    pub async fn create_comment(
        &self,
        ctx: &RequestCtx,
        input: CreateCommentInput,
    ) -> AppResult<CommentMutation> {
        self.create_comment_anchored(ctx, input, None).await
    }

    /// Wie [`App::create_comment`], zusätzlich mit Positionsanker (Start, Ende; Base64) eines
    /// Inline-Kommentars; Anker und Kommentar werden in derselben Transaktion geprüft und gespeichert.
    pub async fn create_comment_anchored(
        &self,
        ctx: &RequestCtx,
        input: CreateCommentInput,
        anchor: Option<(String, String)>,
    ) -> AppResult<CommentMutation> {
        let body = clean_body(&input.body)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;

        // Elternobjekt und (normalisierte) Thread-Wurzel bestimmen.
        let mut root: Option<CommentId> = None;
        let parent = match (&input.reply_to, &input.parent) {
            (Some(_), Some(_)) => {
                return Err(AppError::invalid_request(
                    "Provide either a parent object or a comment to reply to, not both",
                ));
            }
            (Some(reply_to), None) => {
                let (target, _) = load_accessible(
                    &mut *tx,
                    &snap,
                    ctx,
                    reply_to,
                    Action::CommentIssue,
                    true,
                    // Die Wurzel eines Threads mit Platzhalter bleibt antwortbar.
                    true,
                )
                .await?;
                root = Some(target.parent_id.unwrap_or(target.id));
                target.parent
            }
            (None, Some(spec)) => resolve_parent(&mut *tx, spec).await?,
            (None, None) => {
                return Err(AppError::invalid_request(
                    "Provide exactly one of issueId, projectId, initiativeId, documentId, milestoneId, statusUpdateId or parentId",
                ));
            }
        };
        let scope = scope_of(&mut *tx, parent).await?;
        authorize_parent(
            &mut *tx,
            &snap,
            ctx,
            parent,
            &scope,
            Action::CommentIssue,
            true,
        )
        .await?;

        // Ein Status-Update hat genau einen Thread: weitere Kommentare werden Antworten.
        if root.is_none()
            && matches!(
                parent,
                ParentRef::ProjectUpdate(_) | ParentRef::InitiativeUpdate(_)
            )
        {
            root = tx.comments().first_root(parent).await?;
        }
        // Die Wurzel eines Threads, der durch Platzhalter ersetzt wurde, bleibt antwortbar.
        let quoted_text = match input.quoted_text.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(q) => {
                if root.is_some() {
                    return Err(AppError::invalid_request(
                        "quotedText is only allowed on top-level comments",
                    ));
                }
                if q.chars().count() > MAX_QUOTED_CHARS {
                    return Err(AppError::invalid_request(format!(
                        "quotedText must not exceed {MAX_QUOTED_CHARS} characters"
                    )));
                }
                Some(q.to_owned())
            }
        };

        // Inline-Kommentare (docs/06 §1.8): Anker prüfen und zitierten Text im Dokument suchen,
        // bevor etwas gespeichert wird (kein Geisterkommentar bei ungültiger Eingabe).
        if let Some((start, end)) = &anchor {
            if !crate::collab::crdt::anchor_is_valid(start)
                || !crate::collab::crdt::anchor_is_valid(end)
            {
                return Err(AppError::invalid_request(
                    "anchorStart/anchorEnd must be valid relative positions",
                )
                .with_details(json!({ "field": "anchorStart" })));
            }
            if quoted_text.is_none() || root.is_some() || !matches!(parent, ParentRef::Document(_))
            {
                return Err(AppError::invalid_request(
                    "Only inline comments on documents can have anchors",
                )
                .with_details(json!({ "field": "anchorStart" })));
            }
        }
        if let (ParentRef::Document(doc_id), Some(q)) = (parent, &quoted_text)
            && !quoted_text_occurs(&mut *tx, doc_id, q).await?
        {
            return Err(AppError::invalid_request(
                "quotedText does not occur in the document",
            )
            .with_details(json!({ "field": "quotedText" })));
        }

        let mentioned = mentioned_users(&mut *tx, &snap, &scope, &body).await?;
        let now = self.clock.now();
        let author = actor_user(ctx);
        let mut comment = tx
            .comments()
            .insert(&NewComment {
                id: CommentId(self.ids.new_uuid()),
                user_id: author,
                body,
                parent,
                parent_id: root,
                quoted_text,
                now,
            })
            .await?;
        if let Some((start, end)) = &anchor {
            tx.comments().set_anchor(comment.id, start, end).await?;
            comment = tx
                .comments()
                .get(comment.id)
                .await?
                .ok_or_else(|| AppError::internal("comment missing"))?;
        }

        // Autor abonniert das Issue (Top-Level) bzw. den Thread (Antwort).
        if let (Some(uid), ParentRef::Issue(issue)) = (author, parent) {
            let thread = comment.parent_id;
            if tx
                .comments()
                .subscribe(issue, uid, thread, "comment", now)
                .await?
            {
                tx.outbox()
                    .push(NewEvent::from_ctx(
                        DomainEvent::IssueSubscriptionCreated {
                            issue_id: issue,
                            user_id: uid,
                        },
                        ctx,
                    ))
                    .await?;
            }
        }
        if let ParentRef::Issue(issue) = parent {
            crate::usecases::automation::wake_snoozed_issue(&mut *tx, ctx, issue, author, now)
                .await?;
        }
        if let ParentRef::Issue(issue_id) = parent
            && let Some(issue) = tx.issues().get(issue_id, false).await?
        {
            Box::pin(super::issue::mentions::link_issue_mentions(
                &mut *tx,
                &snap,
                ctx,
                self.ids.as_ref(),
                now,
                crate::ports::repos::issue::HistorySource::User,
                &issue,
                &comment.body,
                None,
            ))
            .await?;
        }
        tx.outbox()
            .push(NewEvent::from_ctx(
                DomainEvent::CommentCreated {
                    comment_id: comment.id,
                    mentioned_user_ids: mentioned.clone(),
                },
                ctx,
            ))
            .await?;
        let last_sync_id = tx.last_sync_id().await?;
        let mut views = build_views(&mut *tx, vec![comment]).await?;
        let view = views
            .pop()
            .ok_or_else(|| AppError::internal("comment view missing"))?;
        tx.commit().await?;
        Ok(CommentMutation {
            view,
            last_sync_id,
            mentioned_user_ids: mentioned,
        })
    }

    /// Bearbeitet den Body (nur der Autor). Neue Erwähnungen gehen ins Event.
    /// Hängt Positionsanker (relative Positionen des Editors, Base64) an einen eigenen
    /// Inline-Kommentar eines Dokuments (docs/06 §1.8). Ungültige Kodierung → `invalid_request`.
    pub async fn set_comment_anchor(
        &self,
        ctx: &RequestCtx,
        id: &str,
        start: &str,
        end: &str,
    ) -> AppResult<CommentMutation> {
        let invalid =
            |m: &str| AppError::invalid_request(m).with_details(json!({ "field": "anchorStart" }));
        if !crate::collab::crdt::anchor_is_valid(start)
            || !crate::collab::crdt::anchor_is_valid(end)
        {
            return Err(invalid(
                "anchorStart/anchorEnd must be valid relative positions",
            ));
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let (comment, _scope) =
            load_accessible(&mut *tx, &snap, ctx, id, Action::CommentIssue, true, false).await?;
        if comment.user_id.is_none() || comment.user_id != actor_user(ctx) {
            return Err(AppError::forbidden("Only the author can anchor a comment.")
                .with_details(json!({ "reason": "not_author" })));
        }
        if !matches!(comment.parent, ParentRef::Document(_))
            || comment.parent_id.is_some()
            || comment.quoted_text.is_none()
        {
            return Err(invalid(
                "Only inline comments on documents can have anchors",
            ));
        }
        tx.comments().set_anchor(comment.id, start, end).await?;
        let updated = tx
            .comments()
            .get(comment.id)
            .await?
            .ok_or_else(|| AppError::internal("comment missing"))?;
        let last_sync_id = tx.last_sync_id().await?;
        let mut views = build_views(&mut *tx, vec![updated]).await?;
        let view = views
            .pop()
            .ok_or_else(|| AppError::internal("comment view missing"))?;
        tx.commit().await?;
        Ok(CommentMutation {
            view,
            last_sync_id,
            mentioned_user_ids: vec![],
        })
    }

    pub async fn update_comment(
        &self,
        ctx: &RequestCtx,
        id: &str,
        body: &str,
    ) -> AppResult<CommentMutation> {
        let body = clean_body(body)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let (comment, scope) =
            load_accessible(&mut *tx, &snap, ctx, id, Action::CommentIssue, true, false).await?;
        if comment.user_id.is_none() || comment.user_id != actor_user(ctx) {
            return Err(AppError::forbidden("Only the author can edit a comment.")
                .with_details(json!({ "reason": "not_author" })));
        }
        let previous: BTreeSet<UserId> = mentioned_users(&mut *tx, &snap, &scope, &comment.body)
            .await?
            .into_iter()
            .collect();
        let mentioned: Vec<UserId> = mentioned_users(&mut *tx, &snap, &scope, &body)
            .await?
            .into_iter()
            .filter(|u| !previous.contains(u))
            .collect();
        let updated = tx
            .comments()
            .update_body(comment.id, &body, self.clock.now())
            .await?;
        if let ParentRef::Issue(issue_id) = updated.parent
            && let Some(issue) = tx.issues().get(issue_id, false).await?
        {
            Box::pin(super::issue::mentions::link_issue_mentions(
                &mut *tx,
                &snap,
                ctx,
                self.ids.as_ref(),
                self.clock.now(),
                crate::ports::repos::issue::HistorySource::User,
                &issue,
                &updated.body,
                Some(&comment.body),
            ))
            .await?;
        }
        tx.outbox()
            .push(NewEvent::from_ctx(
                DomainEvent::CommentUpdated {
                    comment_id: updated.id,
                    mentioned_user_ids: mentioned.clone(),
                },
                ctx,
            ))
            .await?;
        let last_sync_id = tx.last_sync_id().await?;
        let mut views = build_views(&mut *tx, vec![updated]).await?;
        let view = views
            .pop()
            .ok_or_else(|| AppError::internal("comment view missing"))?;
        tx.commit().await?;
        Ok(CommentMutation {
            view,
            last_sync_id,
            mentioned_user_ids: mentioned,
        })
    }

    /// Löscht einen Kommentar (Autor oder Admin). Inline-Wurzeln sind nicht löschbar; eine
    /// Top-Level-Wurzel mit Antworten wird durch einen Platzhalter ersetzt.
    pub async fn delete_comment(&self, ctx: &RequestCtx, id: &str) -> AppResult<Deleted> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let (comment, _) =
            load_accessible(&mut *tx, &snap, ctx, id, Action::CommentIssue, true, false).await?;
        let is_author = comment.user_id.is_some() && comment.user_id == actor_user(ctx);
        if !is_author && !is_admin(ctx) {
            return Err(
                AppError::forbidden("Only the author or an admin can delete a comment.")
                    .with_details(json!({ "reason": "not_author" })),
            );
        }
        if comment.parent_id.is_none() && comment.quoted_text.is_some() {
            return Err(AppError::invalid_request(
                "Inline comment roots cannot be deleted. Delete the replies or resolve the thread instead",
            )
            .with_details(json!({ "reason": "inline_root" })));
        }
        let placeholder =
            if comment.parent_id.is_none() && tx.comments().count_replies(comment.id).await? > 0 {
                Some(DELETED_PLACEHOLDER)
            } else {
                None
            };
        tx.comments()
            .remove(comment.id, placeholder, self.clock.now())
            .await?;
        tx.outbox()
            .push(NewEvent::from_ctx(
                DomainEvent::CommentRemoved {
                    comment_id: comment.id,
                },
                ctx,
            ))
            .await?;
        let last_sync_id = tx.last_sync_id().await?;
        tx.commit().await?;
        Ok(Deleted {
            id: comment.id.0,
            last_sync_id,
        })
    }

    /// Löst einen Thread auf (optional mit hervorgehobener Antwort).
    pub async fn resolve_comment(
        &self,
        ctx: &RequestCtx,
        id: &str,
        resolving_comment: Option<&str>,
    ) -> AppResult<CommentMutation> {
        self.set_thread_resolution(ctx, id, true, resolving_comment)
            .await
    }

    /// Öffnet einen aufgelösten Thread wieder.
    pub async fn unresolve_comment(
        &self,
        ctx: &RequestCtx,
        id: &str,
    ) -> AppResult<CommentMutation> {
        self.set_thread_resolution(ctx, id, false, None).await
    }

    async fn set_thread_resolution(
        &self,
        ctx: &RequestCtx,
        id: &str,
        resolve: bool,
        resolving_comment: Option<&str>,
    ) -> AppResult<CommentMutation> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let (root, _) =
            load_accessible(&mut *tx, &snap, ctx, id, Action::CommentIssue, true, false).await?;
        if root.parent_id.is_some() {
            return Err(AppError::invalid_request(
                "Only the root comment of a thread can be resolved",
            ));
        }
        let resolution = if resolve {
            let reply = match resolving_comment {
                Some(r) => {
                    let rid: CommentId = r.trim().parse().map_err(|_| not_found("Comment", r))?;
                    let reply = tx
                        .comments()
                        .get(rid)
                        .await?
                        .filter(|c| c.archived_at.is_none())
                        .ok_or_else(|| not_found("Comment", r))?;
                    if reply.parent_id != Some(root.id) {
                        return Err(AppError::invalid_request(
                            "resolvingComment must be a reply in this thread",
                        ));
                    }
                    Some(reply.id)
                }
                None => None,
            };
            Some((
                actor_user(ctx)
                    .ok_or_else(|| AppError::invalid_request("Only users can resolve threads"))?,
                reply,
            ))
        } else {
            None
        };
        let updated = tx
            .comments()
            .set_resolution(root.id, resolution, self.clock.now())
            .await?;
        tx.outbox()
            .push(NewEvent::from_ctx(
                DomainEvent::CommentUpdated {
                    comment_id: updated.id,
                    mentioned_user_ids: vec![],
                },
                ctx,
            ))
            .await?;
        let last_sync_id = tx.last_sync_id().await?;
        let mut views = build_views(&mut *tx, vec![updated]).await?;
        let view = views
            .pop()
            .ok_or_else(|| AppError::internal("comment view missing"))?;
        tx.commit().await?;
        Ok(CommentMutation {
            view,
            last_sync_id,
            mentioned_user_ids: vec![],
        })
    }

    /// Reaktion hinzufügen (idempotent: eine pro User, Ziel und Emoji).
    pub async fn add_reaction(
        &self,
        ctx: &RequestCtx,
        target: ReactionTargetSpec,
        emoji: &str,
    ) -> AppResult<ReactionMutation> {
        let emoji = normalize_emoji(emoji)?;
        let user =
            actor_user(ctx).ok_or_else(|| AppError::invalid_request("Only users can react"))?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let target = resolve_reaction_target(&mut *tx, &snap, ctx, &target).await?;
        let now = self.clock.now();
        let (reaction, created) = tx
            .comments()
            .add_reaction(target, user, &emoji, now)
            .await?;
        if created {
            tx.outbox()
                .push(NewEvent::from_ctx(
                    DomainEvent::ReactionCreated {
                        reaction_id: reaction.id,
                    },
                    ctx,
                ))
                .await?;
        }
        let last_sync_id = tx.last_sync_id().await?;
        tx.commit().await?;
        Ok(ReactionMutation {
            reaction,
            last_sync_id,
            created,
        })
    }

    /// Entfernt eine Reaktion (nur durch den reagierenden User).
    pub async fn remove_reaction(&self, ctx: &RequestCtx, id: &str) -> AppResult<Deleted> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let rid: ReactionId = id.trim().parse().map_err(|_| not_found("Reaction", id))?;
        let reaction = tx
            .comments()
            .get_reaction(rid)
            .await?
            .ok_or_else(|| not_found("Reaction", id))?;
        let spec = match reaction.target {
            ReactionTarget::Comment(c) => ReactionTargetSpec::Comment(c.to_string()),
            ReactionTarget::Issue(i) => ReactionTargetSpec::Issue(i.to_string()),
            ReactionTarget::ProjectUpdate(i) => ReactionTargetSpec::ProjectUpdate(i.to_string()),
            ReactionTarget::InitiativeUpdate(i) => {
                ReactionTargetSpec::InitiativeUpdate(i.to_string())
            }
        };
        resolve_reaction_target(&mut *tx, &snap, ctx, &spec)
            .await
            .map_err(|e| {
                if e.kind == crate::error::ErrorKind::NotFound {
                    not_found("Reaction", id)
                } else {
                    e
                }
            })?;
        if Some(reaction.user_id) != actor_user(ctx) {
            return Err(
                AppError::forbidden("Only the reacting user can remove a reaction.")
                    .with_details(json!({ "reason": "not_author" })),
            );
        }
        tx.comments().delete_reaction(reaction.id).await?;
        tx.outbox()
            .push(NewEvent::from_ctx(
                DomainEvent::ReactionRemoved {
                    reaction_id: reaction.id,
                },
                ctx,
            ))
            .await?;
        let last_sync_id = tx.last_sync_id().await?;
        tx.commit().await?;
        Ok(Deleted {
            id: reaction.id.0,
            last_sync_id,
        })
    }
}

/// Löst das Reaktionsziel auf und prüft `ReactIssue` auf seinem Elternobjekt.
async fn resolve_reaction_target(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    spec: &ReactionTargetSpec,
) -> AppResult<ReactionTarget> {
    let (target, scope) = match spec {
        ReactionTargetSpec::Comment(id) => {
            let (c, scope) =
                load_accessible(tx, snap, ctx, id, Action::ReactIssue, true, false).await?;
            authorize_parent(tx, snap, ctx, c.parent, &scope, Action::ReactIssue, true).await?;
            return Ok(ReactionTarget::Comment(c.id));
        }
        ReactionTargetSpec::Issue(r) => {
            let p = resolve_parent(tx, &ParentSpec::Issue(r.clone())).await?;
            let ParentRef::Issue(i) = p else {
                return Err(not_found("Issue", r));
            };
            let scope = scope_of(tx, p).await?;
            authorize_parent(tx, snap, ctx, p, &scope, Action::ReactIssue, true).await?;
            return Ok(ReactionTarget::Issue(i));
        }
        ReactionTargetSpec::ProjectUpdate(r) | ReactionTargetSpec::InitiativeUpdate(r) => {
            let kind = if matches!(spec, ReactionTargetSpec::ProjectUpdate(_)) {
                ParentKind::ProjectUpdate
            } else {
                ParentKind::InitiativeUpdate
            };
            let p = resolve_parent(
                tx,
                &ParentSpec::StatusUpdate {
                    id: r.clone(),
                    kind: Some(kind),
                },
            )
            .await?;
            let t = match p {
                ParentRef::ProjectUpdate(i) => ReactionTarget::ProjectUpdate(i),
                ParentRef::InitiativeUpdate(i) => ReactionTarget::InitiativeUpdate(i),
                _ => return Err(not_found("StatusUpdate", r)),
            };
            (t, scope_of(tx, p).await?)
        }
    };
    authorize(snap, ctx, &scope, Action::ReactIssue, true)?;
    Ok(target)
}

/// Emoji als Shortcode-Name oder Unicode: 1–64 Zeichen, ohne Leerraum, Steuerzeichen und HTML-Zeichen.
pub fn normalize_emoji(raw: &str) -> AppResult<String> {
    let e = raw.trim().trim_matches(':');
    let bad = e.is_empty()
        || e.chars().count() > 64
        || e.chars().any(|c| {
            c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '&' | '"' | '\'' | '`')
        });
    if bad {
        return Err(
            AppError::invalid_request("Invalid emoji").with_details(json!({ "field": "emoji" }))
        );
    }
    Ok(e.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mention_runs_find_handles_and_trim_punctuation() {
        let runs = mention_runs("Hi @Bob, schau @alice.smith. Mail a@b.com und @@x @carol-1!");
        assert_eq!(runs[0], vec!["bob"]);
        assert_eq!(runs[1], vec!["alice.smith.", "alice.smith"]);
        assert_eq!(runs[2], vec!["carol-1"]);
        assert_eq!(runs.len(), 3, "{runs:?}");
    }

    #[test]
    fn emoji_is_validated() {
        assert_eq!(normalize_emoji(":thumbsup:").unwrap(), "thumbsup");
        assert_eq!(normalize_emoji("👍").unwrap(), "👍");
        assert!(normalize_emoji("").is_err());
        assert!(normalize_emoji("a b").is_err());
        assert!(normalize_emoji("<b>").is_err());
    }

    #[test]
    fn body_is_sanitized_and_checked() {
        let cleaned = clean_body("hi <script>alert(1)</script> [x](javascript:alert(1))").unwrap();
        assert!(!cleaned.contains("<script"), "{cleaned}");
        assert!(!cleaned.contains("javascript:"), "{cleaned}");
        assert!(clean_body("   ").is_err());
        assert!(clean_body(&"a".repeat(MAX_BODY_CHARS + 1)).is_err());
    }
}
