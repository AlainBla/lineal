//! usecases – Team-Einstellungen Rest (WP30): SLA-Regeln (docs/03 §11.2) und wiederkehrende Issues
//! (docs/03 §13).
//!
//! Sicherheit: jede Methode löst das Team über `resolve::team::resolve_in` (private Teams ohne
//! Zugriff → `not_found`) und prüft für Schreibzugriffe `Action::UpdateTeamSettings` (SLA) bzw.
//! `ManageTeamTemplates` + `CreateIssue` (wiederkehrende Issues, über `prepare_issue_template`).
//! Regeln und Wiederholungen eines Teams werden immer über `team_id` gefiltert; fremde IDs →
//! `not_found`.

use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult, ErrorKind};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::NewEvent;
use crate::ports::repos::issue::HistorySource;
use crate::ports::repos::team::Team;
use crate::ports::repos::triage::{NewRecurringIssue, NewSlaRule, RecurringIssue, SlaRule};
use crate::resolve;
use crate::resolve::team::Visibility;
use crate::resolve::user::actor_user_id;
use crate::usecases::issue::CreateIssueInput;
use crate::usecases::template::{create_issue_tree, prepare_issue_template};
use crate::usecases::triage::sla::amount_of;
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use lineal_domain::enums::SlaType;
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::TeamId;
use lineal_domain::rules::filter::{IsoDuration, parse_issue_filter};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Höchstzahl SLA-Regeln je Team.
pub const MAX_SLA_RULES: usize = 50;
/// Höchstzahl wiederkehrender Issues je Team.
pub const MAX_RECURRING_PER_TEAM: usize = 100;
/// Größe des Regelfilters (serialisiert).
const MAX_FILTER_BYTES: usize = 20_000;
/// Erlaubte Filterfelder einer SLA-Regel (docs/03 §11.2).
const RULE_FILTER_KEYS: [&str; 9] = [
    "and", "or", "priority", "state", "assignee", "creator", "team", "project", "labels",
];
/// Längste Dauer einer Regel in Tagen.
const MAX_SLA_DAYS: i64 = 3650;
/// Wie viele fällige Wiederholungen ein Lauf je Workspace höchstens abarbeitet.
const MAX_RUNS_PER_ORG: usize = 100;

fn invalid(field: &str, reason: &str, msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg).with_details(json!({ "field": field, "reason": reason }))
}

fn not_found(kind: &str, reference: &str) -> AppError {
    AppError::not_found(format!("Could not find referenced {kind}."))
        .with_details(json!({ "reference": reference, "type": kind }))
}

async fn emit(tx: &mut dyn Tx, ctx: &RequestCtx, event: DomainEvent) -> AppResult<()> {
    tx.outbox().push(NewEvent::from_ctx(event, ctx)).await?;
    Ok(())
}

fn tz_of(team: &Team) -> Tz {
    team.timezone.parse().unwrap_or(chrono_tz::UTC)
}

// ---------------------------------------------------------------------------
// SLA
// ---------------------------------------------------------------------------

/// SLA-Einstellungen eines Teams samt Regeln.
#[derive(Debug, Clone, PartialEq)]
pub struct TeamSlaView {
    pub team_id: TeamId,
    pub enabled: bool,
    /// `true` = Arbeitswoche Sonntag–Donnerstag.
    pub sun_thu: bool,
    pub rules: Vec<SlaRule>,
}

/// Eingabe von `set_team_sla`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SetTeamSlaInput {
    pub team: String,
    pub enabled: Option<bool>,
    /// `monToFri` oder `sunToThu`.
    pub work_week: Option<String>,
}

/// Eingabe einer neuen SLA-Regel.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateSlaRuleInput {
    pub team: String,
    pub name: String,
    /// `IssueFilter` als JSON (erlaubt: Priorität, Status, Assignee, Creator, Team, Projekt, Labels).
    pub filter: Value,
    pub removes_sla: bool,
    /// ISO-8601-Dauer (Pflicht bei Setz-Regeln).
    pub duration: Option<String>,
    pub sla_type: SlaType,
    /// `issueCreation` (Default) oder `ruleMatch`.
    pub start_mode: Option<String>,
}

/// Änderung einer SLA-Regel (`None` = unverändert).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateSlaRuleInput {
    pub team: String,
    pub id: Uuid,
    pub name: Option<String>,
    pub filter: Option<Value>,
    pub removes_sla: Option<bool>,
    pub duration: Option<String>,
    pub sla_type: Option<SlaType>,
    pub start_mode: Option<String>,
    pub position: Option<f64>,
}

fn validate_rule(rule: &SlaRule) -> AppResult<()> {
    let name = rule.name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(invalid(
            "name",
            "length",
            "name must be between 1 and 100 characters",
        ));
    }
    if !matches!(rule.start_mode.as_str(), "issueCreation" | "ruleMatch") {
        return Err(invalid(
            "startMode",
            "invalid",
            "startMode must be issueCreation or ruleMatch",
        ));
    }
    let Some(obj) = rule.filter.as_object() else {
        return Err(invalid("filter", "invalid", "filter must be an object"));
    };
    if rule.filter.to_string().len() > MAX_FILTER_BYTES {
        return Err(AppError::limit_exceeded("filter is too large")
            .with_details(json!({ "field": "filter", "max": MAX_FILTER_BYTES })));
    }
    if let Some(k) = obj.keys().find(|k| !RULE_FILTER_KEYS.contains(&k.as_str())) {
        return Err(invalid(
            "filter",
            "unsupported_field",
            format!("`{k}` cannot be used in an SLA rule"),
        ));
    }
    parse_issue_filter(&rule.filter.to_string()).map_err(|e| {
        invalid(
            "filter",
            "invalid",
            format!("invalid filter at {}: {}", e.path, e),
        )
    })?;
    if rule.removes_sla {
        return Ok(());
    }
    let Some(d) = rule.duration.as_deref() else {
        return Err(invalid(
            "duration",
            "required",
            "a rule that sets an SLA needs a duration",
        ));
    };
    match IsoDuration::parse(d) {
        Some(p) if !p.negative => {}
        _ => {
            return Err(invalid(
                "duration",
                "invalid",
                "duration must be a positive ISO-8601 duration",
            ));
        }
    }
    let amount = amount_of(rule).filter(|a| *a > 0);
    let max = match rule.sla_type {
        SlaType::All => MAX_SLA_DAYS * 86_400_000,
        SlaType::OnlyBusinessDays => MAX_SLA_DAYS,
    };
    match amount {
        Some(a) if a <= max => Ok(()),
        _ => Err(invalid(
            "duration",
            "out_of_range",
            "duration must be positive and at most ten years",
        )),
    }
}

impl App {
    /// SLA-Einstellungen und Regeln eines Teams.
    pub async fn get_team_sla(&self, ctx: &RequestCtx, team_ref: &str) -> AppResult<TeamSlaView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, team_ref, Visibility::Content).await?;
        let s = tx.triage().team_sla(team.id).await?;
        let rules = tx.triage().sla_rules(team.id).await?;
        Ok(TeamSlaView {
            team_id: team.id,
            enabled: s.enabled,
            sun_thu: s.sun_thu,
            rules,
        })
    }

    /// Schaltet SLA ein/aus und setzt die Arbeitswoche. Beim ersten Aktivieren ohne Regeln werden
    /// die Standardregeln angelegt (docs/03 §11.2).
    pub async fn set_team_sla(
        &self,
        ctx: &RequestCtx,
        input: SetTeamSlaInput,
    ) -> AppResult<TeamSlaView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, &input.team, Visibility::Content).await?;
        snap.require_team(ctx, Action::UpdateTeamSettings, team.id)?;
        let sun_thu = match input.work_week.as_deref() {
            None => None,
            Some("monToFri") => Some(false),
            Some("sunToThu") => Some(true),
            Some(_) => {
                return Err(invalid(
                    "workWeek",
                    "invalid",
                    "workWeek must be monToFri or sunToThu",
                ));
            }
        };
        let now = self.clock.now();
        tx.triage()
            .set_team_sla(team.id, input.enabled, sun_thu, now)
            .await?;
        if input.enabled == Some(true) && tx.triage().sla_rules(team.id).await?.is_empty() {
            let defaults: [(&str, Value, bool, Option<&str>); 3] = [
                (
                    "Urgent",
                    json!({ "priority": { "eq": 1 } }),
                    false,
                    Some("PT24H"),
                ),
                (
                    "High",
                    json!({ "priority": { "eq": 2 } }),
                    false,
                    Some("P1W"),
                ),
                (
                    "Others",
                    json!({ "priority": { "in": [0, 3, 4] } }),
                    true,
                    None,
                ),
            ];
            for (i, (name, filter, removes, duration)) in defaults.into_iter().enumerate() {
                tx.triage()
                    .insert_sla_rule(&NewSlaRule {
                        id: self.ids.new_uuid(),
                        team_id: team.id,
                        position: (i + 1) as f64 * 1000.0,
                        name: name.into(),
                        filter,
                        removes_sla: removes,
                        duration: duration.map(Into::into),
                        sla_type: SlaType::All,
                        start_mode: "issueCreation".into(),
                        now,
                    })
                    .await?;
            }
        }
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        let s = tx.triage().team_sla(team.id).await?;
        let rules = tx.triage().sla_rules(team.id).await?;
        tx.commit().await?;
        Ok(TeamSlaView {
            team_id: team.id,
            enabled: s.enabled,
            sun_thu: s.sun_thu,
            rules,
        })
    }

    /// Legt eine SLA-Regel am Ende der Liste an.
    pub async fn create_sla_rule(
        &self,
        ctx: &RequestCtx,
        input: CreateSlaRuleInput,
    ) -> AppResult<SlaRule> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, &input.team, Visibility::Content).await?;
        snap.require_team(ctx, Action::UpdateTeamSettings, team.id)?;
        let existing = tx.triage().sla_rules(team.id).await?;
        if existing.len() >= MAX_SLA_RULES {
            return Err(AppError::limit_exceeded("too many SLA rules")
                .with_details(json!({ "max": MAX_SLA_RULES })));
        }
        let rule = SlaRule {
            id: self.ids.new_uuid(),
            team_id: team.id,
            position: existing.iter().map(|r| r.position).fold(0.0, f64::max) + 1000.0,
            name: input.name.trim().to_owned(),
            filter: input.filter,
            removes_sla: input.removes_sla,
            duration: if input.removes_sla {
                None
            } else {
                input.duration
            },
            sla_type: input.sla_type,
            start_mode: input.start_mode.unwrap_or_else(|| "issueCreation".into()),
        };
        validate_rule(&rule)?;
        tx.triage()
            .insert_sla_rule(&NewSlaRule {
                id: rule.id,
                team_id: rule.team_id,
                position: rule.position,
                name: rule.name.clone(),
                filter: rule.filter.clone(),
                removes_sla: rule.removes_sla,
                duration: rule.duration.clone(),
                sla_type: rule.sla_type,
                start_mode: rule.start_mode.clone(),
                now: self.clock.now(),
            })
            .await?;
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        tx.commit().await?;
        Ok(rule)
    }

    /// Ändert eine SLA-Regel. Wirkt nicht rückwirkend (docs/03 §11.2).
    pub async fn update_sla_rule(
        &self,
        ctx: &RequestCtx,
        input: UpdateSlaRuleInput,
    ) -> AppResult<SlaRule> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, &input.team, Visibility::Content).await?;
        snap.require_team(ctx, Action::UpdateTeamSettings, team.id)?;
        let mut rule = tx
            .triage()
            .sla_rules(team.id)
            .await?
            .into_iter()
            .find(|r| r.id == input.id)
            .ok_or_else(|| not_found("SlaRule", &input.id.to_string()))?;
        if let Some(n) = input.name {
            rule.name = n.trim().to_owned();
        }
        if let Some(f) = input.filter {
            rule.filter = f;
        }
        if let Some(r) = input.removes_sla {
            rule.removes_sla = r;
        }
        if let Some(d) = input.duration {
            rule.duration = Some(d);
        }
        if rule.removes_sla {
            rule.duration = None;
        }
        if let Some(t) = input.sla_type {
            rule.sla_type = t;
        }
        if let Some(m) = input.start_mode {
            rule.start_mode = m;
        }
        if let Some(p) = input.position {
            if !p.is_finite() {
                return Err(invalid("position", "invalid", "position must be finite"));
            }
            rule.position = p;
        }
        validate_rule(&rule)?;
        if !tx.triage().save_sla_rule(&rule, self.clock.now()).await? {
            return Err(not_found("SlaRule", &rule.id.to_string()));
        }
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        tx.commit().await?;
        Ok(rule)
    }

    /// Löscht eine SLA-Regel; bereits gesetzte SLAs bleiben bestehen.
    pub async fn delete_sla_rule(
        &self,
        ctx: &RequestCtx,
        team_ref: &str,
        id: Uuid,
    ) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, team_ref, Visibility::Content).await?;
        snap.require_team(ctx, Action::UpdateTeamSettings, team.id)?;
        if !tx.triage().delete_sla_rule(team.id, id).await? {
            return Err(not_found("SlaRule", &id.to_string()));
        }
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        tx.commit().await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Wiederkehrende Issues
// ---------------------------------------------------------------------------

/// Eingabe von `create_recurring_issue`.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateRecurringInput {
    pub team: String,
    /// Issue-Template (UUID oder Name).
    pub template: String,
    /// `daily`, `weekly`, `monthly`.
    pub frequency: String,
    pub interval: Option<i32>,
    /// ISO-Wochentag 1..7 (Pflicht bei `weekly`).
    pub weekday: Option<i32>,
    /// 1..31 (Pflicht bei `monthly`).
    pub day_of_month: Option<i32>,
    /// Lokale Uhrzeit in Minuten seit Mitternacht (Team-Zeitzone).
    pub minute_of_day: i32,
    /// Ankerdatum (Default: heute in der Team-Zeitzone).
    pub start_date: Option<NaiveDate>,
}

/// Änderung eines wiederkehrenden Issues (`None` = unverändert).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateRecurringInput {
    pub id: Uuid,
    pub template: Option<String>,
    pub frequency: Option<String>,
    pub interval: Option<i32>,
    pub weekday: Option<i32>,
    pub day_of_month: Option<i32>,
    pub minute_of_day: Option<i32>,
    pub paused: Option<bool>,
}

fn days_in_month(y: i32, m: u32) -> u32 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1)
        .and_then(|d| d.pred_opt())
        .map_or(28, |d| d.day())
}

fn local_to_utc(tz: Tz, local: NaiveDateTime) -> Option<DateTime<Utc>> {
    use chrono::LocalResult;
    match tz.from_local_datetime(&local) {
        LocalResult::Single(t) => Some(t.with_timezone(&Utc)),
        LocalResult::Ambiguous(a, _) => Some(a.with_timezone(&Utc)),
        // Lücke der Sommerzeitumstellung: eine Stunde später.
        LocalResult::None => tz
            .from_local_datetime(&(local + Duration::hours(1)))
            .earliest()
            .map(|t| t.with_timezone(&Utc)),
    }
}

/// Der erste Ausführungszeitpunkt nach `after` (echt später) für den Rhythmus; `None` bei
/// ungültigen Angaben. Tage vor `start_date` zählen nicht.
#[allow(clippy::too_many_arguments)]
pub fn next_occurrence(
    frequency: &str,
    interval: i32,
    weekday: Option<i32>,
    day_of_month: Option<i32>,
    minute_of_day: i32,
    start_date: NaiveDate,
    tz: Tz,
    after: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    if interval < 1 || !(0..1440).contains(&minute_of_day) {
        return None;
    }
    let time = chrono::NaiveTime::from_hms_opt(
        u32::try_from(minute_of_day / 60).ok()?,
        u32::try_from(minute_of_day % 60).ok()?,
        0,
    )?;
    let first = after
        .with_timezone(&tz)
        .date_naive()
        .pred_opt()?
        .max(start_date);
    let interval = i64::from(interval);
    let start_monday =
        start_date - Duration::days(i64::from(start_date.weekday().num_days_from_monday()));
    let mut d = first;
    // Der längste Abstand zweier Läufe ist unter 365 Tage × 12; großzügige Obergrenze.
    for _ in 0..6000 {
        let hit = match frequency {
            "daily" => (d - start_date).num_days() % interval == 0,
            "weekly" => {
                let wd = i64::from(weekday?);
                let monday = d - Duration::days(i64::from(d.weekday().num_days_from_monday()));
                i64::from(d.weekday().number_from_monday()) == wd
                    && ((monday - start_monday).num_days() / 7) % interval == 0
            }
            "monthly" => {
                let dom = u32::try_from(day_of_month?).ok()?;
                let months = i64::from(d.year() - start_date.year()) * 12 + i64::from(d.month())
                    - i64::from(start_date.month());
                months % interval == 0 && d.day() == dom.min(days_in_month(d.year(), d.month()))
            }
            _ => return None,
        };
        if hit
            && let Some(t) = local_to_utc(tz, d.and_time(time))
            && t > after
        {
            return Some(t);
        }
        d = d.succ_opt()?;
    }
    None
}

fn validate_rhythm(r: &RecurringIssue) -> AppResult<()> {
    if !matches!(r.frequency.as_str(), "daily" | "weekly" | "monthly") {
        return Err(invalid(
            "frequency",
            "invalid",
            "frequency must be daily, weekly or monthly",
        ));
    }
    let max_interval = match r.frequency.as_str() {
        "daily" => 365,
        "weekly" => 52,
        _ => 12,
    };
    if !(1..=max_interval).contains(&r.interval_count) {
        return Err(invalid(
            "interval",
            "out_of_range",
            format!("interval must be between 1 and {max_interval}"),
        ));
    }
    if !(0..1440).contains(&r.minute_of_day) {
        return Err(invalid("time", "out_of_range", "time of day is invalid"));
    }
    match r.frequency.as_str() {
        "weekly" if !r.weekday.is_some_and(|w| (1..=7).contains(&w)) => Err(invalid(
            "weekday",
            "required",
            "weekday (1 = Monday … 7 = Sunday) is required for weekly",
        )),
        "monthly" if !r.day_of_month.is_some_and(|d| (1..=31).contains(&d)) => Err(invalid(
            "dayOfMonth",
            "required",
            "dayOfMonth (1–31) is required for monthly",
        )),
        _ => Ok(()),
    }
}

fn recompute(r: &mut RecurringIssue, tz: Tz, after: DateTime<Utc>) -> AppResult<()> {
    r.next_run_at = next_occurrence(
        &r.frequency,
        r.interval_count,
        r.weekday,
        r.day_of_month,
        r.minute_of_day,
        r.start_date,
        tz,
        after,
    )
    .ok_or_else(|| invalid("frequency", "invalid", "no next occurrence"))?;
    Ok(())
}

impl App {
    /// Wiederkehrende Issues eines Teams.
    pub async fn list_recurring_issues(
        &self,
        ctx: &RequestCtx,
        team_ref: &str,
    ) -> AppResult<Vec<RecurringIssue>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let team =
            resolve::team::resolve_in(&mut *tx, &snap, team_ref, Visibility::Content).await?;
        tx.triage().list_recurring(team.id).await
    }

    /// Legt ein wiederkehrendes Issue an. Das Template muss ein Issue-Template sein, das im Team
    /// verfügbar ist.
    pub async fn create_recurring_issue(
        &self,
        ctx: &RequestCtx,
        input: CreateRecurringInput,
    ) -> AppResult<RecurringIssue> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let probe = CreateIssueInput {
            team: input.team.clone(),
            ..Default::default()
        };
        let (tpl, team, _) =
            prepare_issue_template(&mut *tx, &snap, ctx, &input.template, &probe).await?;
        snap.require_team(ctx, Action::ManageTeamTemplates, team.id)?;
        if tx.triage().list_recurring(team.id).await?.len() >= MAX_RECURRING_PER_TEAM {
            return Err(AppError::limit_exceeded("too many recurring issues")
                .with_details(json!({ "max": MAX_RECURRING_PER_TEAM })));
        }
        let tz = tz_of(&team);
        let now = self.clock.now();
        let mut rec = RecurringIssue {
            id: self.ids.new_uuid(),
            team_id: team.id,
            template_id: tpl.id.0,
            template_name: tpl.name.clone(),
            frequency: input.frequency,
            interval_count: input.interval.unwrap_or(1),
            weekday: input.weekday,
            day_of_month: input.day_of_month,
            minute_of_day: input.minute_of_day,
            start_date: input
                .start_date
                .unwrap_or_else(|| now.with_timezone(&tz).date_naive()),
            paused: false,
            next_run_at: now,
            last_run_at: None,
            last_error: None,
            last_error_at: None,
            version: 1,
            created_at: now,
        };
        if rec.frequency != "weekly" {
            rec.weekday = None;
        }
        if rec.frequency != "monthly" {
            rec.day_of_month = None;
        }
        validate_rhythm(&rec)?;
        recompute(&mut rec, tz, now)?;
        tx.triage()
            .insert_recurring(&NewRecurringIssue {
                rec: rec.clone(),
                creator_id: actor_user_id(ctx),
                now,
            })
            .await?;
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        tx.commit().await?;
        Ok(rec)
    }

    /// Ändert Rhythmus, Template oder Pause. Rhythmus-Änderung und Fortsetzen berechnen den
    /// nächsten Lauf ab jetzt neu (kein Nachholen verpasster Läufe).
    pub async fn update_recurring_issue(
        &self,
        ctx: &RequestCtx,
        input: UpdateRecurringInput,
    ) -> AppResult<RecurringIssue> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let mut rec = tx
            .triage()
            .get_recurring(input.id)
            .await?
            .ok_or_else(|| not_found("RecurringIssue", &input.id.to_string()))?;
        // Unsichtbares Team → not_found, nicht forbidden.
        let team = resolve::team::resolve_in(
            &mut *tx,
            &snap,
            &rec.team_id.to_string(),
            Visibility::Content,
        )
        .await
        .map_err(|_| not_found("RecurringIssue", &input.id.to_string()))?;
        snap.require_team(ctx, Action::ManageTeamTemplates, team.id)?;
        let tz = tz_of(&team);
        let mut rhythm_changed = false;
        if let Some(t) = &input.template {
            let probe = CreateIssueInput {
                team: team.id.to_string(),
                ..Default::default()
            };
            let (tpl, _, _) = prepare_issue_template(&mut *tx, &snap, ctx, t, &probe).await?;
            rec.template_id = tpl.id.0;
            rec.template_name = tpl.name;
        }
        if let Some(f) = input.frequency {
            rec.frequency = f;
            rhythm_changed = true;
        }
        if let Some(i) = input.interval {
            rec.interval_count = i;
            rhythm_changed = true;
        }
        if let Some(w) = input.weekday {
            rec.weekday = Some(w);
            rhythm_changed = true;
        }
        if let Some(d) = input.day_of_month {
            rec.day_of_month = Some(d);
            rhythm_changed = true;
        }
        if let Some(m) = input.minute_of_day {
            rec.minute_of_day = m;
            rhythm_changed = true;
        }
        let now = self.clock.now();
        let mut resumed = false;
        if let Some(p) = input.paused {
            resumed = rec.paused && !p;
            rec.paused = p;
        }
        if rec.frequency != "weekly" {
            rec.weekday = None;
        }
        if rec.frequency != "monthly" {
            rec.day_of_month = None;
        }
        validate_rhythm(&rec)?;
        if rhythm_changed || resumed {
            recompute(&mut rec, tz, now)?;
        }
        if !tx.triage().save_recurring(&rec, now).await? {
            return Err(not_found("RecurringIssue", &rec.id.to_string()));
        }
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        tx.commit().await?;
        rec.version += 1;
        Ok(rec)
    }

    /// Löscht ein wiederkehrendes Issue (bereits erzeugte Issues bleiben).
    pub async fn delete_recurring_issue(&self, ctx: &RequestCtx, id: Uuid) -> AppResult<()> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let rec = tx
            .triage()
            .get_recurring(id)
            .await?
            .ok_or_else(|| not_found("RecurringIssue", &id.to_string()))?;
        let team = resolve::team::resolve_in(
            &mut *tx,
            &snap,
            &rec.team_id.to_string(),
            Visibility::Content,
        )
        .await
        .map_err(|_| not_found("RecurringIssue", &id.to_string()))?;
        snap.require_team(ctx, Action::ManageTeamTemplates, team.id)?;
        tx.triage().delete_recurring(id).await?;
        emit(&mut *tx, ctx, DomainEvent::TeamUpdated { team_id: team.id }).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Job: erzeugt für alle fälligen wiederkehrenden Issues je Workspace ein Issue (Akteur
    /// `system`). Verpasste Läufe werden nicht nachgeholt (ein Issue je Regel und Lauf).
    /// Schlägt die Erzeugung fehl (z. B. Template gelöscht), wird der Fehler an der Regel
    /// vermerkt und der Lauf übersprungen. Idempotent über `recurring_issue_run`.
    pub async fn run_recurring_issues(&self, now: DateTime<Utc>) -> AppResult<u64> {
        let mut sys = self.store.begin_system().await?;
        let orgs = sys.triage().orgs_with_recurring_due(now).await?;
        drop(sys);
        let mut created = 0;
        for org in orgs {
            let ctx = RequestCtx::system(org, "job-recurring-issues");
            for _ in 0..MAX_RUNS_PER_ORG {
                match self.run_one_recurring(&ctx, now).await {
                    // `Uuid::nil()` = Lauf übersprungen/Regel pausiert (nichts erzeugt).
                    Ok(Some(id)) => created += u64::from(!id.is_nil()),
                    Ok(None) => break,
                    Err(e) => {
                        tracing::warn!(org = %org, error = %e, "recurring issue run failed");
                    }
                }
            }
        }
        Ok(created)
    }

    /// Verarbeitet die erste fällige Regel; `None` = keine fällig.
    async fn run_one_recurring(
        &self,
        ctx: &RequestCtx,
        now: DateTime<Utc>,
    ) -> AppResult<Option<Uuid>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let Some(rec) = tx.triage().due_recurring(now, 1).await?.into_iter().next() else {
            return Ok(None);
        };
        let team = tx
            .teams()
            .get(rec.team_id)
            .await?
            .ok_or_else(|| AppError::internal("team of recurring issue is missing"))?;
        let tz = tz_of(&team);
        let next = next_occurrence(
            &rec.frequency,
            rec.interval_count,
            rec.weekday,
            rec.day_of_month,
            rec.minute_of_day,
            rec.start_date,
            tz,
            now,
        )
        .ok_or_else(|| AppError::internal("recurring issue has an invalid rhythm"))?;
        let result = self
            .create_recurring_occurrence(&mut *tx, ctx, &rec, &team)
            .await;
        match result {
            Ok(issue_id) => {
                tx.triage().finish_recurring_run(rec.id, now, next).await?;
                tx.commit().await?;
                Ok(Some(issue_id))
            }
            Err(e) => {
                // Fehler an der Regel vermerken, Lauf überspringen (eigene Transaktion).
                drop(tx);
                let mut tx = self.store.begin(ctx.org_id).await?;
                if e.kind == ErrorKind::NotFound {
                    // Template oder Team existiert nicht mehr: die Regel kann nie wieder laufen.
                    // Pausieren statt bei jedem Lauf still zu scheitern (Fehler bleibt sichtbar).
                    tx.triage()
                        .pause_recurring_run(rec.id, &e.to_string(), now)
                        .await?;
                } else {
                    tx.triage()
                        .skip_recurring_run(rec.id, next, &e.to_string(), now)
                        .await?;
                }
                tx.commit().await?;
                tracing::warn!(recurring = %rec.id, error = %e, "recurring issue skipped");
                Ok(Some(Uuid::nil()))
            }
        }
    }

    async fn create_recurring_occurrence(
        &self,
        tx: &mut dyn Tx,
        ctx: &RequestCtx,
        rec: &RecurringIssue,
        team: &Team,
    ) -> AppResult<Uuid> {
        let snap = PolicySnapshot::load(tx, ctx).await?;
        let input = CreateIssueInput {
            team: team.id.to_string(),
            source: HistorySource::Automation,
            ..Default::default()
        };
        let (tpl, team, data) =
            prepare_issue_template(tx, &snap, ctx, &rec.template_id.to_string(), &input).await?;
        let outcome = create_issue_tree(
            self,
            tx,
            &snap,
            ctx,
            &team,
            Some(&tpl),
            &data,
            input,
            &BTreeSet::new(),
            false,
        )
        .await?;
        let issue_id = outcome.root.issue.id;
        if !tx
            .triage()
            .claim_recurring_run(rec.id, rec.next_run_at, Some(issue_id))
            .await?
        {
            return Err(AppError::conflict(
                "this occurrence has already been created",
            ));
        }
        Ok(issue_id.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }
    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn daily_runs_next_day_when_time_has_passed() {
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        // 2026-03-02 10:00 Berlin = 09:00 UTC; Lauf um 08:00 lokal ist vorbei.
        let n = next_occurrence(
            "daily",
            1,
            None,
            None,
            480,
            d(2026, 3, 2),
            tz,
            utc("2026-03-02T09:00:00Z"),
        );
        assert_eq!(n, Some(utc("2026-03-03T07:00:00Z")));
        // Davor: noch heute.
        let n = next_occurrence(
            "daily",
            1,
            None,
            None,
            480,
            d(2026, 3, 2),
            tz,
            utc("2026-03-02T05:00:00Z"),
        );
        assert_eq!(n, Some(utc("2026-03-02T07:00:00Z")));
    }

    #[test]
    fn daily_interval_counts_from_start_date() {
        let n = next_occurrence(
            "daily",
            3,
            None,
            None,
            0,
            d(2026, 3, 1),
            chrono_tz::UTC,
            utc("2026-03-01T00:00:00Z"),
        );
        assert_eq!(n, Some(utc("2026-03-04T00:00:00Z")));
    }

    #[test]
    fn weekly_hits_the_weekday_and_respects_interval() {
        // 2026-03-02 ist ein Montag. Donnerstag (4), alle 2 Wochen ab Woche von 2026-03-02.
        let n = next_occurrence(
            "weekly",
            2,
            Some(4),
            None,
            600,
            d(2026, 3, 2),
            chrono_tz::UTC,
            utc("2026-03-02T00:00:00Z"),
        );
        assert_eq!(n, Some(utc("2026-03-05T10:00:00Z")));
        let n = next_occurrence(
            "weekly",
            2,
            Some(4),
            None,
            600,
            d(2026, 3, 2),
            chrono_tz::UTC,
            utc("2026-03-05T10:00:00Z"),
        );
        assert_eq!(n, Some(utc("2026-03-19T10:00:00Z")));
    }

    #[test]
    fn monthly_clamps_to_last_day_of_month() {
        let n = next_occurrence(
            "monthly",
            1,
            None,
            Some(31),
            0,
            d(2026, 1, 1),
            chrono_tz::UTC,
            utc("2026-01-31T00:00:00Z"),
        );
        assert_eq!(n, Some(utc("2026-02-28T00:00:00Z")));
        let n = next_occurrence(
            "monthly",
            1,
            None,
            Some(31),
            0,
            d(2026, 1, 1),
            chrono_tz::UTC,
            utc("2026-02-28T00:00:00Z"),
        );
        assert_eq!(n, Some(utc("2026-03-31T00:00:00Z")));
    }

    #[test]
    fn dst_gap_moves_one_hour_later_and_invalid_input_is_none() {
        let tz: Tz = "Europe/Berlin".parse().unwrap();
        // 2026-03-29 02:30 existiert in Berlin nicht (Umstellung 02:00 -> 03:00).
        let n = next_occurrence(
            "daily",
            1,
            None,
            None,
            150,
            d(2026, 3, 29),
            tz,
            utc("2026-03-28T23:00:00Z"),
        );
        assert_eq!(n, Some(utc("2026-03-29T01:30:00Z")));
        assert!(
            next_occurrence(
                "weekly",
                1,
                None,
                None,
                0,
                d(2026, 3, 1),
                tz,
                utc("2026-03-01T00:00:00Z")
            )
            .is_none()
        );
        assert!(
            next_occurrence(
                "daily",
                0,
                None,
                None,
                0,
                d(2026, 3, 1),
                tz,
                utc("2026-03-01T00:00:00Z")
            )
            .is_none()
        );
        assert!(
            next_occurrence(
                "daily",
                1,
                None,
                None,
                1440,
                d(2026, 3, 1),
                tz,
                utc("2026-03-01T00:00:00Z")
            )
            .is_none()
        );
    }

    #[test]
    fn start_date_in_the_future_delays_first_run() {
        let n = next_occurrence(
            "daily",
            1,
            None,
            None,
            0,
            d(2026, 4, 1),
            chrono_tz::UTC,
            utc("2026-03-01T12:00:00Z"),
        );
        assert_eq!(n, Some(utc("2026-04-01T00:00:00Z")));
    }
}
