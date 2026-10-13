//! usecases – Cycles (docs/04 §1): Einstellungen, Generierung, Verschieben, Wechsel-Lauf mit
//! Rollover und Snapshots, Vererbung an Sub-Teams.
//!
//! Die Rechenlogik steht in [`engine`]. Dieses Modul stellt Sichten und die öffentlichen
//! Use-Cases bereit. Cycles werden nie einzeln angelegt, sondern ausschließlich generiert.

pub mod engine;

use super::project::ProjectScope;
use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::cycle::{Cycle, CycleTeamRef};
use crate::ports::repos::team::{Team, TeamPatch};
use crate::resolve;
use crate::resolve::team::Visibility;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use engine::{Env, inherits, team_tz};
use lineal_domain::enums::StateType;
use lineal_domain::ids::{OrgId, TeamId};
use lineal_domain::rules::cycle_calendar::{local_midnight_to_utc, utc_to_local_date};
use serde_json::json;
use std::collections::{BTreeMap, HashSet};

/// Obergrenze für `cycleShiftAll` (Tage, je Richtung).
const MAX_SHIFT_DAYS: i64 = 3660;

/// `type`-Filter von `list_cycles`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleKind {
    Current,
    Previous,
    Next,
}

/// Ein Cycle mit berechneten Flags (docs/04 §1.10) und Fortschritt.
#[derive(Debug, Clone, PartialEq)]
pub struct CycleView {
    pub cycle: Cycle,
    pub team: Team,
    /// 0..1; nach dem Abschluss der eingefrorene Wert.
    pub progress: f64,
    /// Scope und Zähler der aktuellen Issue-Liste.
    pub scope: ProjectScope,
    pub is_active: bool,
    pub is_past: bool,
    pub is_future: bool,
    pub is_next: bool,
    pub is_previous: bool,
    /// Kapazitätsschätzung (docs/04 §1.9): Velocity der drei zuletzt abgeschlossenen Cycles, sonst
    /// `Mitglieder × defaultIssueEstimate × cycleDuration`. Nur für laufende und künftige Cycles.
    pub capacity: Option<f64>,
}

/// Kapazitätsschätzung eines Teams (docs/04 §1.9): Mittel von `completedScopeHistory[last]` der
/// (bis zu) drei zuletzt abgeschlossenen Cycles mit Historie; ohne Historie die grobe Schätzung
/// `Mitglieder × defaultIssueEstimate × cycleDuration` (Wochen). `None`, wenn beides 0 ergibt.
pub fn capacity_estimate(cycles: &[Cycle], members: usize, team: &Team) -> Option<f64> {
    let mut past: Vec<&Cycle> = cycles
        .iter()
        .filter(|c| c.is_past() && !c.completed_scope_history.is_empty())
        .collect();
    past.sort_by_key(|c| std::cmp::Reverse(c.completed_at));
    past.truncate(3);
    let est = if past.is_empty() {
        members as f64 * f64::from(team.default_issue_estimate) * f64::from(team.cycle_duration)
    } else {
        past.iter()
            .filter_map(|c| c.completed_scope_history.last().copied())
            .sum::<f64>()
            / past.len() as f64
    };
    (est > 0.0).then_some(est)
}

/// `existingActiveIssues` beim Aktivieren von „Active issues“.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExistingActiveIssues {
    MoveToBacklog,
    AddToCycle,
}

/// Cycle-Einstellungen eines Teams (docs/04 §1.2); `None` = unverändert.
#[derive(Debug, Clone, Default)]
pub struct CycleSettingsInput {
    pub cycles_enabled: Option<bool>,
    pub duration: Option<i32>,
    pub cooldown: Option<i32>,
    pub start_day: Option<i32>,
    pub upcoming_count: Option<i32>,
    pub auto_assign_started: Option<bool>,
    pub auto_assign_completed: Option<bool>,
    pub lock_to_active: Option<bool>,
    pub timezone: Option<String>,
    /// Startdatum des ersten Cycles (lokales Kalenderdatum) beim Einschalten.
    pub first_start_date: Option<NaiveDate>,
    pub existing_active_issues: Option<ExistingActiveIssues>,
}

/// Änderungen an einem einzelnen Cycle.
#[derive(Debug, Clone, Default)]
pub struct UpdateCycleInput {
    pub id: String,
    pub name: Option<Option<String>>,
    pub description: Option<Option<String>>,
    /// Nur künftige Cycles.
    pub starts_at: Option<DateTime<Utc>>,
    /// Künftige Cycles und der aktive Cycle.
    pub ends_at: Option<DateTime<Utc>>,
}

/// Ergebnis eines Wartungslaufs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CycleRunStats {
    pub teams: usize,
    pub errors: Vec<String>,
}

fn invalid(field: &str, msg: &str) -> AppError {
    AppError::invalid_request(msg.to_owned()).with_details(json!({ "field": field }))
}

fn flags(cycles: &[Cycle], c: &Cycle, now: DateTime<Utc>) -> (bool, bool, bool, bool, bool) {
    let next = cycles
        .iter()
        .filter(|x| x.is_future(now) && x.completed_at.is_none() && x.archived_at.is_none())
        .min_by_key(|x| x.starts_at)
        .map(|x| x.id);
    let previous = cycles
        .iter()
        .filter(|x| x.is_past() && x.archived_at.is_none())
        .max_by_key(|x| x.ends_at)
        .map(|x| x.id);
    (
        c.is_active(now),
        c.is_past(),
        c.is_future(now),
        next == Some(c.id),
        previous == Some(c.id),
    )
}

async fn build_views(
    tx: &mut dyn Tx,
    team: &Team,
    cycles: &[Cycle],
    selection: &[Cycle],
    now: DateTime<Utc>,
) -> AppResult<Vec<CycleView>> {
    let mut out = Vec::with_capacity(selection.len());
    let mut members: Option<usize> = None;
    for c in selection {
        let rows = tx.cycles().issue_rows(c.id).await?;
        let scope = ProjectScope::from_rows(&rows);
        let (is_active, is_past, is_future, is_next, is_previous) = flags(cycles, c, now);
        let capacity = if c.is_past() {
            None
        } else {
            let m = match members {
                Some(m) => m,
                None => {
                    let m = tx.teams().list_memberships(team.id).await?.len();
                    members = Some(m);
                    m
                }
            };
            capacity_estimate(cycles, m, team)
        };
        out.push(CycleView {
            capacity,
            progress: if c.is_past() {
                c.progress.unwrap_or(scope.progress)
            } else {
                scope.progress
            },
            scope,
            is_active,
            is_past,
            is_future,
            is_next,
            is_previous,
            team: team.clone(),
            cycle: c.clone(),
        });
    }
    Ok(out)
}

/// Auto-Zuordnung beim Statuswechsel (docs/04 §1.6 `onIssueStateChanged`): liefert den Cycle, dem
/// ein Issue ohne Cycle zugeordnet werden soll. Für WP06 (Issue-Anlage und Statuswechsel).
pub async fn auto_cycle_for_state(
    tx: &mut dyn Tx,
    team: &Team,
    new_type: StateType,
    now: DateTime<Utc>,
) -> AppResult<Option<Cycle>> {
    if !team.cycles_enabled {
        return Ok(None);
    }
    let cycles = tx.cycles().list_for_team(team.id, false).await?;
    let active = cycles.iter().find(|c| c.is_active(now)).cloned();
    let next = cycles
        .iter()
        .filter(|c| c.is_future(now) && c.completed_at.is_none())
        .min_by_key(|c| c.starts_at)
        .cloned();
    let prev = cycles
        .iter()
        .filter(|c| c.is_past())
        .max_by_key(|c| c.ends_at)
        .cloned();
    Ok(match new_type {
        StateType::Started if team.cycle_issue_auto_assign_started || team.cycle_lock_to_active => {
            active.or(next)
        }
        StateType::Unstarted if team.cycle_lock_to_active => active.or(next),
        StateType::Completed if team.cycle_issue_auto_assign_completed => active.or(prev),
        _ => None,
    })
}

impl App {
    async fn non_inheriting(&self, tx: &mut dyn Tx) -> AppResult<HashSet<TeamId>> {
        Ok(tx
            .cycles()
            .teams_not_inheriting()
            .await?
            .into_iter()
            .collect())
    }

    /// `list_cycles`: Cycles eines Teams (aufsteigend nach Nummer). Ohne aktivierte Cycles leer.
    pub async fn list_cycles(
        &self,
        ctx: &RequestCtx,
        team_ref: &str,
        kind: Option<CycleKind>,
    ) -> AppResult<(Team, Vec<CycleView>)> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, team_ref, Visibility::Content).await?;
        let now = self.clock.now();
        if !team.cycles_enabled {
            return Ok((team, Vec::new()));
        }
        let all = tx.cycles().list_for_team(team.id, false).await?;
        let mut selection: Vec<Cycle> = match kind {
            None => all.clone(),
            Some(CycleKind::Current) => all.iter().filter(|c| c.is_active(now)).cloned().collect(),
            Some(CycleKind::Previous) => all
                .iter()
                .filter(|c| c.is_past())
                .max_by_key(|c| c.ends_at)
                .cloned()
                .into_iter()
                .collect(),
            Some(CycleKind::Next) => all
                .iter()
                .filter(|c| c.is_future(now) && c.completed_at.is_none())
                .min_by_key(|c| c.starts_at)
                .cloned()
                .into_iter()
                .collect(),
        };
        selection.sort_by_key(|c| c.number);
        let views = build_views(&mut *tx, &team, &all, &selection, now).await?;
        Ok((team, views))
    }

    /// Ein Cycle per UUID.
    pub async fn get_cycle(&self, ctx: &RequestCtx, id: &str) -> AppResult<CycleView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let c = resolve::cycle::resolve_in(&mut *tx, &snap, None, id).await?;
        let team = snap
            .tree
            .get(c.team_id)
            .cloned()
            .ok_or_else(crate::policy::team_not_found)?;
        let all = tx.cycles().list_for_team(team.id, false).await?;
        let mut v = build_views(&mut *tx, &team, &all, &[c], self.clock.now()).await?;
        v.pop()
            .ok_or_else(|| AppError::internal("cycle view missing"))
    }

    /// Ändert die Cycle-Einstellungen eines Teams: Aktivieren (Startdatum), Zeitplan ändern
    /// (künftige Cycles neu berechnen), „Active issues“, Deaktivieren (docs/04 §1.2).
    pub async fn configure_team_cycles(
        &self,
        ctx: &RequestCtx,
        team_ref: &str,
        input: CycleSettingsInput,
    ) -> AppResult<Team> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, team_ref, Visibility::Content).await?;
        snap.require_team(ctx, Action::ManageCycles, team.id)?;
        let non_inh = self.non_inheriting(&mut *tx).await?;
        let now = self.clock.now();
        let env = Env {
            tree: &snap.tree,
            non_inheriting: &non_inh,
            now,
            ctx,
            ids: &*self.ids,
        };
        let inherited = inherits(&env, &team);
        let schedule_given = input.duration.is_some()
            || input.cooldown.is_some()
            || input.start_day.is_some()
            || input.upcoming_count.is_some();
        if inherited && schedule_given {
            return Err(invalid(
                "cycleDuration",
                "The cycle schedule is inherited from the parent team.",
            )
            .with_details(json!({ "field": "cycleDuration", "reason": "inherited_schedule" })));
        }
        let range = |field: &str, v: Option<i32>, lo: i32, hi: i32| -> AppResult<Option<i32>> {
            match v {
                Some(x) if !(lo..=hi).contains(&x) => {
                    Err(invalid(field, &format!("must be between {lo} and {hi}")))
                }
                other => Ok(other),
            }
        };
        let max_up = snap.org.limits.max_upcoming_cycles as i32;
        let patch = TeamPatch {
            cycles_enabled: input.cycles_enabled,
            cycle_duration: range("cycleDuration", input.duration, 1, 8)?,
            cycle_cooldown_time: range("cycleCooldownTime", input.cooldown, 0, 8)?,
            cycle_start_day: range("cycleStartDay", input.start_day, 0, 6)?,
            upcoming_cycle_count: range("upcomingCycleCount", input.upcoming_count, 1, max_up)?,
            cycle_issue_auto_assign_started: input.auto_assign_started,
            cycle_issue_auto_assign_completed: input.auto_assign_completed,
            cycle_lock_to_active: input.lock_to_active,
            timezone: match &input.timezone {
                Some(tz) => {
                    lineal_domain::time::parse_timezone(tz)?;
                    Some(tz.clone())
                }
                None => None,
            },
            ..Default::default()
        };
        let activating_lock = input.lock_to_active == Some(true) && !team.cycle_lock_to_active;
        if activating_lock && input.existing_active_issues.is_none() {
            return Err(invalid(
                "existingActiveIssues",
                "`existingActiveIssues` (moveToBacklog or addToCycle) is required when enabling active issues",
            ));
        }
        let was_enabled = team.cycles_enabled;
        let updated = tx.teams().update(team.id, &patch, now).await?;
        // Neuer Zustand für die Engine (Baum frisch laden, damit der Parent-Check stimmt).
        let snap2 = PolicySnapshot::load(&mut *tx, ctx).await?;
        let env = Env {
            tree: &snap2.tree,
            non_inheriting: &non_inh,
            now,
            ctx,
            ids: &*self.ids,
        };
        if updated.cycles_enabled && !was_enabled {
            engine::ensure_upcoming(&mut *tx, &env, &updated, input.first_start_date).await?;
            // Der erste Cycle kann sofort aktiv sein (Startdatum heute oder in der Vergangenheit).
            engine::run_team(&mut *tx, &env, &updated).await?;
        } else if !updated.cycles_enabled && was_enabled {
            engine::deactivate(&mut *tx, &env, &updated).await?;
        } else if updated.cycles_enabled && (schedule_given || input.timezone.is_some()) {
            engine::ensure_upcoming(&mut *tx, &env, &updated, None).await?;
        }
        if activating_lock && updated.cycles_enabled {
            let cycles = tx.cycles().list_for_team(updated.id, false).await?;
            let target = cycles
                .iter()
                .find(|c| c.is_active(now))
                .or_else(|| {
                    cycles
                        .iter()
                        .filter(|c| c.is_future(now))
                        .min_by_key(|c| c.starts_at)
                })
                .cloned();
            match input.existing_active_issues {
                Some(ExistingActiveIssues::AddToCycle) => {
                    if let Some(c) = target {
                        let moved = tx
                            .cycles()
                            .assign_active_issues_without_cycle(updated.id, c.id, now)
                            .await?;
                        for issue in moved {
                            engine::emit_issue_cycle(&mut *tx, &env, issue, updated.id).await?;
                        }
                    }
                }
                Some(ExistingActiveIssues::MoveToBacklog) => {
                    let states = tx.states().list_for_team(updated.id, false).await?;
                    let backlog = states
                        .iter()
                        .find(|s| s.state_type == StateType::Backlog)
                        .ok_or_else(|| AppError::internal("team has no backlog status"))?;
                    let moved = tx
                        .cycles()
                        .move_active_issues_without_cycle_to_backlog(updated.id, backlog.id, now)
                        .await?;
                    for issue in moved {
                        engine::emit_issue_cycle(&mut *tx, &env, issue, updated.id).await?;
                    }
                }
                None => {}
            }
        }
        engine::emit_team_updated(&mut *tx, &env, updated.id).await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Benennt einen Cycle um bzw. verschiebt Beginn/Ende (docs/04 §1.4 „Einzelne Cycles verschieben“).
    pub async fn update_cycle(
        &self,
        ctx: &RequestCtx,
        input: UpdateCycleInput,
    ) -> AppResult<CycleView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let c = resolve::cycle::resolve_in(&mut *tx, &snap, None, &input.id).await?;
        snap.require_team(ctx, Action::ManageCycles, c.team_id)?;
        let non_inh = self.non_inheriting(&mut *tx).await?;
        let env = Env {
            tree: &snap.tree,
            non_inheriting: &non_inh,
            now,
            ctx,
            ids: &*self.ids,
        };
        if (c.archived_at.is_some() || c.completed_at.is_some())
            && (input.starts_at.is_some() || input.ends_at.is_some())
        {
            return Err(invalid("endsAt", "Completed cycles cannot be rescheduled."));
        }
        if input.name.is_some() || input.description.is_some() {
            tx.cycles()
                .update_meta(c.id, input.name.clone(), input.description.clone(), now)
                .await?;
        }
        if let Some(Some(name)) = &input.name {
            renumber_after_rename(&mut *tx, &c, name, now).await?;
        }
        if input.starts_at.is_some() || input.ends_at.is_some() {
            if c.inherited_from_id.is_some() {
                return Err(invalid(
                    "startsAt",
                    "The dates of an inherited cycle come from the parent team's cycle.",
                )
                .with_details(json!({ "field": "startsAt", "reason": "inherited_cycle" })));
            }
            let new_start = input.starts_at.unwrap_or(c.starts_at);
            let new_end = input.ends_at.unwrap_or(c.ends_at);
            if input.starts_at.is_some() && !c.is_future(now) {
                return Err(invalid(
                    "startsAt",
                    "Only the start of an upcoming cycle can be changed.",
                ));
            }
            if new_start < now && input.starts_at.is_some() {
                return Err(invalid("startsAt", "The start must not be in the past."));
            }
            if new_end <= now || new_end <= new_start {
                return Err(invalid(
                    "endsAt",
                    "The end must be after the start and not in the past.",
                ));
            }
            let all = tx.cycles().list_for_team(c.team_id, true).await?;
            let following = all
                .iter()
                .filter(|x| x.starts_at > c.starts_at && x.id != c.id && x.archived_at.is_none())
                .min_by_key(|x| x.starts_at)
                .cloned();
            let preceding = all
                .iter()
                .filter(|x| x.starts_at < c.starts_at && x.id != c.id)
                .max_by_key(|x| x.starts_at);
            if preceding.is_some_and(|p| p.ends_at > new_start) {
                return Err(invalid(
                    "startsAt",
                    "The cycle would overlap the previous cycle.",
                ));
            }
            let mut moves = vec![];
            if let Some(f) = &following
                && new_end > f.starts_at
            {
                // Verlängern: der Start des folgenden Cycles rückt nach hinten, dessen Ende bleibt.
                if f.ends_at - new_end < Duration::days(1) {
                    return Err(invalid(
                        "endsAt",
                        "The following cycle would be shorter than one day.",
                    ));
                }
                moves.push((f.id, new_end, f.ends_at));
            }
            moves.push((c.id, new_start, new_end));
            // zuerst den folgenden Cycle, dann diesen (Verlängern) bzw. nur diesen (Verkürzen)
            engine::move_windows(&mut *tx, &env, &moves).await?;
            // geerbte Cycles der Sub-Teams nachziehen
            for child in snap.tree.children(c.team_id) {
                if inherits(&env, child) {
                    engine::sync_inherited(&mut *tx, &env, child).await?;
                }
            }
        }
        let team = snap
            .tree
            .get(c.team_id)
            .cloned()
            .ok_or_else(crate::policy::team_not_found)?;
        let all = tx.cycles().list_for_team(team.id, false).await?;
        let fresh = tx
            .cycles()
            .get(c.id)
            .await?
            .ok_or_else(|| AppError::internal("cycle missing"))?;
        engine::emit_cycle_updated(&mut *tx, &env, fresh.id, fresh.team_id).await?;
        let mut v = build_views(&mut *tx, &team, &all, &[fresh], now).await?;
        tx.commit().await?;
        v.pop()
            .ok_or_else(|| AppError::internal("cycle view missing"))
    }

    /// `cycleShiftAll`: verschiebt den angegebenen künftigen Cycle und alle späteren um `days`.
    pub async fn shift_cycles(
        &self,
        ctx: &RequestCtx,
        id: &str,
        days: i64,
    ) -> AppResult<Vec<CycleView>> {
        if !(-MAX_SHIFT_DAYS..=MAX_SHIFT_DAYS).contains(&days) {
            return Err(invalid(
                "daysToShift",
                "daysToShift must be between -3660 and 3660.",
            ));
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let c = resolve::cycle::resolve_in(&mut *tx, &snap, None, id).await?;
        snap.require_team(ctx, Action::ManageCycles, c.team_id)?;
        if !c.is_future(now) || c.completed_at.is_some() {
            return Err(invalid("id", "Only an upcoming cycle can be shifted."));
        }
        if c.inherited_from_id.is_some() {
            return Err(invalid(
                "id",
                "Inherited cycles follow the parent team's cycles.",
            ));
        }
        let team = snap
            .tree
            .get(c.team_id)
            .cloned()
            .ok_or_else(crate::policy::team_not_found)?;
        let tz = team_tz(&team);
        let all = tx.cycles().list_for_team(team.id, true).await?;
        let mut moving: Vec<&Cycle> = all
            .iter()
            .filter(|x| x.starts_at >= c.starts_at && x.completed_at.is_none())
            .collect();
        moving.sort_by_key(|x| x.starts_at);
        let shift = |t: DateTime<Utc>| -> AppResult<DateTime<Utc>> {
            let d = utc_to_local_date(t, tz)
                .checked_add_signed(Duration::days(days))
                .ok_or_else(|| invalid("daysToShift", "The shifted date is out of range."))?;
            Ok(local_midnight_to_utc(d, tz))
        };
        let moves: Vec<_> = moving
            .iter()
            .map(|x| Ok((x.id, shift(x.starts_at)?, shift(x.ends_at)?)))
            .collect::<AppResult<_>>()?;
        let first_start = moves.first().map(|m| m.1).unwrap_or(c.starts_at);
        if first_start <= now {
            return Err(invalid(
                "daysToShift",
                "Cycles cannot be shifted into the past.",
            ));
        }
        let preceding = all
            .iter()
            .filter(|x| x.starts_at < c.starts_at)
            .max_by_key(|x| x.starts_at);
        if preceding.is_some_and(|p| p.ends_at > first_start) {
            return Err(invalid(
                "daysToShift",
                "The shift would overlap the previous cycle.",
            ));
        }
        let non_inh = self.non_inheriting(&mut *tx).await?;
        let env = Env {
            tree: &snap.tree,
            non_inheriting: &non_inh,
            now,
            ctx,
            ids: &*self.ids,
        };
        engine::move_windows(&mut *tx, &env, &moves).await?;
        for child in snap.tree.children(team.id) {
            if inherits(&env, child) {
                engine::sync_inherited(&mut *tx, &env, child).await?;
            }
        }
        let all = tx.cycles().list_for_team(team.id, false).await?;
        let selection: Vec<Cycle> = all
            .iter()
            .filter(|x| moves.iter().any(|m| m.0 == x.id))
            .cloned()
            .collect();
        let views = build_views(&mut *tx, &team, &all, &selection, now).await?;
        tx.commit().await?;
        Ok(views)
    }

    /// `cycleStartUpcomingCycleToday`: der nächste noch nicht gestartete Cycle beginnt heute 00:00
    /// (Team-Zeitzone); ein laufender Cycle wird sofort abgeschlossen, unfertige Issues rollen über.
    pub async fn start_cycle_today(&self, ctx: &RequestCtx, id: &str) -> AppResult<CycleView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let c = resolve::cycle::resolve_in(&mut *tx, &snap, None, id).await?;
        snap.require_team(ctx, Action::ManageCycles, c.team_id)?;
        let team = snap
            .tree
            .get(c.team_id)
            .cloned()
            .ok_or_else(crate::policy::team_not_found)?;
        let all = tx.cycles().list_for_team(team.id, true).await?;
        let next = all
            .iter()
            .filter(|x| x.is_future(now) && x.completed_at.is_none())
            .min_by_key(|x| x.starts_at);
        if next.map(|n| n.id) != Some(c.id) {
            return Err(invalid(
                "id",
                "Only the next upcoming cycle can be started today.",
            ));
        }
        if c.inherited_from_id.is_some() {
            return Err(invalid(
                "id",
                "Inherited cycles follow the parent team's cycles.",
            ));
        }
        let tz = team_tz(&team);
        let new_start = local_midnight_to_utc(utc_to_local_date(now, tz), tz);
        let non_inh = self.non_inheriting(&mut *tx).await?;
        let env = Env {
            tree: &snap.tree,
            non_inheriting: &non_inh,
            now,
            ctx,
            ids: &*self.ids,
        };
        if let Some(active) = all.iter().find(|x| x.is_active(now)) {
            if new_start <= active.starts_at {
                return Err(invalid(
                    "id",
                    "The running cycle started today; it cannot be closed before it began.",
                ));
            }
            // Erst den laufenden Cycle verkürzen (Exclusion-Constraint), dann den neuen vorziehen.
            if active.started_processed_at.is_none() {
                tx.cycles().mark_started(active.id, now).await?;
            }
            engine::close_cycles(&mut *tx, &env, active, new_start).await?;
        }
        tx.cycles()
            .update_times(c.id, new_start, c.ends_at, now)
            .await?;
        let fresh = tx
            .cycles()
            .get(c.id)
            .await?
            .ok_or_else(|| AppError::internal("cycle missing"))?;
        // Das Sub-Team-Fenster nachziehen und den neuen Cycle starten (Rollover).
        for child in snap.tree.children(team.id) {
            if inherits(&env, child) {
                engine::sync_inherited(&mut *tx, &env, child).await?;
            }
        }
        engine::start_cycles(&mut *tx, &env, &fresh).await?;
        let all = tx.cycles().list_for_team(team.id, false).await?;
        let fresh = tx
            .cycles()
            .get(c.id)
            .await?
            .ok_or_else(|| AppError::internal("cycle missing"))?;
        let mut v = build_views(&mut *tx, &team, &all, &[fresh], now).await?;
        tx.commit().await?;
        v.pop()
            .ok_or_else(|| AppError::internal("cycle view missing"))
    }

    /// Archiviert einen abgeschlossenen oder künftigen Cycle; zugeordnete Issues verlieren ihn.
    pub async fn archive_cycle(&self, ctx: &RequestCtx, id: &str) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let now = self.clock.now();
        let c = resolve::cycle::resolve_in(&mut *tx, &snap, None, id).await?;
        snap.require_team(ctx, Action::ManageCycles, c.team_id)?;
        if c.is_active(now) {
            return Err(invalid("id", "The active cycle cannot be archived."));
        }
        let non_inh = self.non_inheriting(&mut *tx).await?;
        let env = Env {
            tree: &snap.tree,
            non_inheriting: &non_inh,
            now,
            ctx,
            ids: &*self.ids,
        };
        for issue in tx.cycles().all_issue_ids(c.id).await? {
            let team = tx
                .cycles()
                .set_issue_cycle(issue, Some(c.id), None, now)
                .await?;
            engine::emit_issue_cycle(&mut *tx, &env, issue, team).await?;
        }
        tx.cycles().archive(c.id, now).await?;
        engine::emit_cycle_updated(&mut *tx, &env, c.id, c.team_id).await?;
        tx.commit().await
    }

    /// Wartungslauf des Cycle-Jobs (docs/04 §1.5): pro Team unter Team-Sperre Abschluss, Start und
    /// Rollover, fehlende künftige Cycles, Snapshots; Teams ohne Cycles werden bereinigt.
    /// Parent-Teams laufen vor ihren Sub-Teams. Fehler eines Teams stoppen die anderen nicht.
    pub async fn run_cycle_maintenance(&self, now: DateTime<Utc>) -> AppResult<CycleRunStats> {
        let mut sys = self.store.begin_system().await?;
        let enabled: Vec<CycleTeamRef> = sys.cycles().teams_with_cycles(now).await?;
        let disabled: Vec<CycleTeamRef> = sys.cycles().teams_disabled_with_open_cycles().await?;
        drop(sys);
        let mut by_org: BTreeMap<OrgId, (Vec<TeamId>, Vec<TeamId>)> = BTreeMap::new();
        for r in enabled {
            by_org.entry(r.org_id).or_default().0.push(r.team_id);
        }
        for r in disabled {
            by_org.entry(r.org_id).or_default().1.push(r.team_id);
        }
        let mut stats = CycleRunStats::default();
        for (org, (enabled, disabled)) in by_org {
            let ctx = RequestCtx::system(org, "job-cycles");
            // Reihenfolge: Parents vor Kindern.
            let depth: BTreeMap<TeamId, i64> = {
                let mut tx = self.store.begin(org).await?;
                let snap = PolicySnapshot::load(&mut *tx, &ctx).await?;
                enabled.iter().map(|t| (*t, snap.tree.depth(*t))).collect()
            };
            let mut ordered = enabled.clone();
            ordered.sort_by_key(|t| (depth.get(t).copied().unwrap_or(0), *t));
            for team_id in ordered {
                match self.run_one_team(&ctx, team_id, now, false).await {
                    Ok(()) => stats.teams += 1,
                    Err(e) => {
                        tracing::warn!(team = %team_id, error = %e, "cycle maintenance failed");
                        stats.errors.push(format!("{team_id}: {e}"));
                    }
                }
            }
            for team_id in disabled {
                match self.run_one_team(&ctx, team_id, now, true).await {
                    Ok(()) => stats.teams += 1,
                    Err(e) => {
                        tracing::warn!(team = %team_id, error = %e, "cycle cleanup failed");
                        stats.errors.push(format!("{team_id}: {e}"));
                    }
                }
            }
            if let Err(e) = self.write_project_snapshots(&ctx, now).await {
                tracing::warn!(error = %e, "project snapshots failed");
                stats.errors.push(format!("project snapshots {org}: {e}"));
            }
        }
        Ok(stats)
    }

    async fn run_one_team(
        &self,
        ctx: &RequestCtx,
        team_id: TeamId,
        now: DateTime<Utc>,
        cleanup: bool,
    ) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let non_inh = self.non_inheriting(&mut *tx).await?;
        let Some(team) = snap.tree.get(team_id).cloned() else {
            return Ok(());
        };
        let env = Env {
            tree: &snap.tree,
            non_inheriting: &non_inh,
            now,
            ctx,
            ids: &*self.ids,
        };
        if cleanup {
            tx.cycles().lock_team(team.id).await?;
            engine::deactivate(&mut *tx, &env, &team).await?;
        } else {
            engine::run_team(&mut *tx, &env, &team).await?;
        }
        tx.commit().await
    }

    /// Wöchentlicher Snapshot der Projekt-Historien (Index = Wochen seit Anlage, Montag-basiert).
    pub async fn write_project_snapshots(
        &self,
        ctx: &RequestCtx,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        for id in tx.projects().active_project_ids().await? {
            let Some(p) = tx.projects().get(id).await? else {
                continue;
            };
            let tz = snap
                .tree
                .get(p.lead_team_id)
                .map(team_tz)
                .unwrap_or(chrono_tz::UTC);
            let week = |d: NaiveDate| {
                use chrono::Datelike;
                d - Duration::days(i64::from(d.weekday().num_days_from_monday()))
            };
            let start = week(utc_to_local_date(p.created_at, tz));
            let current = week(utc_to_local_date(now, tz));
            let idx = usize::try_from(((current - start).num_days() / 7).max(0)).unwrap_or(0);
            if p.scope_history.len() > idx {
                continue;
            }
            let rows = tx.projects().issue_rows(id).await?;
            let s = ProjectScope::from_rows(&rows);
            tx.projects()
                .write_history_point(
                    id,
                    idx,
                    crate::ports::repos::cycle::HistoryPoint {
                        scope: s.scope,
                        completed_scope: s.completed_scope,
                        in_progress_scope: s.in_progress_scope,
                        issue_count: s.issue_count as f64,
                        completed_issue_count: s.completed_issue_count as f64,
                    },
                    now,
                )
                .await?;
        }
        tx.commit().await
    }
}

/// Zahl am Ende eines Cycle-Namens (`Sprint 40` → 40, docs/04 §1.3).
pub fn trailing_number(name: &str) -> Option<i32> {
    let t = name.trim_end();
    let digits: String = t
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    // Länger als neun Ziffern passt nicht in eine Cycle-Nummer.
    if digits.is_empty() || digits.len() > 9 {
        return None;
    }
    digits.parse().ok()
}

/// `onCycleRenamed` (docs/04 §1.3): Endet der neue Name auf eine Zahl, werden die nicht
/// gestarteten, späteren Cycles des Teams ab dieser Zahl neu nummeriert; `Team.cycleNumberBase`
/// und der Zähler für neu erzeugte Cycles folgen. Namen der Form `Cycle <n>` werden geleert.
/// Kollidiert eine neue Nummer mit einem Cycle, der seine Nummer behält → `invalid_request`.
async fn renumber_after_rename(
    tx: &mut dyn Tx,
    cycle: &Cycle,
    new_name: &str,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let Some(base) = trailing_number(new_name) else {
        return Ok(());
    };
    let all = tx.cycles().list_for_team(cycle.team_id, true).await?;
    let mut future: Vec<&Cycle> = all
        .iter()
        .filter(|f| {
            f.id != cycle.id
                && f.starts_at > cycle.starts_at
                && f.starts_at > now
                && f.completed_at.is_none()
                && f.archived_at.is_none()
        })
        .collect();
    future.sort_by_key(|f| f.starts_at);
    let moved: HashSet<_> = future.iter().map(|f| f.id).collect();
    let mut kept: HashSet<i32> = all
        .iter()
        .filter(|x| !moved.contains(&x.id))
        .map(|x| x.number)
        .collect();
    // Der umbenannte Cycle behält seine Nummer.
    kept.insert(cycle.number);
    let mut n = base;
    let mut plan = Vec::with_capacity(future.len());
    for f in &future {
        n = n
            .checked_add(1)
            .ok_or_else(|| invalid("name", "Cycle number out of range."))?;
        if kept.contains(&n) {
            return Err(invalid(
                "name",
                "The following cycles cannot be renumbered because a cycle with that number already exists.",
            ));
        }
        plan.push((f, n));
    }
    // Zweiphasig, damit die Eindeutigkeit (Team, Nummer) zwischendurch nicht verletzt wird.
    const OFFSET: i32 = 1_000_000_000;
    for (f, _) in &plan {
        tx.cycles().set_number(f.id, OFFSET + f.number, now).await?;
    }
    for (f, number) in &plan {
        tx.cycles().set_number(f.id, *number, now).await?;
        if f.name.as_deref().is_none_or(is_default_cycle_name) {
            tx.cycles().update_meta(f.id, Some(None), None, now).await?;
        }
    }
    let highest_kept = kept.iter().copied().max().unwrap_or(0);
    tx.cycles()
        .set_team_number_base(cycle.team_id, base, n.max(highest_kept))
        .await
}

/// `Cycle <n>` – ein Name, der der Nummer folgt und deshalb entfällt.
fn is_default_cycle_name(name: &str) -> bool {
    name.strip_prefix("Cycle ")
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod capacity_tests {
    use super::*;

    #[test]
    fn trailing_numbers() {
        assert_eq!(trailing_number("Sprint 40"), Some(40));
        assert_eq!(trailing_number("Sprint 40  "), Some(40));
        assert_eq!(trailing_number("Sprint"), None);
        assert_eq!(trailing_number("Q4 plan"), None);
        assert_eq!(trailing_number("12345678901"), None);
    }

    #[test]
    fn default_names() {
        assert!(is_default_cycle_name("Cycle 7"));
        assert!(!is_default_cycle_name("Cycle x"));
        assert!(!is_default_cycle_name("Sprint 7"));
    }
}
