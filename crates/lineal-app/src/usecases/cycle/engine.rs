//! Cycle-Engine (docs/04 §1.4–1.9): Generator mit Zeitplan-Neuberechnung, Vererbung an Sub-Teams,
//! Abschluss (Phase 1), Start und Rollover (Phase 2), Tages-Snapshots, Deaktivierung.
//!
//! Alle Funktionen laufen in einer vom Aufrufer geöffneten Transaktion und sind idempotent.
//! Kalenderarithmetik ausschließlich über `rules::cycle_calendar` (lokale Mitternacht, DST-sicher).

use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::TeamTree;
use crate::ports::repos::NewEvent;
use crate::ports::repos::cycle::{Cycle, HistoryPoint, IssueScopeRow, NewCycle};
use crate::ports::repos::team::Team;
use crate::ports::{IdGen, Tx};
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use lineal_domain::enums::StateType;
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{CycleId, TeamId};
use lineal_domain::rules::cycle_calendar::{
    CycleSettings, CycleWindow, generate, generate_following, utc_to_local_date,
};
use lineal_domain::rules::progress::stats;
use std::collections::HashSet;

/// Obergrenze der Cycles, die ein einzelner Generatorlauf anlegt.
const MAX_GENERATED_CYCLES: usize = 3000;

/// Issue eines gelöschten eigenen Cycles samt dessen Zeitfenster.
type StrayIssue = (lineal_domain::ids::IssueId, (DateTime<Utc>, DateTime<Utc>));

/// Umgebung eines Engine-Laufs.
pub struct Env<'a> {
    pub tree: &'a TeamTree,
    /// Teams, die die Vererbung des Zeitplans abgewählt haben.
    pub non_inheriting: &'a HashSet<TeamId>,
    pub now: DateTime<Utc>,
    pub ctx: &'a RequestCtx,
    pub ids: &'a dyn IdGen,
}

/// Zeitzone des Teams (Fallback UTC bei ungültigem Namen).
pub fn team_tz(team: &Team) -> Tz {
    team.timezone.parse::<Tz>().unwrap_or(chrono_tz::UTC)
}

/// Ob das Team den Zeitplan seines Parents übernimmt (docs/04 §1.7).
pub fn inherits(env: &Env<'_>, team: &Team) -> bool {
    team.cycles_enabled
        && !env.non_inheriting.contains(&team.id)
        && team
            .parent_id
            .and_then(|p| env.tree.get(p))
            .is_some_and(|p| p.cycles_enabled)
}

/// Zeitplan-Einstellungen eines Teams, validiert.
pub fn settings_of(team: &Team) -> AppResult<CycleSettings> {
    let s = CycleSettings {
        duration_weeks: u32::try_from(team.cycle_duration).unwrap_or(0),
        cooldown_weeks: u32::try_from(team.cycle_cooldown_time).unwrap_or(99),
        start_day: u8::try_from(team.cycle_start_day).unwrap_or(99),
    };
    s.validate()?;
    Ok(s)
}

/// Fingerprint der Zeitplan-Einstellungen (Dauer, Cooldown, Starttag, Zeitzone).
pub fn fingerprint(team: &Team) -> String {
    format!(
        "{}:{}:{}:{}",
        team.cycle_duration, team.cycle_cooldown_time, team.cycle_start_day, team.timezone
    )
}

async fn emit(tx: &mut dyn Tx, env: &Env<'_>, event: DomainEvent) -> AppResult<()> {
    tx.outbox().push(NewEvent::from_ctx(event, env.ctx)).await?;
    Ok(())
}

fn team_of<'a>(env: &'a Env<'_>, id: TeamId) -> AppResult<&'a Team> {
    env.tree
        .get(id)
        .ok_or_else(|| AppError::not_found("Could not find referenced Team."))
}

/// Legt einen Cycle an (Nummer vom Team-Zähler) und schreibt `CycleCreated`.
pub async fn create_cycle(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    team: TeamId,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    inherited_from: Option<&Cycle>,
) -> AppResult<Cycle> {
    let number = tx.teams().next_cycle_number(team).await?;
    let id = CycleId(env.ids.new_uuid());
    tx.cycles()
        .insert(&NewCycle {
            id,
            team_id: team,
            number,
            name: inherited_from.and_then(|p| p.name.clone()),
            description: inherited_from.and_then(|p| p.description.clone()),
            starts_at: start,
            ends_at: end,
            inherited_from_id: inherited_from.map(|p| p.id),
            now: env.now,
        })
        .await?;
    emit(
        tx,
        env,
        DomainEvent::CycleCreated {
            cycle_id: id,
            team_id: team,
        },
    )
    .await?;
    tx.cycles()
        .get(id)
        .await?
        .ok_or_else(|| AppError::internal("cycle missing after insert"))
}

/// Löst alle Issues vom Cycle (History, `IssueUpdated`) und löscht ihn samt geerbten Cycles der
/// Sub-Teams. Mit `move_to` landen die Issues dort statt bei `null`.
pub async fn delete_cycle(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    cycle: &Cycle,
    move_to: Option<&Cycle>,
) -> AppResult<()> {
    // Geerbte Cycles zuerst (FK `inherited_from_id`).
    let mut order = vec![cycle.clone()];
    let mut i = 0;
    while i < order.len() {
        let kids = tx.cycles().list_inheriting(order[i].id).await?;
        order.extend(kids);
        i += 1;
    }
    for c in order.iter().rev() {
        let target = if c.id == cycle.id { move_to } else { None };
        for issue in tx.cycles().all_issue_ids(c.id).await? {
            let team = tx
                .cycles()
                .set_issue_cycle(issue, Some(c.id), target.map(|t| t.id), env.now)
                .await?;
            emit_issue_cycle(tx, env, issue, team).await?;
        }
        tx.cycles().delete(c.id).await?;
        emit(
            tx,
            env,
            DomainEvent::CycleRemoved {
                cycle_id: c.id,
                team_id: c.team_id,
            },
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn emit_issue_cycle(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    issue: lineal_domain::ids::IssueId,
    team: TeamId,
) -> AppResult<()> {
    emit(
        tx,
        env,
        DomainEvent::IssueUpdated {
            previous: None,
            issue_id: issue,
            team_id: team,
            changed: vec!["cycleId".into()],
            mentioned_user_ids: vec![],
        },
    )
    .await
}

/// Parkt die Cycles in disjunkte Fenster weit in der Zukunft und setzt dann die Zielfenster. So
/// entstehen keine zwischenzeitlichen Überlappungen (Exclusion-Constraint je Statement).
pub async fn move_windows(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    moves: &[(CycleId, DateTime<Utc>, DateTime<Utc>)],
) -> AppResult<()> {
    if moves.is_empty() {
        return Ok(());
    }
    let base = Utc
        .with_ymd_and_hms(9000, 1, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| AppError::internal("invalid parking date"))?;
    for (i, (id, _, _)) in moves.iter().enumerate() {
        let s = base + Duration::days(i as i64 * 2);
        tx.cycles()
            .update_times(*id, s, s + Duration::days(1), env.now)
            .await?;
    }
    for (id, s, e) in moves {
        tx.cycles().update_times(*id, *s, *e, env.now).await?;
    }
    for (id, _, _) in moves {
        if let Some(c) = tx.cycles().get(*id).await? {
            emit(
                tx,
                env,
                DomainEvent::CycleUpdated {
                    cycle_id: c.id,
                    team_id: c.team_id,
                },
            )
            .await?;
        }
    }
    Ok(())
}

fn window_utc(w: &CycleWindow) -> (DateTime<Utc>, DateTime<Utc>) {
    (w.starts_at, w.ends_at)
}

/// Fenster eines Folge-Cycles, das nie vor dem Ende des Vorgängers beginnt. Nach einem
/// Zeitzonenwechsel (z. B. nach Osten) liegt die lokale Mitternacht des Folgetags sonst vor dem
/// unveränderten Ende des laufenden Cycles (Überlappung); dann beginnt der Folge-Cycle exakt dort.
fn after_prev(prev_end: DateTime<Utc>, w: &CycleWindow) -> (DateTime<Utc>, DateTime<Utc>) {
    let start = w.starts_at.max(prev_end);
    (start, w.ends_at.max(start + Duration::hours(1)))
}

/// Überlappung zweier Zeitfenster in Sekunden (negativ/0 = keine).
fn overlap_secs(a: (DateTime<Utc>, DateTime<Utc>), b: (DateTime<Utc>, DateTime<Utc>)) -> i64 {
    let start = a.0.max(b.0);
    let end = a.1.min(b.1);
    (end - start).num_seconds()
}

/// Cycle mit der größten zeitlichen Überschneidung (sonst der nächstfolgende).
pub fn best_overlap(cycles: &[Cycle], window: (DateTime<Utc>, DateTime<Utc>)) -> Option<&Cycle> {
    let best = cycles
        .iter()
        .map(|c| (overlap_secs((c.starts_at, c.ends_at), window), c))
        .filter(|(o, _)| *o > 0)
        .max_by_key(|(o, _)| *o)
        .map(|(_, c)| c);
    best.or_else(|| {
        cycles
            .iter()
            .filter(|c| c.starts_at >= window.1)
            .min_by_key(|c| c.starts_at)
    })
}

/// Stellt sicher, dass `upcomingCycleCount` künftige Cycles existieren, berechnet bei geänderten
/// Einstellungen neu und reicht neue Cycles an erbende Sub-Teams weiter (idempotent).
///
/// `first_start` ist das Startdatum des ersten Cycles bei Aktivierung (lokales Kalenderdatum);
/// ohne Angabe beginnt der erste Cycle heute (lokal).
pub async fn ensure_upcoming(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    team: &Team,
    first_start: Option<NaiveDate>,
) -> AppResult<()> {
    if !team.cycles_enabled {
        return Ok(());
    }
    if inherits(env, team) {
        return sync_inherited(tx, env, team).await;
    }
    let tz = team_tz(team);
    let settings = settings_of(team)?;
    let up = usize::try_from(team.upcoming_cycle_count)
        .unwrap_or(2)
        .max(1);
    let cycles = tx.cycles().list_for_team(team.id, true).await?;
    let mut future: Vec<Cycle> = cycles
        .iter()
        .filter(|c| c.starts_at > env.now && c.completed_at.is_none())
        .cloned()
        .collect();
    future.sort_by_key(|c| c.starts_at);

    // `upcomingCycleCount` verkleinert: überzählige künftige Cycles am Ende löschen.
    if future.len() > up {
        for c in future.split_off(up) {
            delete_cycle(tx, env, &c, None).await?;
        }
    }

    // Zeitplan geändert: künftige Cycles in Reihenfolge wiederverwenden und neu berechnen.
    let fp = fingerprint(team);
    let stored = tx.cycles().team_schedule_fp(team.id).await?;
    if !future.is_empty() && stored.as_deref().is_some_and(|s| s != fp) {
        let base = cycles
            .iter()
            .filter(|c| c.starts_at <= env.now)
            .max_by_key(|c| c.starts_at);
        let windows = match base {
            Some(b) => generate_following(
                &settings,
                tz,
                utc_to_local_date(b.ends_at, tz),
                future.len(),
            )?,
            None => generate(
                &settings,
                tz,
                utc_to_local_date(future[0].starts_at, tz),
                future.len(),
            )?,
        };
        let mut prev_end = base.map(|b| b.ends_at);
        let mut moves = Vec::new();
        for (c, w) in future.iter().zip(&windows) {
            let (s, e) = match prev_end {
                Some(p) => after_prev(p, w),
                None => window_utc(w),
            };
            prev_end = Some(e);
            if (c.starts_at, c.ends_at) != (s, e) {
                moves.push((c.id, s, e));
            }
        }
        move_windows(tx, env, &moves).await?;
    }
    if stored.as_deref() != Some(fp.as_str()) {
        tx.cycles().set_team_schedule_fp(team.id, Some(&fp)).await?;
    }

    // Fehlende künftige Cycles anlegen (docs/04 §1.4 `ensureUpcomingCycles`).
    let mut all = tx.cycles().list_for_team(team.id, true).await?;
    let mut future_count = all
        .iter()
        .filter(|c| c.starts_at > env.now && c.completed_at.is_none())
        .count();
    let mut last = all.iter().max_by_key(|c| c.starts_at).cloned();
    let mut guard = 0;
    while future_count < up {
        guard += 1;
        // Startdatum weit in der Vergangenheit: lieber ehrlich ablehnen als einen halben Zustand
        // ohne aktiven Cycle zu hinterlassen (B5.29).
        if guard > MAX_GENERATED_CYCLES {
            return Err(AppError::invalid_request(
                "The first cycle start date is too far in the past for this schedule.",
            )
            .with_details(serde_json::json!({ "field": "firstCycleStartDate" })));
        }
        let window = match &last {
            Some(l) => generate_following(&settings, tz, utc_to_local_date(l.ends_at, tz), 1)?,
            None => {
                let from = first_start.unwrap_or_else(|| utc_to_local_date(env.now, tz));
                generate(&settings, tz, from, 1)?
            }
        }
        .remove(0);
        let (ws, we) = match &last {
            Some(l) => after_prev(l.ends_at, &window),
            None => window_utc(&window),
        };
        let created = create_cycle(tx, env, team.id, ws, we, None).await?;
        if created.starts_at > env.now {
            future_count += 1;
        }
        all.push(created.clone());
        last = Some(created);
    }

    // Weitergabe an erbende Sub-Teams (rekursiv).
    for child in env.tree.children(team.id) {
        if inherits(env, child) {
            sync_inherited(tx, env, child).await?;
        }
    }
    Ok(())
}

/// Spiegelt die Cycles des Parent-Teams in das erbende Sub-Team (docs/04 §1.7 `syncInheritedCycles`).
pub async fn sync_inherited(tx: &mut dyn Tx, env: &Env<'_>, sub: &Team) -> AppResult<()> {
    let Some(parent_id) = sub.parent_id else {
        return Ok(());
    };
    let parent_cycles = tx.cycles().list_for_team(parent_id, false).await?;
    let own = tx.cycles().list_for_team(sub.id, true).await?;

    // Eigene künftige Cycles, die nicht geerbt sind, werden gelöscht; ihre Issues wandern später
    // in den geerbten Cycle mit der größten Überschneidung.
    let strays: Vec<Cycle> = own
        .iter()
        .filter(|c| {
            c.inherited_from_id.is_none() && c.starts_at > env.now && c.completed_at.is_none()
        })
        .cloned()
        .collect();
    let mut stray_issues: Vec<StrayIssue> = Vec::new();
    for c in &strays {
        for issue in tx.cycles().all_issue_ids(c.id).await? {
            stray_issues.push((issue, (c.starts_at, c.ends_at)));
        }
        delete_cycle(tx, env, c, None).await?;
    }
    let mut own = tx.cycles().list_for_team(sub.id, true).await?;

    let mut moves = Vec::new();
    let mut to_create: Vec<&Cycle> = Vec::new();
    let mut parents_sorted: Vec<&Cycle> = parent_cycles
        .iter()
        .filter(|c| c.ends_at > env.now && c.archived_at.is_none())
        .collect();
    parents_sorted.sort_by_key(|c| c.starts_at);
    for pc in parents_sorted {
        match own.iter().find(|c| c.inherited_from_id == Some(pc.id)) {
            Some(sc) => {
                if sc.completed_at.is_none()
                    && (sc.starts_at != pc.starts_at || sc.ends_at != pc.ends_at)
                {
                    moves.push((sc.id, pc.starts_at, pc.ends_at));
                }
            }
            None => to_create.push(pc),
        }
    }
    move_windows(tx, env, &moves).await?;
    own = tx.cycles().list_for_team(sub.id, true).await?;
    let mut created_cycles: Vec<Cycle> = Vec::new();
    for pc in to_create {
        // Überlappt ein bestehendes Cycle des Sub-Teams (z. B. der noch laufende eigene Cycle),
        // wird der geerbte erst angelegt, sobald das Fenster frei ist.
        let blocked = own
            .iter()
            .chain(created_cycles.iter())
            .any(|c| overlap_secs((c.starts_at, c.ends_at), (pc.starts_at, pc.ends_at)) > 0);
        if blocked {
            continue;
        }
        let c = create_cycle(tx, env, sub.id, pc.starts_at, pc.ends_at, Some(pc)).await?;
        if pc.started_processed_at.is_some() {
            start_cycles(tx, env, &c).await?;
        }
        created_cycles.push(c);
    }
    if !stray_issues.is_empty() {
        let inherited: Vec<Cycle> = tx
            .cycles()
            .list_for_team(sub.id, false)
            .await?
            .into_iter()
            .filter(|c| c.inherited_from_id.is_some() && c.starts_at > env.now)
            .collect();
        for (issue, window) in stray_issues {
            if let Some(target) = best_overlap(&inherited, window) {
                let team = tx
                    .cycles()
                    .set_issue_cycle(issue, None, Some(target.id), env.now)
                    .await?;
                emit_issue_cycle(tx, env, issue, team).await?;
            }
        }
    }
    for child in env.tree.children(sub.id) {
        if inherits(env, child) {
            Box::pin(sync_inherited(tx, env, child)).await?;
        }
    }
    Ok(())
}

fn point_of(rows: &[IssueScopeRow]) -> HistoryPoint {
    let inputs: Vec<_> = rows.iter().map(IssueScopeRow::progress_input).collect();
    let s = stats(&inputs);
    let relevant: Vec<&IssueScopeRow> = rows
        .iter()
        .filter(|r| !matches!(r.state_type, StateType::Canceled | StateType::Duplicate))
        .collect();
    HistoryPoint {
        scope: s.scope,
        completed_scope: s.completed,
        in_progress_scope: s.started,
        issue_count: relevant.len() as f64,
        completed_issue_count: relevant
            .iter()
            .filter(|r| r.state_type == StateType::Completed)
            .count() as f64,
    }
}

/// Index im Historien-Array: Tage zwischen lokalem Starttag und lokalem Tag von `at`
/// (der Schluss-Snapshot bei `ends_at` gehört zum letzten Tag).
pub fn history_index(c: &Cycle, tz: Tz, at: DateTime<Utc>) -> usize {
    let start = utc_to_local_date(c.starts_at, tz);
    let day = utc_to_local_date(at, tz);
    let mut idx = (day - start).num_days();
    if at == c.ends_at {
        idx -= 1;
    }
    usize::try_from(idx.max(0)).unwrap_or(0)
}

/// Schreibt den Tageswert (`writeDailySnapshot`).
pub async fn write_snapshot(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    c: &Cycle,
    at: DateTime<Utc>,
) -> AppResult<()> {
    if c.completed_at.is_some() {
        return Ok(());
    }
    let tz = team_tz(team_of(env, c.team_id)?);
    let rows = tx.cycles().issue_rows(c.id).await?;
    let idx = history_index(c, tz, at);
    tx.cycles()
        .write_history_point(c.id, idx, point_of(&rows), env.now)
        .await
}

/// Tages-Snapshot aller laufenden Cycles eines Teams: einmal pro lokalem Tag (Index aus dem Datum).
pub async fn daily_snapshots(tx: &mut dyn Tx, env: &Env<'_>, team: &Team) -> AppResult<()> {
    let tz = team_tz(team);
    for c in tx.cycles().list_for_team(team.id, false).await? {
        if c.is_active(env.now) && c.started_processed_at.is_some() {
            let idx = history_index(&c, tz, env.now);
            if c.scope_history.len() <= idx {
                write_snapshot(tx, env, &c, env.now).await?;
            }
        }
    }
    Ok(())
}

/// Phase 1 (docs/04 §1.5 `closeCycle`): letzten Snapshot schreiben, Historie einfrieren, offene
/// Issues festhalten – samt geerbter Cycles der Sub-Teams.
pub async fn close_cycles(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    cycle: &Cycle,
    close_time: DateTime<Utc>,
) -> AppResult<()> {
    let mut stack = vec![cycle.clone()];
    while let Some(c) = stack.pop() {
        let Some(c) = tx.cycles().get(c.id).await? else {
            continue;
        };
        if c.completed_at.is_some() {
            continue;
        }
        write_snapshot(tx, env, &c, close_time.min(c.ends_at)).await?;
        let rows = tx.cycles().issue_rows(c.id).await?;
        let progress = stats(
            &rows
                .iter()
                .map(IssueScopeRow::progress_input)
                .collect::<Vec<_>>(),
        )
        .progress;
        let open = tx.cycles().open_issue_ids(c.id).await?;
        tx.cycles()
            .close(
                c.id,
                close_time,
                close_time.min(c.ends_at),
                &open,
                progress,
                env.now,
            )
            .await?;
        emit(
            tx,
            env,
            DomainEvent::CycleUpdated {
                cycle_id: c.id,
                team_id: c.team_id,
            },
        )
        .await?;
        stack.extend(tx.cycles().list_inheriting(c.id).await?);
    }
    Ok(())
}

/// Phase 2 (docs/04 §1.5 `startCycle`): unfertige Issues des zuletzt abgeschlossenen Cycles
/// übernehmen, Tag-0-Snapshot – samt geerbter Cycles der Sub-Teams.
pub async fn start_cycles(tx: &mut dyn Tx, env: &Env<'_>, cycle: &Cycle) -> AppResult<()> {
    let mut stack = vec![cycle.clone()];
    while let Some(c) = stack.pop() {
        let Some(c) = tx.cycles().get(c.id).await? else {
            continue;
        };
        if c.started_processed_at.is_some() {
            continue;
        }
        let siblings = tx.cycles().list_for_team(c.team_id, true).await?;
        let prev = siblings
            .iter()
            .filter(|p| p.id != c.id && p.completed_at.is_some() && p.ends_at <= c.starts_at)
            .max_by_key(|p| p.ends_at);
        if let Some(prev) = prev {
            let ids = prev.uncompleted_issue_ids.clone().unwrap_or_default();
            for (issue, state) in tx.cycles().issue_states_in_cycle(prev.id, &ids).await? {
                let target = match state {
                    StateType::Unstarted | StateType::Started => Some(Some(c.id)),
                    StateType::Backlog | StateType::Triage => Some(None),
                    _ => None, // completed/canceled/duplicate: bleibt im alten Cycle
                };
                if let Some(to) = target {
                    let team = tx
                        .cycles()
                        .set_issue_cycle(issue, Some(prev.id), to, env.now)
                        .await?;
                    emit_issue_cycle(tx, env, issue, team).await?;
                }
            }
        }
        tx.cycles().mark_started(c.id, env.now).await?;
        let fresh = tx
            .cycles()
            .get(c.id)
            .await?
            .ok_or_else(|| AppError::internal("cycle vanished"))?;
        write_snapshot(tx, env, &fresh, fresh.starts_at).await?;
        emit(
            tx,
            env,
            DomainEvent::CycleUpdated {
                cycle_id: c.id,
                team_id: c.team_id,
            },
        )
        .await?;
        stack.extend(tx.cycles().list_inheriting(c.id).await?);
    }
    Ok(())
}

/// Ein Wechsel-Durchlauf für ein Team (unter Team-Sperre): Start/Rollover und Abschluss in
/// zeitlicher Reihenfolge, fehlende künftige Cycles, Tages-Snapshot.
pub async fn run_team(tx: &mut dyn Tx, env: &Env<'_>, team: &Team) -> AppResult<()> {
    tx.cycles().lock_team(team.id).await?;
    if !inherits(env, team) {
        let cycles = tx.cycles().list_for_team(team.id, false).await?;
        for c in cycles {
            let Some(c) = tx.cycles().get(c.id).await? else {
                continue;
            };
            if c.started_processed_at.is_none() && c.starts_at <= env.now {
                start_cycles(tx, env, &c).await?;
            }
            let Some(c) = tx.cycles().get(c.id).await? else {
                continue;
            };
            if c.completed_at.is_none() && c.ends_at <= env.now {
                close_cycles(tx, env, &c, c.ends_at).await?;
            }
        }
    }
    ensure_upcoming(tx, env, team, None).await?;
    daily_snapshots(tx, env, team).await
}

/// Deaktivierung (docs/04 §1.2): geerbte Sub-Team-Cycles werden eigenständig, der aktive Cycle wird
/// sofort abgeschlossen, künftige Cycles werden gelöscht; offene Issues verlieren die Zuordnung.
pub async fn deactivate(tx: &mut dyn Tx, env: &Env<'_>, team: &Team) -> AppResult<()> {
    let cycles = tx.cycles().list_for_team(team.id, true).await?;
    for c in &cycles {
        for child in tx.cycles().list_inheriting(c.id).await? {
            tx.cycles()
                .set_inherited_from(child.id, None, env.now)
                .await?;
        }
    }
    for c in cycles
        .iter()
        .filter(|c| c.completed_at.is_none() && c.archived_at.is_none())
    {
        if c.starts_at <= env.now {
            if c.started_processed_at.is_none() {
                tx.cycles().mark_started(c.id, env.now).await?;
            }
            let fresh = tx.cycles().get(c.id).await?.unwrap_or_else(|| c.clone());
            close_cycles_single(tx, env, &fresh).await?;
            for issue in tx.cycles().all_issue_ids(c.id).await? {
                // offene Issues verlieren die Zuordnung; erledigte bleiben dem Cycle zugerechnet
                let state = tx
                    .cycles()
                    .issue_states_in_cycle(c.id, &[issue.0])
                    .await?
                    .into_iter()
                    .next();
                if state.is_some_and(|(_, s)| {
                    matches!(
                        s,
                        StateType::Triage
                            | StateType::Backlog
                            | StateType::Unstarted
                            | StateType::Started
                    )
                }) {
                    let t = tx
                        .cycles()
                        .set_issue_cycle(issue, Some(c.id), None, env.now)
                        .await?;
                    emit_issue_cycle(tx, env, issue, t).await?;
                }
            }
        } else {
            delete_cycle(tx, env, c, None).await?;
        }
    }
    tx.cycles().set_team_schedule_fp(team.id, None).await?;
    Ok(())
}

/// Schließt genau diesen Cycle (ohne Sub-Team-Cycles; bei Deaktivierung sind sie bereits gelöst).
async fn close_cycles_single(tx: &mut dyn Tx, env: &Env<'_>, c: &Cycle) -> AppResult<()> {
    if c.completed_at.is_some() {
        return Ok(());
    }
    let close_time = env.now.min(c.ends_at);
    write_snapshot(tx, env, c, close_time).await?;
    let rows = tx.cycles().issue_rows(c.id).await?;
    let progress = stats(
        &rows
            .iter()
            .map(IssueScopeRow::progress_input)
            .collect::<Vec<_>>(),
    )
    .progress;
    let open = tx.cycles().open_issue_ids(c.id).await?;
    tx.cycles()
        .close(c.id, close_time, close_time, &open, progress, env.now)
        .await?;
    emit(
        tx,
        env,
        DomainEvent::CycleUpdated {
            cycle_id: c.id,
            team_id: c.team_id,
        },
    )
    .await
}

/// `CycleUpdated`-Event.
pub async fn emit_cycle_updated(
    tx: &mut dyn Tx,
    env: &Env<'_>,
    cycle: CycleId,
    team: TeamId,
) -> AppResult<()> {
    emit(
        tx,
        env,
        DomainEvent::CycleUpdated {
            cycle_id: cycle,
            team_id: team,
        },
    )
    .await
}

/// `TeamUpdated`-Event (Cycle-Einstellungen geändert).
pub async fn emit_team_updated(tx: &mut dyn Tx, env: &Env<'_>, team: TeamId) -> AppResult<()> {
    emit(tx, env, DomainEvent::TeamUpdated { team_id: team }).await
}
