//! usecases – Triage (docs/03 §10): accept, decline, duplicate, snooze, move, Triage-Verantwortung
//! und Schichtpläne. Die Automationen beim Statuswechsel stehen in [`automation`], die Jobs
//! (Auto-Close, Auto-Archive, Purge, SLA, Rotation, Snooze-Wiedervorlage) in [`jobs`], die
//! SLA-Regeln in [`sla`].
//!
//! Sicherheit: jede Aktion löst das Issue über `resolve::issue::resolve_in` (private Teams →
//! `not_found`), prüft `Action::TriageIssue` (Scope `write`, Team-Zugriff) und – bei Gegenstücken
//! (Duplikat-Ziel, Zielteam) – das Schreibrecht auch dort.

pub mod automation;
pub mod jobs;
pub mod sla;

use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot, team_not_found};
use crate::ports::Tx;
use crate::ports::repos::NewEvent;
use crate::ports::repos::comment::{NewComment, ParentRef};
use crate::ports::repos::issue::{HistorySource, Issue};
use crate::ports::repos::team::Team;
use crate::ports::repos::triage::{
    NewResponsibility, NewScheduleEntry, ScheduleEntry, TimeSchedule, TriageAction,
    TriageResponsibility,
};
use crate::resolve;
use crate::resolve::team::Visibility;
use crate::resolve::user::{actor_user_id, can_see_user};
use crate::usecases::issue::fields::{
    change_state, default_state, first_state_of, user_can_access_team,
};
use crate::usecases::issue::history::{self, Delta};
use crate::usecases::issue::{IssueView, RelationChanges, UpdateIssueInput};
use chrono::{DateTime, Duration, Utc};
use lineal_domain::enums::{Priority, StateType};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{CommentId, IssueId, UserId};
use lineal_domain::rules::markdown;
use lineal_domain::rules::state::TransitionSource;
use serde_json::{Value, json};
use uuid::Uuid;

/// Höchste Länge eines Triage-Kommentars (wie normale Kommentare).
const MAX_COMMENT_CHARS: usize = 100_000;
/// Längste Snooze-Dauer.
const MAX_SNOOZE_DAYS: i64 = 366;
/// Obergrenzen für Schichtpläne.
const MAX_SCHEDULE_ENTRIES: usize = 1000;
const MAX_MANUAL_USERS: usize = 50;
const MAX_SHIFT_DAYS: i64 = 366;

fn invalid(field: &str, reason: &str, msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg).with_details(json!({ "field": field, "reason": reason }))
}

/// Eingabe von Accept.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriageAcceptInput {
    pub issue: String,
    /// `backlog`/`unstarted`/`started`-Status; Default `defaultIssueState`.
    pub state: Option<String>,
    /// Priorität setzen (nötig, wenn das Team `requirePriorityToLeaveTriage` verlangt).
    pub priority: Option<i64>,
    pub comment: Option<String>,
    pub source: HistorySource,
}

/// Eingabe von Decline.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriageDeclineInput {
    pub issue: String,
    /// `canceled`-Status; Default: erster `canceled`-Status.
    pub state: Option<String>,
    pub priority: Option<i64>,
    pub comment: Option<String>,
    pub source: HistorySource,
}

/// Eingabe von „Als Duplikat markieren“.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriageDuplicateInput {
    pub issue: String,
    pub duplicate_of: String,
    pub comment: Option<String>,
    pub source: HistorySource,
}

/// Eingabe von Snooze.
#[derive(Debug, Clone, PartialEq)]
pub struct TriageSnoozeInput {
    pub issue: String,
    pub until: DateTime<Utc>,
    pub source: HistorySource,
}

/// Eingabe von „Move to team“.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriageMoveInput {
    pub issue: String,
    pub team: String,
    pub source: HistorySource,
}

/// Kurzform eines Users in Antworten (nie mit E-Mail).
#[derive(Debug, Clone, PartialEq)]
pub struct UserBrief {
    pub id: UserId,
    pub name: String,
}

/// Eine Schicht der Rotation.
#[derive(Debug, Clone, PartialEq)]
pub struct ShiftView {
    pub user: UserBrief,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

/// Antwort von `get_triage_responsibility`.
#[derive(Debug, Clone, PartialEq)]
pub struct TriageInfo {
    pub team: Team,
    pub triage_enabled: bool,
    pub action: Option<TriageAction>,
    pub current_user: Option<UserBrief>,
    pub schedule: Option<TimeSchedule>,
    pub upcoming_shifts: Vec<ShiftView>,
}

/// Eingabe von `set_triage_responsibility`: genau eine Quelle (`users` oder `schedule`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SetTriageResponsibilityInput {
    pub team: String,
    pub action: Option<TriageAction>,
    pub users: Vec<String>,
    pub schedule: Option<String>,
}

/// Eine Schicht beim Anlegen eines Schichtplans.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleEntryInput {
    pub user: Option<String>,
    pub user_email: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub overridden_user: Option<String>,
}

/// Aktionen, die `IssueTriaged` meldet.
mod action {
    pub const ACCEPT: &str = "accept";
    pub const DECLINE: &str = "decline";
    pub const DUPLICATE: &str = "duplicate";
    pub const SNOOZE: &str = "snooze";
    pub const UNSNOOZE: &str = "unsnooze";
    pub const MOVE: &str = "move";
}

fn val_uuid(id: Uuid) -> Value {
    Value::String(id.to_string())
}

fn val_ts(v: Option<DateTime<Utc>>) -> Value {
    v.map_or(Value::Null, |t| {
        Value::String(t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
    })
}

/// History-Delta und geänderte Feldnamen eines Triage-Wechsels.
fn delta_of(old: &Issue, new: &Issue) -> (Delta, Vec<String>) {
    let mut d = Delta::default();
    d.set(
        "stateId",
        val_uuid(old.state_id.0),
        val_uuid(new.state_id.0),
    );
    d.set(
        "priority",
        json!(old.priority.value()),
        json!(new.priority.value()),
    );
    d.set(
        "assigneeId",
        old.assignee_id.map_or(Value::Null, |u| val_uuid(u.0)),
        new.assignee_id.map_or(Value::Null, |u| val_uuid(u.0)),
    );
    d.set(
        "snoozedUntilAt",
        val_ts(old.snoozed_until_at),
        val_ts(new.snoozed_until_at),
    );
    d.set(
        "cycleId",
        old.cycle_id.map_or(Value::Null, |u| val_uuid(u.0)),
        new.cycle_id.map_or(Value::Null, |u| val_uuid(u.0)),
    );
    let changed = d.changes.keys().cloned().collect();
    (d, changed)
}

fn priority_of(value: i64) -> AppResult<Priority> {
    u8::try_from(value)
        .ok()
        .and_then(|v| Priority::try_from(v).ok())
        .ok_or_else(|| {
            invalid(
                "priority",
                "out_of_range",
                "priority must be between 0 and 4",
            )
        })
}

fn clean_comment(raw: &str) -> AppResult<Option<String>> {
    let cleaned = markdown::sanitize(raw.trim());
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return Ok(None);
    }
    if cleaned.chars().count() > MAX_COMMENT_CHARS {
        return Err(AppError::limit_exceeded(format!(
            "comment must not exceed {MAX_COMMENT_CHARS} characters"
        ))
        .with_details(json!({ "field": "comment", "max": MAX_COMMENT_CHARS })));
    }
    Ok(Some(cleaned.to_owned()))
}

/// Gemeinsamer Anfang jeder Triage-Aktion: Issue auflösen (sichtbar, sonst `not_found`), Recht
/// prüfen, Zeile sperren, Zustand prüfen.
async fn prelude(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    reference: &str,
    require_triage: bool,
) -> AppResult<(Issue, Team)> {
    let found = resolve::issue::resolve_in(tx, snap, ctx, reference).await?;
    snap.require_team(ctx, Action::TriageIssue, found.team_id)?;
    let issue = tx
        .issues()
        .get(found.id, true)
        .await?
        .ok_or_else(|| resolve::issue::issue_not_found(reference))?;
    if issue.trashed {
        return Err(invalid(
            "issue",
            "issue_trashed",
            "the issue is in the trash; restore it first",
        ));
    }
    if issue.archived_at.is_some() {
        return Err(invalid(
            "issue",
            "issue_archived",
            "archived issues are read-only",
        ));
    }
    if require_triage && issue.state_type != StateType::Triage {
        return Err(invalid(
            "issue",
            "not_in_triage",
            "the issue is not in triage",
        ));
    }
    let team = snap
        .tree
        .get(issue.team_id)
        .cloned()
        .ok_or_else(team_not_found)?;
    Ok((issue, team))
}

async fn view_of(tx: &mut dyn Tx, issue: Issue) -> AppResult<IssueView> {
    let mut extras = tx.issues().extras(&[issue.id]).await?;
    Ok(IssueView {
        extras: extras.remove(&issue.id).unwrap_or_default(),
        issue,
        previous_identifiers: Vec::new(),
    })
}

async fn emit(tx: &mut dyn Tx, ctx: &RequestCtx, event: DomainEvent) -> AppResult<()> {
    tx.outbox().push(NewEvent::from_ctx(event, ctx)).await?;
    Ok(())
}

/// Speichert, protokolliert und meldet einen Triage-Wechsel (`IssueUpdated` + `IssueTriaged`).
#[allow(clippy::too_many_arguments)]
async fn persist(
    tx: &mut dyn Tx,
    ctx: &RequestCtx,
    ids: &dyn crate::ports::IdGen,
    old: &Issue,
    issue: &Issue,
    source: HistorySource,
    act: &str,
    now: DateTime<Utc>,
) -> AppResult<Issue> {
    let saved = tx.issues().save(issue, now).await?;
    let (delta, mut changed) = delta_of(old, &saved);
    history::record(
        tx,
        ids.new_uuid(),
        &saved,
        actor_user_id(ctx),
        source,
        &delta,
        now,
    )
    .await?;
    if old.state_id != saved.state_id {
        changed.push("triagedAt".into());
    }
    let previous = lineal_domain::events::IssuePrevious::of(
        &changed,
        old.assignee_id,
        old.state_type.as_str(),
    );
    emit(
        tx,
        ctx,
        DomainEvent::IssueUpdated {
            previous,
            issue_id: saved.id,
            team_id: saved.team_id,
            changed,
            mentioned_user_ids: vec![],
        },
    )
    .await?;
    emit(
        tx,
        ctx,
        DomainEvent::IssueTriaged {
            issue_id: saved.id,
            team_id: saved.team_id,
            action: act.to_owned(),
        },
    )
    .await?;
    Ok(saved)
}

/// Legt den optionalen Begründungskommentar in derselben Transaktion an.
async fn add_comment(
    tx: &mut dyn Tx,
    ctx: &RequestCtx,
    ids: &dyn crate::ports::IdGen,
    issue: IssueId,
    body: Option<&str>,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let Some(body) = body else { return Ok(()) };
    let Some(body) = clean_comment(body)? else {
        return Ok(());
    };
    let author = actor_user_id(ctx);
    let comment = tx
        .comments()
        .insert(&NewComment {
            id: CommentId(ids.new_uuid()),
            user_id: author,
            body,
            parent: ParentRef::Issue(issue),
            parent_id: None,
            quoted_text: None,
            now,
        })
        .await?;
    if let Some(uid) = author
        && tx
            .comments()
            .subscribe(issue, uid, None, "comment", now)
            .await?
    {
        emit(
            tx,
            ctx,
            DomainEvent::IssueSubscriptionCreated {
                issue_id: issue,
                user_id: uid,
            },
        )
        .await?;
    }
    emit(
        tx,
        ctx,
        DomainEvent::CommentCreated {
            comment_id: comment.id,
            mentioned_user_ids: vec![],
        },
    )
    .await
}

impl App {
    /// Accept: wechselt auf `defaultIssueState` oder einen gewählten Status
    /// (`backlog`/`unstarted`/`started`) und legt optional einen Kommentar an (docs/03 §10.3).
    pub async fn triage_accept(
        &self,
        ctx: &RequestCtx,
        input: TriageAcceptInput,
    ) -> AppResult<IssueView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let (mut issue, team) = prelude(&mut *tx, &snap, ctx, &input.issue, true).await?;
        let old = issue.clone();
        let target = match &input.state {
            Some(r) => {
                let s = resolve::state::resolve(&mut *tx, &team, r).await?;
                if !matches!(
                    s.state_type,
                    StateType::Backlog | StateType::Unstarted | StateType::Started
                ) {
                    return Err(invalid(
                        "state",
                        "invalid_accept_state",
                        "accept needs a backlog, unstarted or started status",
                    ));
                }
                s
            }
            None => default_state(&mut *tx, &team).await?,
        };
        if let Some(p) = input.priority {
            issue.priority = priority_of(p)?;
        }
        change_state(
            &mut *tx,
            &team,
            &mut issue,
            &target,
            TransitionSource::Update,
            None,
            now,
            true,
        )
        .await?;
        let saved = persist(
            &mut *tx,
            ctx,
            self.ids.as_ref(),
            &old,
            &issue,
            input.source,
            action::ACCEPT,
            now,
        )
        .await?;
        add_comment(
            &mut *tx,
            ctx,
            self.ids.as_ref(),
            saved.id,
            input.comment.as_deref(),
            now,
        )
        .await?;
        let view = view_of(&mut *tx, saved).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Decline: wechselt auf den ersten `canceled`-Status (oder einen gewählten) und legt
    /// optional einen Begründungskommentar an.
    pub async fn triage_decline(
        &self,
        ctx: &RequestCtx,
        input: TriageDeclineInput,
    ) -> AppResult<IssueView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let (mut issue, team) = prelude(&mut *tx, &snap, ctx, &input.issue, true).await?;
        let old = issue.clone();
        let target = match &input.state {
            Some(r) => {
                let s = resolve::state::resolve(&mut *tx, &team, r).await?;
                if s.state_type != StateType::Canceled {
                    return Err(invalid(
                        "state",
                        "invalid_decline_state",
                        "decline needs a canceled status",
                    ));
                }
                s
            }
            None => first_state_of(&mut *tx, team.id, StateType::Canceled)
                .await?
                .ok_or_else(|| {
                    invalid(
                        "state",
                        "no_canceled_state",
                        "the team has no canceled status",
                    )
                })?,
        };
        if let Some(p) = input.priority {
            issue.priority = priority_of(p)?;
        }
        change_state(
            &mut *tx,
            &team,
            &mut issue,
            &target,
            TransitionSource::Update,
            None,
            now,
            true,
        )
        .await?;
        let saved = persist(
            &mut *tx,
            ctx,
            self.ids.as_ref(),
            &old,
            &issue,
            input.source,
            action::DECLINE,
            now,
        )
        .await?;
        add_comment(
            &mut *tx,
            ctx,
            self.ids.as_ref(),
            saved.id,
            input.comment.as_deref(),
            now,
        )
        .await?;
        let view = view_of(&mut *tx, saved).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// „Als Duplikat markieren“ aus der Triage: Duplicate-Status, Relation, Anhänge wandern
    /// zum kanonischen Issue (docs/03 §4.3, §10.3).
    pub async fn triage_duplicate(
        &self,
        ctx: &RequestCtx,
        input: TriageDuplicateInput,
    ) -> AppResult<IssueView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let (issue, _team) = prelude(&mut *tx, &snap, ctx, &input.issue, true).await?;
        // Gegenstück: sichtbar und beschreibbar (die Anhänge wandern dorthin).
        let canonical =
            resolve::issue::resolve_in(&mut *tx, &snap, ctx, &input.duplicate_of).await?;
        if canonical.trashed {
            return Err(resolve::issue::issue_not_found(&input.duplicate_of));
        }
        snap.require_team(ctx, Action::UpdateIssue, canonical.team_id)?;
        if canonical.id == issue.id {
            return Err(invalid(
                "duplicateOf",
                "self_relation",
                "an issue cannot be a duplicate of itself",
            ));
        }
        let view = self
            .update_issue_in(
                &mut *tx,
                ctx,
                UpdateIssueInput {
                    id: issue.id.to_string(),
                    relations: RelationChanges {
                        duplicate_of: Some(Some(canonical.id.to_string())),
                        ..Default::default()
                    },
                    source: input.source,
                    ..Default::default()
                },
            )
            .await?;
        tx.triage().move_attachments(issue.id, canonical.id).await?;
        add_comment(
            &mut *tx,
            ctx,
            self.ids.as_ref(),
            issue.id,
            input.comment.as_deref(),
            now,
        )
        .await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::IssueTriaged {
                issue_id: issue.id,
                team_id: view.issue.team_id,
                action: action::DUPLICATE.to_owned(),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Snooze: das Issue verschwindet für alle aus der Triage-Liste, bis `until` erreicht ist.
    pub async fn triage_snooze(
        &self,
        ctx: &RequestCtx,
        input: TriageSnoozeInput,
    ) -> AppResult<IssueView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let (mut issue, _team) = prelude(&mut *tx, &snap, ctx, &input.issue, true).await?;
        let latest = now
            .checked_add_signed(Duration::days(MAX_SNOOZE_DAYS))
            .unwrap_or(now);
        if input.until <= now || input.until > latest {
            return Err(invalid(
                "until",
                "invalid_snooze_time",
                "the snooze time must be in the future and within one year",
            ));
        }
        let old = issue.clone();
        issue.snoozed_until_at = Some(input.until);
        issue.snoozed_by_id = actor_user_id(ctx);
        let saved = persist(
            &mut *tx,
            ctx,
            self.ids.as_ref(),
            &old,
            &issue,
            input.source,
            action::SNOOZE,
            now,
        )
        .await?;
        let view = view_of(&mut *tx, saved).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Beendet ein Snooze vorzeitig.
    pub async fn triage_unsnooze(
        &self,
        ctx: &RequestCtx,
        issue_ref: &str,
        source: HistorySource,
    ) -> AppResult<IssueView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let (mut issue, _team) = prelude(&mut *tx, &snap, ctx, issue_ref, true).await?;
        if issue.snoozed_until_at.is_none() {
            let view = view_of(&mut *tx, issue).await?;
            return Ok(view);
        }
        let old = issue.clone();
        issue.snoozed_until_at = None;
        issue.snoozed_by_id = None;
        let saved = persist(
            &mut *tx,
            ctx,
            self.ids.as_ref(),
            &old,
            &issue,
            source,
            action::UNSNOOZE,
            now,
        )
        .await?;
        let view = view_of(&mut *tx, saved).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// „Move to team“: Teamwechsel mit erneuter Triage-Regel im Zielteam (docs/03 §10.2).
    pub async fn triage_move(
        &self,
        ctx: &RequestCtx,
        input: TriageMoveInput,
    ) -> AppResult<IssueView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let (issue, _team) = prelude(&mut *tx, &snap, ctx, &input.issue, true).await?;
        // `update_issue_in` prüft Sichtbarkeit und Schreibrecht des Zielteams.
        let view = self
            .update_issue_in(
                &mut *tx,
                ctx,
                UpdateIssueInput {
                    id: issue.id.to_string(),
                    team: Some(input.team),
                    source: input.source,
                    ..Default::default()
                },
            )
            .await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::IssueTriaged {
                issue_id: issue.id,
                team_id: view.issue.team_id,
                action: action::MOVE.to_owned(),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Triage-Liste eines Teams (nicht gesnoozte; älteste zuerst).
    pub async fn list_triage_issues(
        &self,
        ctx: &RequestCtx,
        team_ref: &str,
        include_snoozed: bool,
        limit: i64,
    ) -> AppResult<Vec<IssueView>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, team_ref, Visibility::Content).await?;
        if !team.triage_enabled {
            return Ok(Vec::new());
        }
        let ids = tx
            .triage()
            .triage_issues(
                team.id,
                self.clock.now(),
                include_snoozed,
                limit.clamp(1, 250),
            )
            .await?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(issue) = tx.issues().get(id, false).await? {
                out.push(view_of(&mut *tx, issue).await?);
            }
        }
        Ok(out)
    }

    /// `get_triage_responsibility`: aktueller Verantwortlicher, Aktion und kommende Schichten.
    pub async fn get_triage_responsibility(
        &self,
        ctx: &RequestCtx,
        team_ref: &str,
        user_ref: Option<&str>,
        limit: i64,
    ) -> AppResult<TriageInfo> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, team_ref, Visibility::Content).await?;
        let now = self.clock.now();
        let mut info = TriageInfo {
            triage_enabled: team.triage_enabled,
            team: team.clone(),
            action: None,
            current_user: None,
            schedule: None,
            upcoming_shifts: Vec::new(),
        };
        if !team.triage_enabled {
            return Ok(info);
        }
        let filter_user = match user_ref {
            Some(r) => Some(resolve::user::resolve_in(&mut *tx, &snap, ctx, r).await?),
            None => None,
        };
        let Some(tr) = tx.triage().get_responsibility(team.id).await? else {
            return Ok(info);
        };
        info.action = Some(tr.action);
        let users = automation::responsible_users(&mut *tx, &tr, team.id, now).await?;
        let current = match (tr.time_schedule_id, users.is_empty()) {
            (_, true) => None,
            (Some(_), false) => users.first().copied(),
            (None, false) => {
                let n = users.len() as i64;
                users
                    .get((i64::from(tr.assignment_index)).rem_euclid(n) as usize)
                    .copied()
            }
        };
        if let Some(id) = current {
            info.current_user = brief(&mut *tx, &snap, ctx, id).await?;
        }
        if let Some(schedule) = tr.time_schedule_id {
            info.schedule = tx.triage().get_schedule(schedule).await?;
            let entries = tx
                .triage()
                .schedule_entries(
                    schedule,
                    now,
                    filter_user.as_ref().map(|u| u.id),
                    limit.clamp(1, 50),
                )
                .await?;
            for e in entries {
                let Some(uid) = e.user_id else { continue };
                if let Some(user) = brief(&mut *tx, &snap, ctx, uid).await? {
                    info.upcoming_shifts.push(ShiftView {
                        user,
                        starts_at: e.starts_at,
                        ends_at: e.ends_at,
                    });
                }
            }
        }
        Ok(info)
    }

    /// Setzt die Triage-Verantwortung eines Teams (genau eine Quelle: Personen oder Schichtplan).
    pub async fn set_triage_responsibility(
        &self,
        ctx: &RequestCtx,
        input: SetTriageResponsibilityInput,
    ) -> AppResult<TriageResponsibility> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, &input.team, Visibility::Content).await?;
        snap.require_team(ctx, Action::ManageTriageSettings, team.id)?;
        let has_users = !input.users.is_empty();
        if has_users == input.schedule.is_some() {
            return Err(invalid(
                "users",
                "exactly_one_source",
                "provide either users or a schedule",
            ));
        }
        if input.users.len() > MAX_MANUAL_USERS {
            return Err(AppError::limit_exceeded("too many users")
                .with_details(json!({ "field": "users", "max": MAX_MANUAL_USERS })));
        }
        let mut manual = Vec::new();
        for r in &input.users {
            let u = resolve::user::resolve_in(&mut *tx, &snap, ctx, r).await?;
            if !u.active || !user_can_access_team(&mut *tx, &snap.tree, &u, team.id).await? {
                return Err(invalid(
                    "users",
                    "user_has_no_access",
                    "the user has no access to this team",
                ));
            }
            if !manual.contains(&u.id) {
                manual.push(u.id);
            }
        }
        let schedule = match &input.schedule {
            Some(r) => {
                let id = Uuid::parse_str(r.trim()).map_err(|_| {
                    AppError::not_found("Could not find referenced TimeSchedule.")
                        .with_details(json!({ "reference": r, "type": "TimeSchedule" }))
                })?;
                let s = tx.triage().get_schedule(id).await?.ok_or_else(|| {
                    AppError::not_found("Could not find referenced TimeSchedule.")
                        .with_details(json!({ "reference": r, "type": "TimeSchedule" }))
                })?;
                Some(s.id)
            }
            None => None,
        };
        let saved = tx
            .triage()
            .upsert_responsibility(&NewResponsibility {
                id: self.ids.new_uuid(),
                team_id: team.id,
                action: input.action.unwrap_or(TriageAction::Notify),
                manual_user_ids: manual,
                time_schedule_id: schedule,
                now: self.clock.now(),
            })
            .await?;
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        tx.commit().await?;
        Ok(saved)
    }

    /// Entfernt die Triage-Verantwortung eines Teams.
    pub async fn delete_triage_responsibility(
        &self,
        ctx: &RequestCtx,
        team_ref: &str,
    ) -> AppResult<bool> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, team_ref, Visibility::Content).await?;
        snap.require_team(ctx, Action::ManageTriageSettings, team.id)?;
        let removed = tx.triage().delete_responsibility(team.id).await?;
        if removed {
            emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        }
        tx.commit().await?;
        Ok(removed)
    }

    /// Legt einen Schichtplan an (Admin; docs/01 §2.7). Schichten überlappen nicht, außer
    /// Vertretungen (`overridden_user`).
    pub async fn create_time_schedule(
        &self,
        ctx: &RequestCtx,
        name: &str,
        entries: Vec<ScheduleEntryInput>,
    ) -> AppResult<(TimeSchedule, Vec<ScheduleEntry>)> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::UpdateWorkspaceSettings)?;
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 120 {
            return Err(invalid(
                "name",
                "invalid_length",
                "name must have 1 to 120 characters",
            ));
        }
        if entries.len() > MAX_SCHEDULE_ENTRIES {
            return Err(AppError::limit_exceeded("too many schedule entries")
                .with_details(json!({ "field": "entries", "max": MAX_SCHEDULE_ENTRIES })));
        }
        let now = self.clock.now();
        let schedule_id = self.ids.new_uuid();
        let mut new_entries: Vec<NewScheduleEntry> = Vec::with_capacity(entries.len());
        for e in &entries {
            if e.ends_at <= e.starts_at || e.ends_at - e.starts_at > Duration::days(MAX_SHIFT_DAYS)
            {
                return Err(invalid(
                    "entries",
                    "invalid_shift",
                    "a shift must end after it starts and last at most one year",
                ));
            }
            let user = match &e.user {
                Some(r) => Some(resolve::user::resolve_in(&mut *tx, &snap, ctx, r).await?.id),
                None => None,
            };
            let overridden = match &e.overridden_user {
                Some(r) => Some(resolve::user::resolve_in(&mut *tx, &snap, ctx, r).await?.id),
                None => None,
            };
            let email = e
                .user_email
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if user.is_none() && email.is_none() {
                return Err(invalid(
                    "entries",
                    "user_required",
                    "a shift needs a user or an email",
                ));
            }
            new_entries.push(NewScheduleEntry {
                id: self.ids.new_uuid(),
                schedule_id,
                user_id: user,
                user_email: email.map(str::to_owned),
                starts_at: e.starts_at,
                ends_at: e.ends_at,
                overridden_user_id: overridden,
            });
        }
        for (i, a) in new_entries.iter().enumerate() {
            for b in new_entries.iter().skip(i + 1) {
                let overlap = a.starts_at < b.ends_at && b.starts_at < a.ends_at;
                if overlap && a.overridden_user_id.is_none() && b.overridden_user_id.is_none() {
                    return Err(invalid(
                        "entries",
                        "overlapping_shifts",
                        "shifts of one schedule must not overlap unless one is a substitution",
                    ));
                }
            }
        }
        tx.triage().insert_schedule(schedule_id, name, now).await?;
        for e in &new_entries {
            tx.triage().insert_schedule_entry(e, now).await?;
        }
        let stored = tx
            .triage()
            .schedule_entries(
                schedule_id,
                DateTime::<Utc>::UNIX_EPOCH,
                None,
                MAX_SCHEDULE_ENTRIES as i64,
            )
            .await?;
        tx.commit().await?;
        Ok((
            TimeSchedule {
                id: schedule_id,
                name: name.to_owned(),
            },
            stored,
        ))
    }
}

/// Kurzform eines Users, wenn der Akteur ihn sehen darf (Gäste sehen nur gemeinsame Teams).
async fn brief(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    id: UserId,
) -> AppResult<Option<UserBrief>> {
    let Some(u) = tx.users().get(id).await? else {
        return Ok(None);
    };
    if !can_see_user(tx, snap, ctx, &u).await? {
        return Ok(None);
    }
    Ok(Some(UserBrief {
        id: u.id,
        name: u.name,
    }))
}
