//! Reine Hilfen der Planung (ohne I/O): Datumsauflösung, Projektstatus-Sätze, Meilenstein-Status,
//! Status-Update-Diff, Staleness, Health-Rollup (docs/04 §2.3, §2.5, §3.1, §4.5, §5.3, §5.5).

use crate::error::{AppError, AppResult};
use crate::policy::TeamTree;
use crate::ports::repos::project::ProjectStatus;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use lineal_domain::enums::{DateResolution, Health, ProjectStatusType};
use lineal_domain::ids::TeamId;
use lineal_domain::rules::patch::{PatchError, PatchFailure};
use serde_json::{Value, json};
use uuid::Uuid;

/// Bildet einen Patch-Fehler auf das Fehlermodell ab (docs/07 §2.6).
pub fn patch_error(e: PatchError) -> AppError {
    let details = json!({
        "opIndex": e.op_index, "op": e.op, "anchor": e.anchor, "matches": e.matches,
    });
    match e.reason {
        PatchFailure::NoOps | PatchFailure::TooManyOps | PatchFailure::EmptyArgument => {
            AppError::invalid_request(e.to_string()).with_details(details)
        }
        PatchFailure::AnchorNotFound | PatchFailure::AnchorAmbiguous => {
            AppError::conflict(e.to_string()).with_details(details)
        }
        PatchFailure::LimitExceeded => {
            AppError::limit_exceeded(e.to_string()).with_details(details)
        }
    }
}

fn last_day_of_month(year: i32, month: u32) -> NaiveDate {
    let (y, m) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(y, m, 1)
        .and_then(|d| d.pred_opt())
        .unwrap_or_default()
}

/// Längengrenze für Rich-Text-Felder (Zeichen); darüber `limit_exceeded` (B5.31).
pub fn check_len(field: &str, s: &str, max: usize) -> AppResult<()> {
    if s.chars().count() > max {
        return Err(
            AppError::limit_exceeded(format!("`{field}` exceeds {max} characters"))
                .with_details(json!({ "field": field, "limit": max })),
        );
    }
    Ok(())
}

/// Periodenlänge der Auflösung in Monaten.
fn period_months(res: DateResolution) -> u32 {
    match res {
        DateResolution::Month => 1,
        DateResolution::Quarter => 3,
        DateResolution::HalfYear => 6,
        DateResolution::Year => 12,
    }
}

/// Erster Monat (Jahr, Monat 1-12) der Periode, in der `date` liegt. Quartale, Halbjahre und
/// Jahre richten sich nach `fiscal` (`Organization.fiscalYearStartMonth`, 0 = Januar ... 11).
fn period_start(date: NaiveDate, res: DateResolution, fiscal: u32) -> (i32, u32) {
    let m0 = date.month0();
    let f = fiscal % 12;
    let rel = (m0 + 12 - f) % 12;
    let len = period_months(res);
    let start0 = (f + rel / len * len) % 12;
    let year = if start0 > m0 {
        date.year() - 1
    } else {
        date.year()
    };
    (year, start0 + 1)
}

/// Normalisiert ein Startdatum (docs/04 §2.5): erster Tag der Auflösung.
pub fn normalize_start(date: NaiveDate, res: Option<DateResolution>, fiscal: u32) -> NaiveDate {
    let Some(res) = res else { return date };
    let (y, m) = period_start(date, res, fiscal);
    NaiveDate::from_ymd_opt(y, m, 1).unwrap_or(date)
}

/// Normalisiert ein Zieldatum (docs/04 §2.5): letzter Tag der Auflösung.
pub fn normalize_target(date: NaiveDate, res: Option<DateResolution>, fiscal: u32) -> NaiveDate {
    let Some(res) = res else { return date };
    let (y, m) = period_start(date, res, fiscal);
    let end_m0 = m - 1 + period_months(res) - 1;
    last_day_of_month(y + (end_m0 / 12) as i32, end_m0 % 12 + 1)
}

/// Anzeige eines Datums nach Auflösung (`Q4 2026`, `Jan 2027`, `Nov 14, 2026`). Fiskalquartale
/// tragen das Jahr, in dem das Geschäftsjahr endet.
pub fn display_date(date: NaiveDate, res: Option<DateResolution>, fiscal: u32) -> String {
    let f = fiscal % 12;
    let rel = (date.month0() + 12 - f) % 12;
    let fiscal_year = if f > 0 && date.month0() >= f {
        date.year() + 1
    } else {
        date.year()
    };
    match res {
        None => date.format("%b %-d, %Y").to_string(),
        Some(DateResolution::Month) => date.format("%b %Y").to_string(),
        Some(DateResolution::Quarter) => format!("Q{} {}", rel / 3 + 1, fiscal_year),
        Some(DateResolution::HalfYear) => format!("H{} {}", rel / 6 + 1, fiscal_year),
        Some(DateResolution::Year) => fiscal_year.to_string(),
    }
}

/// Parst `YYYY-MM-DD`; sonst `invalid_request` mit `field`.
pub fn parse_date(field: &str, s: &str) -> AppResult<NaiveDate> {
    lineal_domain::rules::date::parse_iso_date(field, s).map_err(|_| {
        AppError::invalid_request(format!(
            "`{field}` must be a date (YYYY-MM-DD, year 1900-2200)"
        ))
        .with_details(json!({ "field": field }))
    })
}

/// Start-/Zieldatum samt Auflösung validieren und normalisieren. Liefert
/// `(start, start_res, target, target_res)` des Endzustands.
#[allow(clippy::type_complexity)]
pub fn resolve_dates(
    start: Option<NaiveDate>,
    start_res: Option<DateResolution>,
    target: Option<NaiveDate>,
    target_res: Option<DateResolution>,
    fiscal: u32,
) -> AppResult<(
    Option<NaiveDate>,
    Option<DateResolution>,
    Option<NaiveDate>,
    Option<DateResolution>,
)> {
    if start.is_none() && start_res.is_some() {
        return Err(
            AppError::invalid_request("`startDateResolution` requires `startDate`")
                .with_details(json!({ "field": "startDateResolution" })),
        );
    }
    if target.is_none() && target_res.is_some() {
        return Err(
            AppError::invalid_request("`targetDateResolution` requires `targetDate`")
                .with_details(json!({ "field": "targetDateResolution" })),
        );
    }
    let start = start.map(|d| normalize_start(d, start_res, fiscal));
    let target = target.map(|d| normalize_target(d, target_res, fiscal));
    if let (Some(s), Some(t)) = (start, target)
        && s > t
    {
        return Err(
            AppError::invalid_request("`startDate` must not be after `targetDate`")
                .with_details(json!({ "field": "startDate" })),
        );
    }
    Ok((start, start_res, target, target_res))
}

/// Effektiver Projektstatus-Satz eines Teams (docs/04 §2.3 `effectiveStatusSet`).
pub fn effective_status_set(
    all: &[ProjectStatus],
    tree: &TeamTree,
    team: Option<TeamId>,
) -> Vec<ProjectStatus> {
    let workspace = || -> Vec<ProjectStatus> {
        all.iter()
            .filter(|s| s.team_id.is_none() && s.archived_at.is_none())
            .cloned()
            .collect()
    };
    let Some(team_id) = team else {
        return workspace();
    };
    let Some(t) = tree.get(team_id) else {
        return workspace();
    };
    if t.inherit_project_statuses {
        return match t.parent_id {
            Some(p) => effective_status_set(all, tree, Some(p)),
            None => workspace(),
        };
    }
    let own: Vec<ProjectStatus> = all
        .iter()
        .filter(|s| s.team_id == Some(team_id) && s.archived_at.is_none())
        .cloned()
        .collect();
    if own.is_empty() { workspace() } else { own }
}

/// Löst eine Status-Referenz gegen einen Satz auf: UUID, Name (case-insensitiv), Kategoriename.
pub fn resolve_status_ref<'a>(
    set: &'a [ProjectStatus],
    reference: &str,
) -> Option<&'a ProjectStatus> {
    let r = reference.trim();
    if let Ok(id) = Uuid::parse_str(r) {
        return set.iter().find(|s| s.id == id);
    }
    let lower = r.to_lowercase();
    if let Some(s) = set.iter().find(|s| s.name.to_lowercase() == lower) {
        return Some(s);
    }
    let kind = r.parse::<ProjectStatusType>().ok()?;
    set.iter().find(|s| s.status_type == kind)
}

/// Neuzuordnung des Status in einen neuen Satz (docs/04 §2.3 `remapStatus`).
pub fn remap_status<'a>(
    current: &ProjectStatus,
    new_set: &'a [ProjectStatus],
) -> Option<&'a ProjectStatus> {
    if let Some(s) = new_set.iter().find(|s| s.id == current.id) {
        return Some(s);
    }
    let name = current.name.to_lowercase();
    if let Some(s) = new_set
        .iter()
        .find(|s| s.status_type == current.status_type && s.name.to_lowercase() == name)
    {
        return Some(s);
    }
    new_set
        .iter()
        .find(|s| s.status_type == current.status_type)
}

/// Berechneter Status eines Meilensteins (docs/04 §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MilestoneStatus {
    Unstarted,
    Next,
    Overdue,
    Done,
}

impl MilestoneStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Unstarted => "unstarted",
            Self::Next => "next",
            Self::Overdue => "overdue",
            Self::Done => "done",
        }
    }
}

/// Eingabe je Meilenstein (in `sortOrder`-Reihenfolge).
#[derive(Debug, Clone, Copy)]
pub struct MilestoneStatusInput {
    pub target_date: Option<NaiveDate>,
    /// Es gibt relevante Issues (nicht `canceled`/`duplicate`).
    pub has_relevant_issues: bool,
    /// Alle relevanten Issues sind `completed`.
    pub all_completed: bool,
}

/// Status aller Meilensteine eines Projekts (docs/04 §3.1 `milestoneStatuses`).
pub fn milestone_statuses(
    items: &[MilestoneStatusInput],
    today: NaiveDate,
) -> Vec<MilestoneStatus> {
    let mut next_assigned = false;
    items
        .iter()
        .map(|m| {
            if m.has_relevant_issues && m.all_completed {
                MilestoneStatus::Done
            } else if m.target_date.is_some_and(|d| d < today) {
                MilestoneStatus::Overdue
            } else if !next_assigned {
                next_assigned = true;
                MilestoneStatus::Next
            } else {
                MilestoneStatus::Unstarted
            }
        })
        .collect()
}

/// Zustand der Aktualität eines Projekts/einer Initiative (docs/04 §5.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Staleness {
    Fresh,
    /// „Update missing“.
    Missing,
    /// Länger überfällig, Health-Icon grau.
    Grey,
    /// Keine Erwartung (abgeschlossen, abgebrochen, Erinnerungen aus).
    NoUpdateExpected,
}

/// Staleness nach docs/04 §5.5 / A19: überfällig = Erinnerungszyklus plus 3 Tage; grau nach
/// zwei Zyklen plus 3 Tagen. Nur wenn das letzte Update `onTrack` war, gilt „Update missing“.
pub fn staleness(
    expected: bool,
    cycle_days: Option<i64>,
    last_update: Option<(DateTime<Utc>, Health)>,
    now: DateTime<Utc>,
) -> Staleness {
    let Some(days) = cycle_days.filter(|_| expected) else {
        return Staleness::NoUpdateExpected;
    };
    let Some((at, health)) = last_update else {
        return Staleness::Fresh;
    };
    let overdue = now - at;
    if overdue > Duration::days(2 * days + 3) {
        Staleness::Grey
    } else if overdue > Duration::days(days + 3) && health == Health::OnTrack {
        Staleness::Missing
    } else {
        Staleness::Fresh
    }
}

/// Health-Rollup einer Initiative (docs/04 §4.5): schlechtester Wert aktiver, nicht veralteter Projekte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HealthCounts {
    pub on_track: u32,
    pub at_risk: u32,
    pub off_track: u32,
    pub none: u32,
}

impl HealthCounts {
    /// `None` = grau.
    pub fn color(&self) -> Option<Health> {
        if self.off_track > 0 {
            Some(Health::OffTrack)
        } else if self.at_risk > 0 {
            Some(Health::AtRisk)
        } else if self.on_track > 0 {
            Some(Health::OnTrack)
        } else {
            None
        }
    }
}

/// Zählt Health-Werte: `None` bzw. veraltet zählt als „none“.
pub fn health_rollup(items: impl IntoIterator<Item = Option<Health>>) -> HealthCounts {
    let mut c = HealthCounts::default();
    for h in items {
        match h {
            Some(Health::OnTrack) => c.on_track += 1,
            Some(Health::AtRisk) => c.at_risk += 1,
            Some(Health::OffTrack) => c.off_track += 1,
            None => c.none += 1,
        }
    }
    c
}

/// Snapshot-Teil eines Meilensteins im Update-Snapshot.
fn ms_map(snapshot: &Value) -> Vec<(String, String, f64)> {
    snapshot
        .get("milestones")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    Some((
                        m.get("id")?.as_str()?.to_owned(),
                        m.get("name")?.as_str()?.to_owned(),
                        m.get("progress")?.as_f64()?,
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn res_of(v: &Value, key: &str) -> Option<DateResolution> {
    v.get(key)
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
}

fn date_of(v: &Value, key: &str) -> Option<NaiveDate> {
    v.get(key)
        .and_then(Value::as_str)
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
}

/// Diff zwischen zwei Snapshots (docs/04 §5.3). `Some((diff, markdown))`, wenn sich etwas geändert hat.
/// Der Gesamtfortschritt zählt erst ab mehr als 2 Prozentpunkten Änderung; Meilensteine nur bei Projekten.
pub fn compute_diff(
    prev: &Value,
    cur: &Value,
    with_progress: bool,
    fiscal: u32,
) -> Option<(Value, String)> {
    let mut diff = serde_json::Map::new();
    let mut lines: Vec<String> = Vec::new();

    let name = |v: &Value| {
        v.get("status")
            .and_then(|s| s.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    if name(prev) != name(cur) {
        diff.insert(
            "status".into(),
            json!({ "from": name(prev), "to": name(cur) }),
        );
        lines.push(format!(
            "- Status changed from **{}** to **{}**",
            name(prev).unwrap_or_else(|| "none".into()),
            name(cur).unwrap_or_else(|| "none".into())
        ));
    }

    let (pd, pr) = (
        date_of(prev, "targetDate"),
        res_of(prev, "targetDateResolution"),
    );
    let (cd, cr) = (
        date_of(cur, "targetDate"),
        res_of(cur, "targetDateResolution"),
    );
    if (pd, pr) != (cd, cr) {
        diff.insert(
            "targetDate".into(),
            json!({
                "from": pd.map(|d| d.to_string()), "to": cd.map(|d| d.to_string()),
                "fromResolution": pr.map(|r| r.as_str()), "toResolution": cr.map(|r| r.as_str()),
            }),
        );
        let show = |d: Option<NaiveDate>, r| {
            d.map(|d| display_date(d, r, fiscal))
                .unwrap_or_else(|| "none".into())
        };
        lines.push(format!(
            "- Target date moved from **{}** to **{}**",
            show(pd, pr),
            show(cd, cr)
        ));
    }

    let lead_key = if cur.get("leadId").is_some() {
        "leadId"
    } else {
        "ownerId"
    };
    let (pl, cl) = (prev.get(lead_key).cloned(), cur.get(lead_key).cloned());
    if pl != cl && !(pl.is_none() && cl.is_none()) {
        diff.insert("lead".into(), json!({ "from": pl, "to": cl }));
        let label = if lead_key == "leadId" {
            "Lead"
        } else {
            "Owner"
        };
        let who = |v: &Value, id_key: &str, name_key: &str| {
            v.get(name_key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| v.get(id_key).and_then(Value::as_str).map(str::to_owned))
                .unwrap_or_else(|| "none".into())
        };
        let name_key = if lead_key == "leadId" {
            "leadName"
        } else {
            "ownerName"
        };
        lines.push(format!(
            "- {label} changed from **{}** to **{}**",
            who(prev, lead_key, name_key),
            who(cur, lead_key, name_key)
        ));
    }

    if with_progress {
        let pp = prev.get("progress").and_then(Value::as_f64);
        let cp = cur.get("progress").and_then(Value::as_f64);
        if let (Some(p), Some(c)) = (pp, cp)
            && (c - p).abs() > 0.02
        {
            diff.insert("progress".into(), json!({ "from": p, "to": c }));
            lines.push(format!(
                "- Progress {}% → {}%",
                (p * 100.0).round() as i64,
                (c * 100.0).round() as i64
            ));
        }
        let before = ms_map(prev);
        let mut ms_changes = Vec::new();
        for (id, name, progress) in ms_map(cur) {
            match before.iter().find(|(i, _, _)| *i == id) {
                None => {
                    ms_changes.push(json!({ "id": id, "name": name, "new": true,
                        "progress": { "from": null, "to": progress }, "completed": progress >= 1.0 }));
                    lines.push(format!("- Milestone **{name}** added"));
                }
                Some((_, _, old)) if (progress - old).abs() > f64::EPSILON => {
                    let completed = progress >= 1.0 && *old < 1.0;
                    ms_changes.push(json!({ "id": id, "name": name,
                        "progress": { "from": old, "to": progress }, "completed": progress >= 1.0 }));
                    if completed {
                        lines.push(format!("- Milestone **{name}** completed"));
                    } else {
                        lines.push(format!(
                            "- Milestone **{name}** {}% → {}%",
                            (old * 100.0).round() as i64,
                            (progress * 100.0).round() as i64
                        ));
                    }
                }
                _ => {}
            }
        }
        if !ms_changes.is_empty() {
            diff.insert("milestones".into(), Value::Array(ms_changes));
        }
    }

    if diff.is_empty() {
        None
    } else {
        Some((Value::Object(diff), lines.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn resolution_normalizes_dates() {
        use DateResolution::*;
        let x = d(2026, 11, 14);
        assert_eq!(normalize_start(x, None, 0), x);
        assert_eq!(normalize_start(x, Some(Month), 0), d(2026, 11, 1));
        assert_eq!(normalize_target(x, Some(Month), 0), d(2026, 11, 30));
        assert_eq!(normalize_start(x, Some(Quarter), 0), d(2026, 10, 1));
        assert_eq!(normalize_target(x, Some(Quarter), 0), d(2026, 12, 31));
        assert_eq!(normalize_start(x, Some(HalfYear), 0), d(2026, 7, 1));
        assert_eq!(
            normalize_target(d(2026, 3, 3), Some(HalfYear), 0),
            d(2026, 6, 30)
        );
        assert_eq!(normalize_start(x, Some(Year), 0), d(2026, 1, 1));
        assert_eq!(normalize_target(x, Some(Year), 0), d(2026, 12, 31));
        assert_eq!(
            normalize_target(d(2028, 2, 3), Some(Month), 0),
            d(2028, 2, 29)
        );
    }

    #[test]
    fn fiscal_year_start_shifts_quarters_halves_and_years() {
        use DateResolution::*;
        // B5.05: Fiskaljahr ab Februar (1): Quartale Feb-Apr, Mai-Jul, Aug-Okt, Nov-Jan.
        let x = d(2026, 11, 20);
        assert_eq!(normalize_start(x, Some(Quarter), 1), d(2026, 11, 1));
        assert_eq!(normalize_target(x, Some(Quarter), 1), d(2027, 1, 31));
        assert_eq!(
            normalize_target(d(2027, 1, 5), Some(Quarter), 1),
            d(2027, 1, 31)
        );
        assert_eq!(
            normalize_start(d(2027, 1, 5), Some(Quarter), 1),
            d(2026, 11, 1)
        );
        assert_eq!(normalize_start(x, Some(HalfYear), 1), d(2026, 8, 1));
        assert_eq!(normalize_target(x, Some(HalfYear), 1), d(2027, 1, 31));
        assert_eq!(normalize_start(d(2027, 1, 5), Some(Year), 1), d(2026, 2, 1));
        assert_eq!(
            normalize_target(d(2027, 1, 5), Some(Year), 1),
            d(2027, 1, 31)
        );
        assert_eq!(
            normalize_target(d(2026, 6, 5), Some(Year), 1),
            d(2027, 1, 31)
        );
        // Oktober-Start (9).
        assert_eq!(
            normalize_target(d(2026, 11, 20), Some(Quarter), 9),
            d(2026, 12, 31)
        );
        assert_eq!(
            normalize_start(d(2026, 11, 20), Some(Quarter), 9),
            d(2026, 10, 1)
        );
        assert_eq!(display_date(d(2027, 1, 31), Some(Quarter), 1), "Q4 2027");
        assert_eq!(display_date(d(2026, 11, 1), Some(Quarter), 1), "Q4 2027");
    }

    #[test]
    fn display_follows_resolution() {
        assert_eq!(
            display_date(d(2026, 12, 31), Some(DateResolution::Quarter), 0),
            "Q4 2026"
        );
        assert_eq!(
            display_date(d(2027, 1, 31), Some(DateResolution::Month), 0),
            "Jan 2027"
        );
        assert_eq!(display_date(d(2026, 11, 14), None, 0), "Nov 14, 2026");
        assert_eq!(
            display_date(d(2026, 12, 31), Some(DateResolution::HalfYear), 0),
            "H2 2026"
        );
        assert_eq!(
            display_date(d(2026, 12, 31), Some(DateResolution::Year), 0),
            "2026"
        );
    }

    #[test]
    fn dates_are_validated() {
        // Auflösung ohne Datum
        assert!(resolve_dates(None, Some(DateResolution::Month), None, None, 0).is_err());
        // Start nach Ziel (nach Normalisierung)
        assert!(resolve_dates(Some(d(2026, 12, 5)), None, Some(d(2026, 12, 1)), None, 0).is_err());
        let (s, _, t, _) = resolve_dates(
            Some(d(2026, 10, 14)),
            Some(DateResolution::Quarter),
            Some(d(2026, 11, 14)),
            Some(DateResolution::Quarter),
            0,
        )
        .unwrap();
        assert_eq!((s, t), (Some(d(2026, 10, 1)), Some(d(2026, 12, 31))));
    }

    #[test]
    fn milestone_status_rules() {
        let today = d(2026, 10, 10);
        let items = [
            MilestoneStatusInput {
                target_date: None,
                has_relevant_issues: true,
                all_completed: true,
            },
            MilestoneStatusInput {
                target_date: Some(d(2026, 10, 1)),
                has_relevant_issues: true,
                all_completed: false,
            },
            MilestoneStatusInput {
                target_date: None,
                has_relevant_issues: false,
                all_completed: false,
            },
            MilestoneStatusInput {
                target_date: None,
                has_relevant_issues: true,
                all_completed: false,
            },
        ];
        let s = milestone_statuses(&items, today);
        assert_eq!(
            s,
            [
                MilestoneStatus::Done,
                MilestoneStatus::Overdue,
                MilestoneStatus::Next,
                MilestoneStatus::Unstarted
            ]
        );
        // Ein Meilenstein ohne Issues ist nie done.
        let only = [MilestoneStatusInput {
            target_date: None,
            has_relevant_issues: false,
            all_completed: true,
        }];
        assert_eq!(milestone_statuses(&only, today), [MilestoneStatus::Next]);
    }

    fn st(name: &str, kind: ProjectStatusType, team: Option<TeamId>) -> ProjectStatus {
        ProjectStatus {
            id: Uuid::new_v4(),
            name: name.into(),
            status_type: kind,
            color: "#000000".into(),
            description: None,
            position: 1.0,
            indefinite: true,
            team_id: team,
            inherited_from_id: None,
            archived_at: None,
        }
    }

    #[test]
    fn status_resolution_and_remap() {
        let ws = [
            st("Backlog", ProjectStatusType::Backlog, None),
            st("In Progress", ProjectStatusType::Started, None),
        ];
        assert_eq!(
            resolve_status_ref(&ws, "in progress").map(|s| s.name.as_str()),
            Some("In Progress")
        );
        assert_eq!(
            resolve_status_ref(&ws, "started").map(|s| s.name.as_str()),
            Some("In Progress")
        );
        assert_eq!(
            resolve_status_ref(&ws, &ws[0].id.to_string()).map(|s| s.name.as_str()),
            Some("Backlog")
        );
        assert!(resolve_status_ref(&ws, "nope").is_none());
        let team = TeamId::new_v7();
        let own = [
            st("Doing", ProjectStatusType::Started, Some(team)),
            st("In Progress", ProjectStatusType::Started, Some(team)),
        ];
        // gleicher Name und gleiche Kategorie gewinnt vor erster Status der Kategorie
        assert_eq!(
            remap_status(&ws[1], &own).map(|s| s.name.as_str()),
            Some("In Progress")
        );
        // sonst erster der Kategorie
        let own2 = [st("Doing", ProjectStatusType::Started, Some(team))];
        assert_eq!(
            remap_status(&ws[1], &own2).map(|s| s.name.as_str()),
            Some("Doing")
        );
        assert!(remap_status(&ws[0], &own2).is_none());
    }

    #[test]
    fn diff_lists_status_target_progress_and_milestones() {
        let prev = json!({
            "status": { "name": "Planned" }, "targetDate": "2026-12-31", "targetDateResolution": "quarter",
            "progress": 0.18, "milestones": [{ "id": "m1", "name": "Beta", "progress": 0.5 }]
        });
        let cur = json!({
            "status": { "name": "In Progress" }, "targetDate": "2027-01-31", "targetDateResolution": "month",
            "progress": 0.42, "milestones": [{ "id": "m1", "name": "Beta", "progress": 1.0 }]
        });
        let (diff, md) = compute_diff(&prev, &cur, true, 0).unwrap();
        assert_eq!(diff["status"]["to"], "In Progress");
        assert_eq!(diff["targetDate"]["toResolution"], "month");
        assert_eq!(diff["milestones"][0]["completed"], true);
        assert_eq!(
            md,
            "- Status changed from **Planned** to **In Progress**\n\
             - Target date moved from **Q4 2026** to **Jan 2027**\n\
             - Progress 18% → 42%\n\
             - Milestone **Beta** completed"
        );
        // Fortschritt unter 2 Prozentpunkten wird nicht aufgenommen.
        let small = json!({ "progress": 0.19 });
        assert!(compute_diff(&json!({ "progress": 0.18 }), &small, true, 0).is_none());
        assert!(compute_diff(&cur, &cur, true, 0).is_none());
    }

    #[test]
    fn staleness_rules() {
        let now = Utc::now();
        let day = |n: i64| now - Duration::days(n);
        // Zyklus 7 Tage: ab 10 Tagen "missing" (nur bei onTrack), ab 17 Tagen grau
        assert_eq!(
            staleness(true, Some(7), Some((day(5), Health::OnTrack)), now),
            Staleness::Fresh
        );
        assert_eq!(
            staleness(true, Some(7), Some((day(11), Health::OnTrack)), now),
            Staleness::Missing
        );
        assert_eq!(
            staleness(true, Some(7), Some((day(11), Health::AtRisk)), now),
            Staleness::Fresh
        );
        assert_eq!(
            staleness(true, Some(7), Some((day(18), Health::AtRisk)), now),
            Staleness::Grey
        );
        assert_eq!(
            staleness(false, Some(7), None, now),
            Staleness::NoUpdateExpected
        );
        assert_eq!(
            staleness(true, None, None, now),
            Staleness::NoUpdateExpected
        );
    }

    #[test]
    fn rollup_picks_worst() {
        let c = health_rollup([Some(Health::OnTrack), Some(Health::AtRisk), None]);
        assert_eq!(c.color(), Some(Health::AtRisk));
        assert_eq!((c.on_track, c.at_risk, c.none), (1, 1, 1));
        assert_eq!(health_rollup([None]).color(), None);
        assert_eq!(
            health_rollup([Some(Health::OffTrack), Some(Health::OnTrack)]).color(),
            Some(Health::OffTrack)
        );
    }
}
