//! Mehrfachaktionen auf Issues (docs/02 §7.4, docs/08): `issue_batch_update` wendet dieselbe
//! Änderung auf bis zu [`MAX_BATCH`] Issues an. Jedes Issue läuft in einer eigenen Transaktion
//! mit den normalen Prüfungen von `update_issue`/`delete_issue`; Teilerfolge werden mit einer
//! Fehlerliste gemeldet (Festlegung Lineal, abweichend vom „alles oder nichts“ in docs/02 §7.4).

use super::history::{self, Delta};
use super::{IssueView, UpdateIssueInput, emit, invalid, load_view};
use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult, ErrorKind};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::repos::issue::HistorySource;
use crate::resolve;
use crate::resolve::user::actor_user_id;
use lineal_domain::events::DomainEvent;
use std::collections::HashSet;

/// Obergrenze je Aufruf (entspricht dem Listenmaximum).
pub const MAX_BATCH: usize = 250;

/// Änderung für alle ausgewählten Issues (Tri-State: `None` = unverändert).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BatchPatch {
    pub state: Option<String>,
    pub priority: Option<i64>,
    pub assignee: Option<Option<String>>,
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<String>,
    pub project: Option<Option<String>>,
    pub cycle: Option<Option<String>>,
    pub estimate: Option<Option<f64>>,
    /// Zielteam (Bulk-Move, docs/02 §7.4).
    pub team: Option<String>,
    pub move_sub_issues: bool,
    /// Archivieren (Issue bleibt wiederherstellbar).
    pub archive: bool,
    /// In den Papierkorb legen.
    pub delete: bool,
}

impl BatchPatch {
    fn has_field_changes(&self) -> bool {
        self.state.is_some()
            || self.priority.is_some()
            || self.assignee.is_some()
            || !self.add_labels.is_empty()
            || !self.remove_labels.is_empty()
            || self.project.is_some()
            || self.cycle.is_some()
            || self.estimate.is_some()
            || self.team.is_some()
    }
}

/// Eingabe von `issue_batch_update`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BatchUpdateInput {
    /// UUIDs oder Kennungen.
    pub ids: Vec<String>,
    pub patch: BatchPatch,
}

/// Fehler eines einzelnen Issues.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchFailure {
    pub id: String,
    /// Fehlercode des Fehlermodells (`not_found`, `forbidden`, …).
    pub code: String,
    pub message: String,
}

/// Ergebnis: erfolgreiche Issues (in Eingabereihenfolge) und Fehler.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BatchResult {
    pub updated: Vec<IssueView>,
    pub failures: Vec<BatchFailure>,
}

fn validate(input: &BatchUpdateInput) -> AppResult<Vec<String>> {
    let mut seen = HashSet::new();
    let ids: Vec<String> = input
        .ids
        .iter()
        .map(|i| i.trim().to_owned())
        // Leere IDs bleiben erhalten und werden je Eintrag als Fehler gemeldet (B8.23).
        .filter(|i| i.is_empty() || seen.insert(i.to_lowercase()))
        .collect();
    if ids.iter().all(String::is_empty) {
        return Err(invalid("ids", "empty", "select at least one issue"));
    }
    if ids.len() > MAX_BATCH {
        return Err(
            AppError::limit_exceeded(format!("at most {MAX_BATCH} issues per call"))
                .with_details(serde_json::json!({ "limit": "issueBatchUpdate", "max": MAX_BATCH })),
        );
    }
    let p = &input.patch;
    if p.archive && p.delete {
        return Err(invalid(
            "patch",
            "archive_and_delete",
            "archive and delete cannot be combined",
        ));
    }
    if (p.archive || p.delete) && p.has_field_changes() {
        return Err(invalid(
            "patch",
            "archive_with_changes",
            "archive/delete cannot be combined with other changes",
        ));
    }
    if !p.archive && !p.delete && !p.has_field_changes() {
        return Err(invalid("patch", "empty", "no change requested"));
    }
    if p.team.is_some() && (!p.add_labels.is_empty() || !p.remove_labels.is_empty()) {
        return Err(invalid(
            "team",
            "team_change_with_add_labels",
            "addLabels/removeLabels cannot be combined with a team change",
        ));
    }
    if p.move_sub_issues && p.team.is_none() {
        return Err(invalid(
            "moveSubIssues",
            "needs_team",
            "moveSubIssues requires a target team",
        ));
    }
    Ok(ids)
}

impl App {
    /// Wendet `patch` auf alle `ids` an (je Issue eigene Transaktion).
    pub async fn issue_batch_update(
        &self,
        ctx: &RequestCtx,
        input: BatchUpdateInput,
    ) -> AppResult<BatchResult> {
        let ids = validate(&input)?;
        let p = &input.patch;
        let mut result = BatchResult::default();
        for id in ids {
            if id.is_empty() {
                result.failures.push(BatchFailure {
                    id,
                    code: ErrorKind::InvalidRequest.code().to_owned(),
                    message: "the issue id must not be empty".to_owned(),
                });
                continue;
            }
            let outcome = if p.delete {
                Box::pin(self.delete_issue(ctx, &id, false))
                    .await
                    .map(|issue| IssueView {
                        extras: Default::default(),
                        previous_identifiers: Vec::new(),
                        issue,
                    })
            } else if p.archive {
                self.archive_issue(ctx, &id).await
            } else {
                Box::pin(self.update_issue(
                    ctx,
                    UpdateIssueInput {
                        id: id.clone(),
                        team: p.team.clone(),
                        state: p.state.clone(),
                        priority: p.priority,
                        assignee: p.assignee.clone(),
                        estimate: p.estimate,
                        add_labels: p.add_labels.clone(),
                        remove_labels: p.remove_labels.clone(),
                        project: p.project.clone(),
                        cycle: p.cycle.clone(),
                        move_sub_issues: p.move_sub_issues,
                        source: HistorySource::User,
                        ..Default::default()
                    },
                ))
                .await
            };
            match outcome {
                Ok(v) => result.updated.push(v),
                Err(e) => result.failures.push(BatchFailure {
                    id,
                    code: e.kind.code().to_owned(),
                    message: e.message,
                }),
            }
        }
        Ok(result)
    }

    /// Archiviert ein Issue (read-only, wiederherstellbar über `restore_issue`).
    pub async fn archive_issue(&self, ctx: &RequestCtx, reference: &str) -> AppResult<IssueView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let found = resolve::issue::resolve_in(&mut *tx, &snap, ctx, reference).await?;
        snap.require_team(ctx, Action::UpdateIssue, found.team_id)?;
        let mut issue = tx
            .issues()
            .get(found.id, true)
            .await?
            .ok_or_else(|| resolve::issue::issue_not_found(reference))?;
        if issue.trashed {
            return Err(invalid(
                "id",
                "issue_trashed",
                "the issue is in the trash; restore it first",
            ));
        }
        if issue.archived_at.is_some() {
            return load_view(&mut *tx, issue, true).await;
        }
        issue.archived_at = Some(now);
        let saved = tx.issues().save(&issue, now).await?;
        let mut d = Delta::default();
        d.flag("archived");
        history::record(
            &mut *tx,
            self.ids.new_uuid(),
            &saved,
            actor_user_id(ctx),
            HistorySource::User,
            &d,
            now,
        )
        .await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::IssueUpdated {
                issue_id: saved.id,
                team_id: saved.team_id,
                changed: vec!["archivedAt".into()],
                mentioned_user_ids: vec![],
                previous: None,
            },
        )
        .await?;
        let view = load_view(&mut *tx, saved, true).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Ob der Aufrufer das Issue abonniert hat (aktives Issue-Abo, kein reines Thread-Abo).
    pub async fn issue_subscribed(&self, ctx: &RequestCtx, reference: &str) -> AppResult<bool> {
        let Some(me) = actor_user_id(ctx) else {
            return Ok(false);
        };
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let found = resolve::issue::resolve_in(&mut *tx, &snap, ctx, reference).await?;
        let subs = tx
            .notifications()
            .list_issue_subscriptions(found.id)
            .await?;
        Ok(subs
            .iter()
            .any(|s| s.user_id == me && s.active && s.thread_comment_id.is_none()))
    }
}
