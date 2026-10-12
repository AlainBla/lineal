//! SLA (docs/03 §11): Regelauswertung und Meldungen (Risikoschwellen, `IssueSla*`-Events).
//!
//! * [`evaluate_rules`] wendet die geordneten Team-Regeln auf ein Issue an (erste passende Regel
//!   gewinnt; Setz-Regeln überschreiben ein manuelles SLA nicht, Entfernen-Regeln schon).
//!   Aufgerufen bei Statuswechseln (`change_state`) und vom SLA-Job für kürzlich geänderte Issues
//!   (so greifen Regeln auch für Create/Updates, die nicht über `change_state` laufen).
//! * [`process_issue`] berechnet fehlende Schwellen und meldet „gesetzt“, „hohes Risiko“ und
//!   „Breach“ je SLA genau einmal (Meldemarken am Issue).

use crate::error::AppResult;
use crate::ports::Tx;
use crate::ports::repos::NewEvent;
use crate::ports::repos::issue::{HistorySource, Issue};
use crate::ports::repos::team::Team;
use crate::ports::repos::triage::{SlaMarker, SlaRule};
use crate::usecases::issue::history::{self, Delta};
use crate::usecases::triage::automation::is_closed;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use lineal_domain::enums::SlaType;
use lineal_domain::events::{DomainEvent, EventOrigin};
use lineal_domain::ids::LabelId;
use lineal_domain::rules::filter::IsoDuration;
use lineal_domain::rules::sla::{self, WorkWeek};
use serde_json::Value;
use uuid::Uuid;

const DAY_MS: i64 = 86_400_000;

fn tz_of(team: &Team) -> Tz {
    team.timezone.parse().unwrap_or(chrono_tz::UTC)
}

fn week_of(sun_thu: bool) -> WorkWeek {
    if sun_thu {
        WorkWeek::SunThu
    } else {
        WorkWeek::MonFri
    }
}

/// Dauer einer Regel: Millisekunden (`all`) bzw. Arbeitstage (`onlyBusinessDays`, eine Woche = 5).
fn amount_of(rule: &SlaRule) -> Option<i64> {
    let d = IsoDuration::parse(rule.duration.as_deref()?)?;
    if d.negative {
        return None;
    }
    let (weeks, days) = (i64::from(d.weeks), i64::from(d.days));
    match rule.sla_type {
        SlaType::All => {
            let days_total = i64::from(d.years)
                .checked_mul(365)?
                .checked_add(i64::from(d.months).checked_mul(30)?)?
                .checked_add(weeks.checked_mul(7)?)?
                .checked_add(days)?;
            days_total
                .checked_mul(DAY_MS)?
                .checked_add(i64::from(d.hours).checked_mul(3_600_000)?)?
                .checked_add(i64::from(d.minutes).checked_mul(60_000)?)?
                .checked_add(i64::from(d.seconds).checked_mul(1000)?)
        }
        SlaType::OnlyBusinessDays => weeks.checked_mul(5)?.checked_add(days),
    }
}

// --- Filter-Auswertung im Speicher (Teilmenge von docs/05 §1) ---------------------------------

fn set_match(cmp: &Value, actual: Option<&str>) -> bool {
    let Some(obj) = cmp.as_object() else {
        return false;
    };
    obj.iter().all(|(k, v)| match k.as_str() {
        "eq" => v.as_str() == actual,
        "neq" => v.as_str() != actual,
        "in" => v
            .as_array()
            .is_some_and(|a| actual.is_some_and(|x| a.iter().any(|e| e.as_str() == Some(x)))),
        "nin" => v
            .as_array()
            .is_some_and(|a| actual.is_none_or(|x| !a.iter().any(|e| e.as_str() == Some(x)))),
        "null" => v.as_bool().is_some_and(|b| b == actual.is_none()),
        _ => false,
    })
}

fn num_match(cmp: &Value, actual: i64) -> bool {
    let Some(obj) = cmp.as_object() else {
        return false;
    };
    let num = |v: &Value| v.as_i64();
    obj.iter().all(|(k, v)| match k.as_str() {
        "eq" => num(v) == Some(actual),
        "neq" => num(v).is_some_and(|n| n != actual),
        "in" => v
            .as_array()
            .is_some_and(|a| a.iter().any(|e| num(e) == Some(actual))),
        "nin" => v
            .as_array()
            .is_some_and(|a| !a.iter().any(|e| num(e) == Some(actual))),
        "lt" => num(v).is_some_and(|n| actual < n),
        "lte" => num(v).is_some_and(|n| actual <= n),
        "gt" => num(v).is_some_and(|n| actual > n),
        "gte" => num(v).is_some_and(|n| actual >= n),
        _ => false,
    })
}

/// To-One-Relation: `{ "id": <Set-Vergleich>, "null": bool }` sowie `or`/`and`.
fn relation_match(f: &Value, id: Option<String>) -> bool {
    let Some(obj) = f.as_object() else {
        return false;
    };
    obj.iter().all(|(k, v)| match k.as_str() {
        "id" => set_match(v, id.as_deref()),
        "null" => v.as_bool().is_some_and(|b| b == id.is_none()),
        "or" => v
            .as_array()
            .is_some_and(|a| a.is_empty() || a.iter().any(|x| relation_match(x, id.clone()))),
        "and" => v
            .as_array()
            .is_some_and(|a| a.iter().all(|x| relation_match(x, id.clone()))),
        _ => false,
    })
}

fn labels_match(f: &Value, labels: &[LabelId]) -> bool {
    let Some(obj) = f.as_object() else {
        return false;
    };
    obj.iter().all(|(k, v)| match k.as_str() {
        "some" => labels
            .iter()
            .any(|l| relation_match(v, Some(l.to_string()))),
        "every" => labels
            .iter()
            .all(|l| relation_match(v, Some(l.to_string()))),
        "length" => num_match(v, labels.len() as i64),
        _ => false,
    })
}

/// Trifft die Filterbedingung einer Regel auf das Issue zu? Unbekannte Felder treffen nie
/// (konservativ: eine Regel mit unverständlicher Bedingung setzt kein SLA).
pub(crate) fn rule_matches(filter: &Value, issue: &Issue, labels: &[LabelId]) -> bool {
    let Some(obj) = filter.as_object() else {
        return false;
    };
    obj.iter().all(|(k, v)| match k.as_str() {
        "and" => v
            .as_array()
            .is_some_and(|a| a.iter().all(|x| rule_matches(x, issue, labels))),
        "or" => v
            .as_array()
            .is_some_and(|a| a.is_empty() || a.iter().any(|x| rule_matches(x, issue, labels))),
        "priority" => num_match(v, i64::from(issue.priority.value())),
        "state" => v.as_object().is_some_and(|s| {
            s.iter().all(|(sk, sv)| match sk.as_str() {
                "type" => set_match(sv, Some(issue.state_type.as_str())),
                "id" => set_match(sv, Some(&issue.state_id.to_string())),
                _ => false,
            })
        }),
        "assignee" => relation_match(v, issue.assignee_id.map(|u| u.to_string())),
        "creator" => relation_match(v, issue.creator_id.map(|u| u.to_string())),
        "team" => relation_match(v, Some(issue.team_id.to_string())),
        "project" => relation_match(v, issue.project_id.map(|p| p.to_string())),
        "labels" => labels_match(v, labels),
        _ => false,
    })
}

fn clear(issue: &mut Issue) {
    issue.sla_type = None;
    issue.sla_started_at = None;
    issue.sla_medium_risk_at = None;
    issue.sla_high_risk_at = None;
    issue.sla_breaches_at = None;
}

/// Wertet die SLA-Regeln des Teams für ein Issue aus und ändert es im Speicher (der Aufrufer
/// speichert). `true`, wenn sich SLA-Felder geändert haben.
pub(crate) async fn evaluate_rules(
    tx: &mut dyn Tx,
    team: &Team,
    issue: &mut Issue,
    now: DateTime<Utc>,
) -> AppResult<bool> {
    if issue.trashed || issue.archived_at.is_some() || is_closed(issue.state_type) {
        return Ok(false);
    }
    let settings = tx.triage().team_sla(team.id).await?;
    if !settings.enabled {
        return Ok(false);
    }
    let rules = tx.triage().sla_rules(team.id).await?;
    if rules.is_empty() {
        return Ok(false);
    }
    let labels = tx.issues().label_ids(issue.id).await?;
    let auto = tx.triage().issue_automation(issue.id).await?;
    let manual = issue.sla_breaches_at.is_some() && auto.sla_rule_id.is_none();
    for rule in rules {
        if !rule_matches(&rule.filter, issue, &labels) {
            continue;
        }
        if rule.removes_sla {
            if issue.sla_breaches_at.is_some() {
                clear(issue);
                tx.triage().set_sla_rule(issue.id, None).await?;
                return Ok(true);
            }
        } else if !manual && auto.sla_rule_id != Some(rule.id) {
            let Some(amount) = amount_of(&rule) else {
                return Ok(false);
            };
            let start = if rule.start_mode == "ruleMatch" {
                now
            } else {
                issue.created_at
            };
            let (tz, week) = (tz_of(team), week_of(settings.sun_thu));
            let breach = sla::breach_at(start, amount, rule.sla_type, tz, week);
            let th = sla::risk_levels(start, breach, rule.sla_type, tz, week);
            issue.sla_type = Some(rule.sla_type);
            issue.sla_started_at = Some(start);
            issue.sla_breaches_at = Some(breach);
            issue.sla_medium_risk_at = Some(th.medium_risk_at);
            issue.sla_high_risk_at = Some(th.high_risk_at);
            // SLA und Fälligkeitsdatum schließen sich aus (docs/03 §2.6).
            issue.due_date = None;
            tx.triage().set_sla_rule(issue.id, Some(rule.id)).await?;
            return Ok(true);
        }
        // Nur die erste passende Regel greift.
        return Ok(false);
    }
    Ok(false)
}

async fn emit(tx: &mut dyn Tx, event: DomainEvent) -> AppResult<()> {
    tx.outbox()
        .push(NewEvent::new(event, EventOrigin::System))
        .await?;
    Ok(())
}

/// Berechnet fehlende Schwellen und meldet fällige SLA-Ereignisse. Liefert die Anzahl der
/// ausgelösten Meldungen (Schwellen-Nachberechnung zählt nicht).
pub(crate) async fn process_issue(
    tx: &mut dyn Tx,
    team: &Team,
    issue: &Issue,
    now: DateTime<Utc>,
) -> AppResult<u64> {
    let Some(breaches) = issue.sla_breaches_at else {
        return Ok(0);
    };
    if is_closed(issue.state_type) {
        return Ok(0);
    }
    let started = issue.sla_started_at.unwrap_or(issue.created_at);
    let ty = issue.sla_type.unwrap_or(SlaType::All);
    let (high, thresholds_changed) = match (issue.sla_medium_risk_at, issue.sla_high_risk_at) {
        (Some(_), Some(h)) => (h, false),
        _ => {
            let settings = tx.triage().team_sla(team.id).await?;
            let th = sla::risk_levels(
                started,
                breaches,
                ty,
                tz_of(team),
                week_of(settings.sun_thu),
            );
            tx.triage()
                .set_sla_thresholds(issue.id, th.medium_risk_at, th.high_risk_at)
                .await?;
            (th.high_risk_at, true)
        }
    };
    let auto = tx.triage().issue_automation(issue.id).await?;
    let mut fired = 0;
    if auto.sla_set_notified_at.is_none_or(|m| m < started) {
        emit(tx, DomainEvent::IssueSlaSet { issue_id: issue.id }).await?;
        tx.triage().mark_sla(issue.id, SlaMarker::Set, now).await?;
        fired += 1;
    }
    let high_pending = auto.sla_high_risk_notified_at.is_none_or(|m| m < high);
    if now >= breaches {
        if auto.sla_breached_notified_at.is_none_or(|m| m < breaches) {
            if high_pending {
                tx.triage()
                    .mark_sla(issue.id, SlaMarker::HighRisk, now)
                    .await?;
            }
            emit(tx, DomainEvent::IssueSlaBreached { issue_id: issue.id }).await?;
            tx.triage()
                .mark_sla(issue.id, SlaMarker::Breached, now)
                .await?;
            let mut delta = Delta::default();
            delta.flag("slaBreached");
            history::record(
                tx,
                Uuid::now_v7(),
                issue,
                None,
                HistorySource::System,
                &delta,
                now,
            )
            .await?;
            fired += 1;
        }
    } else if now >= high && high_pending {
        emit(tx, DomainEvent::IssueSlaHighRisk { issue_id: issue.id }).await?;
        tx.triage()
            .mark_sla(issue.id, SlaMarker::HighRisk, now)
            .await?;
        fired += 1;
    }
    if thresholds_changed {
        emit(
            tx,
            DomainEvent::IssueUpdated {
                previous: None,
                issue_id: issue.id,
                team_id: issue.team_id,
                changed: vec!["slaMediumRiskAt".into(), "slaHighRiskAt".into()],
                mentioned_user_ids: vec![],
            },
        )
        .await?;
    }
    Ok(fired)
}

/// Speichert ein per Regel geändertes Issue mit History und `IssueUpdated`.
pub(crate) async fn persist_rule_change(
    tx: &mut dyn Tx,
    old: &Issue,
    issue: &Issue,
    now: DateTime<Utc>,
) -> AppResult<Issue> {
    let saved = tx.issues().save(issue, now).await?;
    let ts = |v: Option<DateTime<Utc>>| {
        v.map_or(Value::Null, |t| {
            Value::String(t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        })
    };
    let mut d = Delta::default();
    d.set(
        "slaStartedAt",
        ts(old.sla_started_at),
        ts(saved.sla_started_at),
    );
    d.set(
        "slaBreachesAt",
        ts(old.sla_breaches_at),
        ts(saved.sla_breaches_at),
    );
    d.set(
        "slaType",
        serde_json::json!(old.sla_type.map(|t| t.as_str())),
        serde_json::json!(saved.sla_type.map(|t| t.as_str())),
    );
    d.set(
        "dueDate",
        serde_json::json!(old.due_date.map(|x| x.to_string())),
        serde_json::json!(saved.due_date.map(|x| x.to_string())),
    );
    let changed: Vec<String> = d.changes.keys().cloned().collect();
    history::record(
        tx,
        Uuid::now_v7(),
        &saved,
        None,
        HistorySource::Automation,
        &d,
        now,
    )
    .await?;
    if !changed.is_empty() {
        emit(
            tx,
            DomainEvent::IssueUpdated {
                previous: None,
                issue_id: saved.id,
                team_id: saved.team_id,
                changed,
                mentioned_user_ids: vec![],
            },
        )
        .await?;
    }
    Ok(saved)
}
