//! Automationen beim Statuswechsel (docs/03 §3.4 Parent-/Child-Automation, §10.6 Triage-
//! Verantwortung). Wird aus `issue::fields::change_state` aufgerufen und arbeitet in der
//! Transaktion des Aufrufers.
//!
//! Die Kaskade läuft iterativ (Worklist) und ändert jedes Issue pro Transaktion höchstens einmal.

use crate::error::AppResult;
use crate::policy::TeamTree;
use crate::ports::Tx;
use crate::ports::repos::NewEvent;
use crate::ports::repos::issue::{HistorySource, Issue};
use crate::ports::repos::notification::NewNotification;
use crate::ports::repos::team::Team;
use crate::ports::repos::triage::{TriageAction, TriageResponsibility};
use crate::usecases::issue::fields::{change_state_core, first_state_of, user_can_access_team};
use crate::usecases::issue::history::{self, Delta};
use crate::usecases::notification::kinds::{category_of, is_always_priority};
use chrono::{DateTime, Utc};
use lineal_domain::enums::StateType;
use lineal_domain::events::{DomainEvent, EventOrigin};
use lineal_domain::ids::{IssueId, NotificationId, TeamId, UserId};
use lineal_domain::rules::state::TransitionSource;
use serde_json::{Value, json};
use std::collections::{BTreeSet, VecDeque};
use uuid::Uuid;

/// Kategorie ist geschlossen (`completed`, `canceled`, `duplicate`).
pub(crate) fn is_closed(t: StateType) -> bool {
    matches!(
        t,
        StateType::Completed | StateType::Canceled | StateType::Duplicate
    )
}

fn val_uuid(id: Uuid) -> Value {
    Value::String(id.to_string())
}

/// Ein Statuswechsel, aus dem weitere Wechsel folgen können.
struct Changed {
    issue: Issue,
    old: StateType,
}

/// Wertet nach einem Statuswechsel die Team-Automationen aus. `issue` ist das (noch nicht
/// gespeicherte) Issue mit dem neuen Status; `old` die vorherige Kategorie.
pub(crate) async fn after_state_change(
    tx: &mut dyn Tx,
    team: &Team,
    issue: &mut Issue,
    old: StateType,
    now: DateTime<Utc>,
) -> AppResult<()> {
    if old != StateType::Triage && issue.state_type == StateType::Triage {
        run_triage_responsibility(tx, team, issue, now).await?;
    }
    // SLA-Regeln neu auswerten (Wiederöffnen, Statusbedingungen; docs/03 §11.2/§11.4).
    super::sla::evaluate_rules(tx, team, issue, now).await?;
    if is_closed(issue.state_type) && !is_closed(old) {
        cascade(tx, issue, old, now).await?;
    }
    Ok(())
}

async fn cascade(
    tx: &mut dyn Tx,
    issue: &Issue,
    old: StateType,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let mut visited: BTreeSet<IssueId> = BTreeSet::from([issue.id]);
    let mut queue: VecDeque<Changed> = VecDeque::from([Changed {
        issue: issue.clone(),
        old,
    }]);
    while let Some(ev) = queue.pop_front() {
        if !is_closed(ev.issue.state_type) || is_closed(ev.old) {
            continue;
        }
        // 1. Letztes offenes Child geschlossen -> Parent abschließen.
        if let Some(pid) = ev.issue.parent_id
            && !visited.contains(&pid)
            && let Some(done) = close_parent(tx, pid, &ev.issue, now).await?
        {
            visited.insert(done.issue.id);
            queue.push_back(done);
        }
        // 2. Parent abgeschlossen/abgebrochen -> offene Children in dieselbe Kategorie.
        if matches!(
            ev.issue.state_type,
            StateType::Completed | StateType::Canceled
        ) {
            for c in tx.issues().children(ev.issue.id).await? {
                if visited.contains(&c.id) {
                    continue;
                }
                if let Some(done) = close_child(tx, c.id, ev.issue.state_type, now).await? {
                    visited.insert(done.issue.id);
                    queue.push_back(done);
                }
            }
        }
    }
    Ok(())
}

async fn close_parent(
    tx: &mut dyn Tx,
    parent_id: IssueId,
    child: &Issue,
    now: DateTime<Utc>,
) -> AppResult<Option<Changed>> {
    let Some(mut parent) = tx.issues().get(parent_id, true).await? else {
        return Ok(None);
    };
    if parent.trashed || parent.archived_at.is_some() || is_closed(parent.state_type) {
        return Ok(None);
    }
    // Maßgeblich ist das Team des automatisch geänderten Issues (docs/03 §3.4, Annahme 23).
    let Some(team) = tx.teams().get(parent.team_id).await? else {
        return Ok(None);
    };
    if !team.auto_close_parent_issues {
        return Ok(None);
    }
    let children: Vec<Issue> = tx
        .issues()
        .children(parent.id)
        .await?
        .into_iter()
        .filter(|c| c.archived_at.is_none())
        .map(|c| if c.id == child.id { child.clone() } else { c })
        .collect();
    let all_closed = children.iter().all(|c| is_closed(c.state_type));
    let any_completed = children
        .iter()
        .any(|c| c.state_type == StateType::Completed);
    if !(all_closed && any_completed) {
        return Ok(None);
    }
    let Some(target) = first_state_of(tx, parent.team_id, StateType::Completed).await? else {
        return Ok(None);
    };
    let old = parent.state_type;
    apply(tx, &team, &mut parent, &target, now, false).await?;
    Ok(Some(Changed { issue: parent, old }))
}

async fn close_child(
    tx: &mut dyn Tx,
    child_id: IssueId,
    kind: StateType,
    now: DateTime<Utc>,
) -> AppResult<Option<Changed>> {
    let Some(mut child) = tx.issues().get(child_id, true).await? else {
        return Ok(None);
    };
    if child.trashed || child.archived_at.is_some() || is_closed(child.state_type) {
        return Ok(None);
    }
    let Some(team) = tx.teams().get(child.team_id).await? else {
        return Ok(None);
    };
    if !team.auto_close_child_issues {
        return Ok(None);
    }
    let Some(target) = first_state_of(tx, child.team_id, kind).await? else {
        return Ok(None);
    };
    let old = child.state_type;
    apply(tx, &team, &mut child, &target, now, true).await?;
    Ok(Some(Changed { issue: child, old }))
}

/// Wechselt den Status als System-Akteur, speichert, schreibt History und Event.
async fn apply(
    tx: &mut dyn Tx,
    team: &Team,
    issue: &mut Issue,
    target: &crate::ports::repos::state::WorkflowState,
    now: DateTime<Utc>,
    closed_by_parent: bool,
) -> AppResult<()> {
    let before = issue.clone();
    change_state_core(
        tx,
        team,
        issue,
        target,
        TransitionSource::Update,
        None,
        now,
        false,
    )
    .await?;
    let saved = tx.issues().save(issue, now).await?;
    if closed_by_parent {
        tx.triage().set_closed_by_parent(saved.id, true).await?;
    }
    let mut delta = Delta::default();
    delta.set(
        "stateId",
        val_uuid(before.state_id.0),
        val_uuid(saved.state_id.0),
    );
    history::record(
        tx,
        Uuid::now_v7(),
        &saved,
        None,
        HistorySource::Automation,
        &delta,
        now,
    )
    .await?;
    tx.outbox()
        .push(NewEvent::new(
            DomainEvent::IssueUpdated {
                previous: None,
                issue_id: saved.id,
                team_id: saved.team_id,
                changed: vec!["stateId".into()],
                mentioned_user_ids: vec![],
            },
            EventOrigin::System,
        ))
        .await?;
    *issue = saved;
    Ok(())
}

/// Aktuell Verantwortliche eines Teams (docs/03 §10.6 `runTriageResponsibility`, Nutzerliste):
/// Schicht, die `now` enthält (Vertretungen haben Vorrang), sonst die manuelle Auswahl;
/// nur aktive User mit Zugriff auf das Team.
pub(crate) async fn responsible_users(
    tx: &mut dyn Tx,
    tr: &TriageResponsibility,
    team_id: TeamId,
    now: DateTime<Utc>,
) -> AppResult<Vec<UserId>> {
    let candidates: Vec<UserId> = match tr.time_schedule_id {
        Some(schedule) => {
            let entries = tx
                .triage()
                .schedule_entries(schedule, now, None, 500)
                .await?;
            let current: Vec<_> = entries
                .into_iter()
                .filter(|e| e.starts_at <= now && now < e.ends_at)
                .collect();
            let overrides: Vec<UserId> = current
                .iter()
                .filter(|e| e.overridden_user_id.is_some())
                .filter_map(|e| e.user_id)
                .collect();
            if overrides.is_empty() {
                current.iter().filter_map(|e| e.user_id).collect()
            } else {
                overrides
            }
        }
        None => tr.manual_user_ids.clone(),
    };
    if candidates.is_empty() {
        return Ok(candidates);
    }
    let tree = TeamTree::new(tx.teams().list_all().await?);
    let mut out = Vec::new();
    for id in candidates {
        let Some(user) = tx.users().get(id).await? else {
            continue;
        };
        if user.active
            && user_can_access_team(tx, &tree, &user, team_id).await?
            && !out.contains(&id)
        {
            out.push(id);
        }
    }
    Ok(out)
}

/// Führt die Triage-Verantwortung für ein Issue aus, das gerade in Triage eintritt. Ohne
/// Verantwortung oder ohne Verantwortliche benachrichtigt der normale Fan-out die Triage-Abonnenten.
pub(crate) async fn run_triage_responsibility(
    tx: &mut dyn Tx,
    team: &Team,
    issue: &mut Issue,
    now: DateTime<Utc>,
) -> AppResult<()> {
    if !team.triage_enabled {
        return Ok(());
    }
    let Some(tr) = tx.triage().get_responsibility(team.id).await? else {
        return Ok(());
    };
    let users = responsible_users(tx, &tr, team.id, now).await?;
    if users.is_empty() {
        return Ok(());
    }
    let mut delta = Delta::default();
    match tr.action {
        TriageAction::Notify => {
            let kind = "triageResponsibilityIssueAddedToTriage";
            for u in &users {
                let n = NewNotification {
                    id: NotificationId::new_v7(),
                    user_id: *u,
                    kind: kind.into(),
                    category: category_of(kind).into(),
                    target_type: "issue".into(),
                    actor_id: None,
                    issue_id: Some(issue.id),
                    team_id: Some(issue.team_id),
                    project_id: None,
                    project_update_id: None,
                    project_milestone_id: None,
                    initiative_id: None,
                    initiative_update_id: None,
                    document_id: None,
                    comment_id: None,
                    parent_comment_id: None,
                    reaction_emoji: None,
                    is_priority: is_always_priority(kind),
                    dedup_key: format!(
                        "triage:{}:{}:{}:{kind}",
                        issue.id,
                        u,
                        issue.started_triage_at.unwrap_or(now).timestamp_millis()
                    ),
                    now,
                };
                if tx.notifications().insert_notification(&n).await? {
                    tx.outbox()
                        .push(NewEvent::new(
                            DomainEvent::NotificationCreated {
                                notification_id: n.id,
                                user_id: *u,
                            },
                            EventOrigin::System,
                        ))
                        .await?;
                }
            }
            delta.set(
                "triageResponsibilityNotifiedUsers",
                Value::Null,
                json!(users.iter().map(|u| u.to_string()).collect::<Vec<_>>()),
            );
        }
        TriageAction::Assign => {
            if issue.assignee_id.is_some() {
                return Ok(());
            }
            let chosen = if tr.time_schedule_id.is_some() {
                users[0]
            } else {
                let idx = tx.triage().advance_assignment_index(team.id, now).await?;
                let n = users.len() as i64;
                users[(i64::from(idx)).rem_euclid(n) as usize]
            };
            issue.assignee_id = Some(chosen);
            tx.issues()
                .subscribe(Uuid::now_v7(), issue.id, chosen, "assignee", now)
                .await?;
            delta.flag("triageResponsibilityAutoAssigned");
        }
    }
    history::record(
        tx,
        Uuid::now_v7(),
        issue,
        None,
        HistorySource::Automation,
        &delta,
        now,
    )
    .await
}
