//! CSV-Adapter (Linear-CSV-Export): nur Issues mit Basisfeldern (docs/13 §2).
//!
//! Aus den Zeilen werden Teams, Platzhalter-User (nur Namen, ohne E-Mail: `…@import.invalid`),
//! Status (Typ aus dem Namen geraten), Labels und Issues im normalisierten Format erzeugt. Alles
//! andere (Kommentare, Projekte, Cycles, Anhänge, Historie) fehlt im CSV und wird als Warnung
//! gemeldet.

use super::fake::FakeSource;
use crate::port::{
    Downloaded, ImportSource, PageQuery, SourceError, SourcePage, SourceResult, SourceWorkspace,
};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Höchstzahl importierter Zeilen (Schutz vor Missbrauch).
pub const MAX_ROWS: usize = 200_000;

/// Linear-CSV als Quelle.
pub struct CsvSource {
    inner: FakeSource,
    notices: Vec<String>,
    /// Zeilenwarnungen (stille Datenänderungen); werden einmal über `take_warnings` gemeldet.
    row_warnings: std::sync::Mutex<Vec<String>>,
}

/// Obergrenze der einzeln gemeldeten Zeilenwarnungen (danach eine Sammelmeldung).
const MAX_ROW_WARNINGS: usize = 500;

/// Typ-Heuristik kannte den Statusnamen nicht (Standardtyp `unstarted` geraten).
fn state_type_is_guess(name: &str) -> bool {
    let n = name.to_lowercase();
    state_type(name) == "unstarted"
        && !["todo", "to do", "unstarted", "open", "ready", "planned", "new"]
            .iter()
            .any(|k| n.contains(k))
}

/// Ist die Priorität ein erkannter Wert (`urgent`, `high`, `medium`, `low`, `no priority`, 0-4)?
fn priority_is_known(s: &str) -> bool {
    let p = s.trim().to_lowercase();
    matches!(
        p.as_str(),
        "urgent" | "high" | "medium" | "low" | "no priority" | "none" | ""
    ) || p.parse::<i64>().is_ok_and(|n| (0..=4).contains(&n))
}

fn slug(s: &str) -> String {
    let mut out: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').chars().take(40).collect()
}

/// RFC 3339 oder die JavaScript-Schreibweise des Linear-Exports.
fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(t) = s.parse::<DateTime<Utc>>() {
        return Some(t);
    }
    // `Tue Jan 07 2025 10:00:00 GMT+0000 (Coordinated Universal Time)`
    let head = s.split(" (").next().unwrap_or(s);
    DateTime::parse_from_str(head, "%a %b %d %Y %H:%M:%S GMT%z")
        .ok()
        .map(|t| t.with_timezone(&Utc))
        .or_else(|| {
            NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|n| n.and_utc())
        })
}

fn priority(s: &str) -> i64 {
    match s.trim().to_lowercase().as_str() {
        "urgent" => 1,
        "high" => 2,
        "medium" => 3,
        "low" => 4,
        other => other
            .parse::<i64>()
            .ok()
            .filter(|p| (0..=4).contains(p))
            .unwrap_or(0),
    }
}

fn state_type(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.contains("triage") {
        "triage"
    } else if n.contains("backlog") {
        "backlog"
    } else if n.contains("done")
        || n.contains("complete")
        || n.contains("released")
        || n.contains("merged")
    {
        "completed"
    } else if n.contains("cancel")
        || n.contains("duplicate")
        || n.contains("won't")
        || n.contains("wont")
    {
        "canceled"
    } else if n.contains("progress")
        || n.contains("review")
        || n.contains("doing")
        || n.contains("testing")
    {
        "started"
    } else {
        "unstarted"
    }
}

impl CsvSource {
    /// Liest den CSV-Inhalt. `team_filter` (Key) beschränkt auf ein Team.
    pub fn from_csv(bytes: &[u8], team_filter: Option<&str>) -> Result<Self, String> {
        let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(bytes);
        let headers: Vec<String> = rdr
            .headers()
            .map_err(|_| "the CSV header cannot be read".to_owned())?
            .iter()
            .map(|h| h.trim().to_lowercase())
            .collect();
        let col = |name: &str| headers.iter().position(|h| h == name);
        let id_col = col("id").ok_or("the CSV has no `ID` column (is this a Linear export?)")?;
        let title_col = col("title").ok_or("the CSV has no `Title` column")?;
        let get = |rec: &csv::StringRecord, name: &str| -> String {
            col(name)
                .and_then(|i| rec.get(i))
                .unwrap_or("")
                .trim()
                .to_owned()
        };
        let ws_name = "Linear CSV";
        let mut teams: BTreeMap<String, String> = BTreeMap::new();
        let mut users: BTreeMap<String, String> = BTreeMap::new();
        let mut states: BTreeMap<(String, String), &'static str> = BTreeMap::new();
        let mut labels: BTreeSet<String> = BTreeSet::new();
        let mut issues: Vec<Value> = Vec::new();
        let mut state_pos: BTreeMap<String, f64> = BTreeMap::new();
        let mut had_projects = false;
        let mut row_warnings: Vec<String> = Vec::new();
        let mut warned_states: BTreeSet<(String, String)> = BTreeSet::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut dropped_warnings = 0usize;
        let mut warn = |msg: String| {
            if row_warnings.len() < MAX_ROW_WARNINGS {
                row_warnings.push(msg);
            } else {
                dropped_warnings += 1;
            }
        };
        for (n_row, rec) in rdr.records().enumerate() {
            if n_row >= MAX_ROWS {
                return Err(format!("the CSV has more than {MAX_ROWS} rows"));
            }
            let rec = rec.map_err(|_| format!("row {} cannot be read", n_row + 2))?;
            let ident = rec.get(id_col).unwrap_or("").trim().to_uppercase();
            let row_no = n_row + 2;
            let parsed = ident
                .rsplit_once('-')
                .and_then(|(key, num)| Some((key, num.parse::<i64>().ok()?)))
                .filter(|(key, number)| !key.is_empty() && *number > 0);
            let Some((key, number)) = parsed else {
                if !ident.is_empty() {
                    warn(format!(
                        "row {row_no}: `{}` is not a valid issue ID (expected KEY-123); the row was skipped",
                        ident.chars().take(40).collect::<String>()
                    ));
                }
                continue;
            };
            if team_filter.is_some_and(|t| !t.eq_ignore_ascii_case(key)) {
                continue;
            }
            if !seen.insert(ident.clone()) {
                warn(format!(
                    "row {row_no}: duplicate ID {ident}; the first row was kept and this row was ignored"
                ));
                continue;
            }
            let team_name = get(&rec, "team");
            teams.entry(key.to_owned()).or_insert_with(|| {
                if team_name.is_empty() {
                    key.to_owned()
                } else {
                    team_name.clone()
                }
            });
            let team_id = format!("csv-team-{key}");
            let mut node = Map::new();
            node.insert("id".into(), json!(format!("csv-issue-{ident}")));
            node.insert("number".into(), json!(number));
            node.insert("identifier".into(), json!(ident));
            let title = rec.get(title_col).unwrap_or("");
            if title.trim().is_empty() {
                warn(format!("row {row_no} ({ident}): empty title; `(untitled)` was used"));
            } else if title.chars().count() > 255 {
                warn(format!(
                    "row {row_no} ({ident}): the title is longer than 255 characters and was truncated"
                ));
            }
            node.insert("title".into(), json!(title));
            node.insert("description".into(), json!(get(&rec, "description")));
            node.insert("team".into(), json!({ "id": team_id }));
            let status = get(&rec, "status");
            if !status.is_empty() {
                let ty = state_type(&status);
                if state_type_is_guess(&status)
                    && warned_states.insert((key.to_owned(), status.clone()))
                {
                    warn(format!(
                        "status `{}` of team {key} is not a known name; it was imported as type `unstarted`",
                        status.chars().take(60).collect::<String>()
                    ));
                }
                states.entry((key.to_owned(), status.clone())).or_insert(ty);
                let pos = state_pos.len() as f64;
                state_pos.entry(format!("{key}/{status}")).or_insert(pos);
                node.insert(
                    "state".into(),
                    json!({ "id": format!("csv-state-{key}-{}", slug(&status)) }),
                );
            }
            let prio = get(&rec, "priority");
            if !priority_is_known(&prio) {
                warn(format!(
                    "row {row_no} ({ident}): unknown priority `{}`; `No priority` was used",
                    prio.chars().take(30).collect::<String>()
                ));
            }
            node.insert("priority".into(), json!(priority(&prio)));
            let estimate = get(&rec, "estimate");
            match estimate.parse::<f64>() {
                Ok(e) => {
                    node.insert("estimate".into(), json!(e));
                }
                Err(_) if !estimate.is_empty() => warn(format!(
                    "row {row_no} ({ident}): the estimate `{}` is not a number and was ignored",
                    estimate.chars().take(30).collect::<String>()
                )),
                Err(_) => {}
            }
            for (csv_name, field) in [("assignee", "assignee"), ("creator", "creator")] {
                let name = get(&rec, csv_name);
                if !name.is_empty() {
                    let uid = format!("csv-user-{}", slug(&name));
                    users.entry(uid.clone()).or_insert(name);
                    node.insert(field.into(), json!({ "id": uid }));
                }
            }
            let lbls: Vec<String> = get(&rec, "labels")
                .split(',')
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect();
            let mut label_refs = Vec::new();
            for l in &lbls {
                labels.insert(l.clone());
                label_refs.push(json!({ "id": format!("csv-label-{}", slug(l)) }));
            }
            node.insert("labels".into(), Value::Array(label_refs));
            for (csv_name, field) in [
                ("created", "createdAt"),
                ("updated", "updatedAt"),
                ("started", "startedAt"),
                ("triaged", "triagedAt"),
                ("completed", "completedAt"),
                ("canceled", "canceledAt"),
                ("archived", "archivedAt"),
            ] {
                let raw = get(&rec, csv_name);
                if let Some(t) = parse_ts(&raw) {
                    node.insert(field.into(), json!(t.to_rfc3339()));
                } else if !raw.is_empty() {
                    warn(format!(
                        "row {row_no} ({ident}): the {csv_name} date `{}` is invalid and was ignored",
                        raw.chars().take(40).collect::<String>()
                    ));
                }
            }
            let due = get(&rec, "due date");
            if let Some(t) = parse_ts(&due) {
                node.insert("dueDate".into(), json!(t.date_naive().to_string()));
            } else if let Some(d) = due
                .get(..10)
                .filter(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok())
            {
                node.insert("dueDate".into(), json!(d));
            } else if !due.is_empty() {
                warn(format!(
                    "row {row_no} ({ident}): the due date `{}` is invalid and was ignored",
                    due.chars().take(40).collect::<String>()
                ));
            }
            let parent = get(&rec, "parent issue").to_uppercase();
            if !parent.is_empty() {
                node.insert(
                    "parent".into(),
                    json!({ "id": format!("csv-issue-{parent}") }),
                );
            }
            if !get(&rec, "project").is_empty() || !get(&rec, "project id").is_empty() {
                had_projects = true;
            }
            issues.push(Value::Object(node));
        }
        if issues.is_empty() {
            return Err("the CSV contains no importable issues".into());
        }
        issues.sort_by_key(|v| v["number"].as_i64().unwrap_or(0));
        let user_nodes: Vec<Value> = users
            .iter()
            .map(|(id, name)| {
                let s = id.trim_start_matches("csv-user-");
                json!({ "id": id, "name": name, "displayName": s, "email": format!("{s}@import.invalid"), "active": true })
            })
            .collect();
        let team_nodes: Vec<Value> = teams
            .iter()
            .map(|(key, name)| json!({ "id": format!("csv-team-{key}"), "key": key, "name": name, "private": false }))
            .collect();
        let state_nodes: Vec<Value> = states
            .iter()
            .map(|((key, name), ty)| {
                let pos = state_pos.get(&format!("{key}/{name}")).copied().unwrap_or(0.0);
                json!({ "id": format!("csv-state-{key}-{}", slug(name)), "name": name, "type": ty,
                        "color": "#95A2B3", "position": pos, "team": { "id": format!("csv-team-{key}") } })
            })
            .collect();
        let label_nodes: Vec<Value> = labels
            .iter()
            .map(|l| json!({ "id": format!("csv-label-{}", slug(l)), "name": l, "color": "#95A2B3" }))
            .collect();
        let fixture = json!({
            "workspace": { "id": "csv", "name": ws_name, "urlKey": "" },
            "entities": {
                "User": user_nodes, "Team": team_nodes, "WorkflowState": state_nodes,
                "IssueLabel": label_nodes, "Issue": issues,
            },
        });
        let inner = FakeSource::from_json(&fixture)?.with_kind("linearCsv");
        let mut notices = vec![
            "CSV import: only issues with basic fields are available; comments, attachments, history, projects, cycles, initiatives, documents, subscriptions and exact users (no e-mail addresses) are missing".to_owned(),
        ];
        if had_projects {
            notices.push("CSV import: the project assignment of issues is not imported".to_owned());
        }
        if dropped_warnings > 0 {
            row_warnings.push(format!(
                "{dropped_warnings} further row warnings were omitted"
            ));
        }
        Ok(Self {
            inner,
            notices,
            row_warnings: std::sync::Mutex::new(row_warnings),
        })
    }

    /// Warnungen über fehlende Daten (für CLI und Bericht).
    pub fn warnings(&self) -> &[String] {
        &self.notices
    }
}

#[async_trait]
impl ImportSource for CsvSource {
    fn kind(&self) -> &'static str {
        "linearCsv"
    }

    fn notices(&self) -> Vec<String> {
        self.notices.clone()
    }

    fn take_warnings(&self) -> Vec<String> {
        std::mem::take(
            &mut *self
                .row_warnings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    async fn workspace(&self) -> SourceResult<SourceWorkspace> {
        self.inner.workspace().await
    }

    async fn page(&self, q: &PageQuery) -> SourceResult<SourcePage> {
        self.inner.page(q).await
    }

    async fn download(&self, _url: &str) -> SourceResult<Downloaded> {
        Err(SourceError::Rejected(
            "downloads are not available for CSV imports".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::port::SourceEntity;

    const CSV: &str = "ID,Team,Title,Description,Status,Estimate,Priority,Project,Creator,Assignee,Labels,Created,Updated,Completed,Due Date,Parent issue\n\
ENG-1,Engineering,First,Body one,Todo,3,High,,Alice,Bob,\"Bug, Api\",2025-01-06T09:00:00.000Z,2025-01-07T09:00:00.000Z,,2025-02-01,\n\
ENG-2,Engineering,Second,,Done,,Urgent,Roadmap,Bob,,,Tue Jan 07 2025 10:00:00 GMT+0000 (Coordinated Universal Time),2025-01-08T09:00:00.000Z,2025-01-08T09:00:00.000Z,,ENG-1\n\
WEB-7,Web,Other team,,In Progress,,,,,,,2025-01-09T09:00:00.000Z,2025-01-09T09:00:00.000Z,,,\n";

    #[tokio::test]
    async fn csv_rows_become_normalized_nodes() {
        let src = CsvSource::from_csv(CSV.as_bytes(), None).unwrap();
        assert_eq!(src.kind(), "linearCsv");
        assert!(!src.warnings().is_empty());
        let issues = src.collect(SourceEntity::Issues, None, 5).await.unwrap();
        assert_eq!(issues.len(), 3);
        assert_eq!(issues[0]["identifier"], "ENG-1");
        assert_eq!(issues[0]["priority"], 2);
        assert_eq!(issues[0]["labels"].as_array().unwrap().len(), 2);
        assert_eq!(issues[1]["parent"]["id"], "csv-issue-ENG-1");
        assert_eq!(issues[1]["createdAt"], "2025-01-07T10:00:00+00:00");
        let teams = src.collect(SourceEntity::Teams, None, 5).await.unwrap();
        assert_eq!(teams.len(), 2);
        let states = src
            .collect(SourceEntity::WorkflowStates, None, 5)
            .await
            .unwrap();
        assert!(
            states
                .iter()
                .any(|s| s["name"] == "Done" && s["type"] == "completed")
        );
        let users = src.collect(SourceEntity::Users, None, 5).await.unwrap();
        assert!(
            users
                .iter()
                .all(|u| u["email"].as_str().unwrap().ends_with("@import.invalid"))
        );
    }

    #[tokio::test]
    async fn silent_data_changes_are_reported_as_row_warnings() {
        let long = "x".repeat(300);
        let csv = format!(
            "ID,Team,Title,Status,Priority,Estimate,Created,Due Date\n\
ENG-1,Eng,First,Todo,High,3,2025-01-06T09:00:00.000Z,2025-02-01\n\
ENG-1,Eng,Duplicate,Todo,High,3,2025-01-06T09:00:00.000Z,2025-02-01\n\
ENG-2,Eng,{long},Weird,Extreme,many,not-a-date,someday\n\
bogus,Eng,Bad id,Todo,,,,\n"
        );
        let src = CsvSource::from_csv(csv.as_bytes(), None).unwrap();
        let w = src.take_warnings().join("\n");
        for needle in [
            "duplicate ID ENG-1",
            "longer than 255",
            "status `Weird`",
            "unknown priority `Extreme`",
            "estimate `many`",
            "created date `not-a-date`",
            "due date `someday`",
            "`BOGUS` is not a valid issue ID",
        ] {
            assert!(w.contains(needle), "missing `{needle}` in:\n{w}");
        }
        assert!(src.take_warnings().is_empty(), "warnings are reported once");
        let issues = src.collect(SourceEntity::Issues, None, 5).await.unwrap();
        assert_eq!(issues.len(), 2, "the duplicate row was dropped");
    }

    #[tokio::test]
    async fn team_filter_and_errors() {
        let src = CsvSource::from_csv(CSV.as_bytes(), Some("web")).unwrap();
        assert_eq!(
            src.collect(SourceEntity::Issues, None, 5)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(CsvSource::from_csv(b"a,b\n1,2\n", None).is_err());
        assert!(CsvSource::from_csv(CSV.as_bytes(), Some("NOPE")).is_err());
        assert!(
            src.download("https://uploads.linear.app/x.png")
                .await
                .is_err()
        );
    }
}
