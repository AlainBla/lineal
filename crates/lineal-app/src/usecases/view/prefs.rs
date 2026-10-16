//! Anzeigeoptionen (docs/05 §2): Schlüssel, Wertebereiche, System-Defaults und die Zusammenführung
//! `shallowMerge(sys, org, mine)` mit `sanitize`.
//!
//! Rein (keine I/O), damit die Regeln als Tabellentests prüfbar sind.

use crate::error::{AppError, AppResult};
use serde_json::{Map, Value, json};

/// Kanonische vordefinierte Ansichten (docs/05 §3.5).
pub const VIEW_TYPES: [&str; 12] = [
    "teamTriage",
    "teamActive",
    "teamBacklog",
    "teamAll",
    "teamCurrentCycle",
    "teamUpcomingCycle",
    "myIssuesAssigned",
    "projectsAll",
    "teamProjects",
    "initiativesActive",
    "initiativesPlanned",
    "initiativesAll",
];

/// Issue-Tab einer Projektseite; Ziel ist das Projekt (`view_target_id`). Nur für Anzeigeoptionen,
/// keine favorisierbare Ansicht (daher nicht in [`VIEW_TYPES`]).
pub const PROJECT_ISSUES: &str = "projectIssues";

/// Entität, auf die sich die Optionen beziehen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Issues,
    Projects,
    Initiatives,
}

/// Art einer vordefinierten Ansicht; `None` für unbekannte Typen.
pub fn kind_of_view_type(t: &str) -> Option<Kind> {
    match t {
        "teamTriage" | "teamActive" | "teamBacklog" | "teamAll" | "teamCurrentCycle"
        | "teamUpcomingCycle" | "myIssuesAssigned" | PROJECT_ISSUES => Some(Kind::Issues),
        "projectsAll" | "teamProjects" => Some(Kind::Projects),
        "initiativesActive" | "initiativesPlanned" | "initiativesAll" => Some(Kind::Initiatives),
        _ => None,
    }
}

const ISSUE_GROUPING: [&str; 13] = [
    "none",
    "status",
    "assignee",
    "project",
    "priority",
    "cycle",
    "label",
    "labelGroup",
    "parentIssue",
    "team",
    "milestone",
    "slaStatus",
    "focus",
];
const PROJECT_GROUPING: [&str; 12] = [
    "none",
    "status",
    "initiative",
    "lead",
    "member",
    "priority",
    "label",
    "leadTeam",
    "team",
    "health",
    "startDate",
    "targetDate",
];
const ISSUE_ORDERING: [&str; 12] = [
    "manual",
    "title",
    "status",
    "priority",
    "assignee",
    "estimate",
    "updatedAt",
    "createdAt",
    "dueDate",
    "linkCount",
    "timeInStatus",
    "importance",
];
const PROJECT_ORDERING: [&str; 9] = [
    "manual",
    "name",
    "status",
    "priority",
    "updatedAt",
    "createdAt",
    "healthUpdatedAt",
    "startDate",
    "targetDate",
];

/// Anzeige-Eigenschaften der Issue-Zeile (docs/05 §2.1 „Display properties“).
pub const DISPLAY_PROPERTIES: [&str; 14] = [
    "id",
    "status",
    "assignee",
    "creator",
    "priority",
    "dueDate",
    "milestone",
    "cycle",
    "labels",
    "links",
    "timeInStatus",
    "created",
    "updated",
    "pullRequests",
];

/// Voreingestellte Anzeige-Eigenschaften.
const DEFAULT_DISPLAY_PROPERTIES: [&str; 8] = [
    "id",
    "status",
    "assignee",
    "priority",
    "dueDate",
    "milestone",
    "labels",
    "updated",
];

fn bad(key: &str, reason: impl Into<String>) -> AppError {
    AppError::invalid_request(format!("Invalid display option `{key}`: {}", reason.into()))
        .with_details(json!({ "field": key }))
}

/// `labelGroup:<uuid>` ist nur mit gültiger UUID erlaubt.
fn grouping_ok(kind: Kind, v: &str, allow_focus: bool) -> bool {
    if let Some(id) = v.strip_prefix("labelGroup:") {
        return kind == Kind::Issues && uuid::Uuid::parse_str(id).is_ok();
    }
    match kind {
        Kind::Issues => {
            ISSUE_GROUPING.contains(&v) && v != "labelGroup" && (allow_focus || v != "focus")
        }
        _ => PROJECT_GROUPING.contains(&v),
    }
}

fn enum_of<'a>(m: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    m.get(key).and_then(Value::as_str)
}

/// System-Default je Ansicht (docs/05 §2.1).
pub fn system_defaults(view_type: Option<&str>, kind: Kind) -> Map<String, Value> {
    let mut m = Map::new();
    let board = matches!(
        view_type,
        Some("teamActive" | "teamCurrentCycle" | "teamUpcomingCycle")
    );
    m.insert("layout".into(), json!(if board { "board" } else { "list" }));
    m.insert(
        "grouping".into(),
        json!(if kind == Kind::Issues {
            "status"
        } else {
            "none"
        }),
    );
    m.insert("groupOrdering".into(), json!("default"));
    m.insert("subGrouping".into(), json!("none"));
    let ordering = match (kind, board, view_type) {
        (Kind::Issues, _, Some("myIssuesAssigned")) => "importance",
        (Kind::Issues, true, _) => "manual",
        (Kind::Issues, false, _) => "priority",
        _ => "manual",
    };
    m.insert("ordering".into(), json!(ordering));
    m.insert("orderCompletedByRecency".into(), json!(false));
    m.insert(
        "showCompletedIssues".into(),
        json!(if view_type == Some("myIssuesAssigned") {
            "pastDay"
        } else {
            "all"
        }),
    );
    m.insert("showSubIssues".into(), json!(true));
    m.insert("nestedSubIssues".into(), json!("hide"));
    m.insert("showEmptyGroups".into(), json!(false));
    m.insert("showEmptySubGroups".into(), json!(false));
    m.insert("showTriageIssues".into(), json!(false));
    m.insert("showClosedProjects".into(), json!("all"));
    m.insert("groupCountMode".into(), json!("count"));
    m.insert(
        "displayProperties".into(),
        json!(DEFAULT_DISPLAY_PROPERTIES),
    );
    m
}

/// Prüft eine Änderung (nur gesetzte Schlüssel) und liefert sie als Map. Unbekannte Schlüssel
/// und Werte außerhalb des Wertebereichs → `invalid_request`.
pub fn validate_patch(
    patch: &Value,
    kind: Kind,
    view_type: Option<&str>,
) -> AppResult<Map<String, Value>> {
    let Some(obj) = patch.as_object() else {
        return Err(AppError::invalid_request(
            "preferences must be a JSON object",
        ));
    };
    let my_issues = view_type == Some("myIssuesAssigned");
    let mut out = Map::new();
    for (k, v) in obj {
        let ok = match k.as_str() {
            "layout" => v.as_str().is_some_and(|s| {
                s == "list" || s == "board" || (s == "timeline" && kind != Kind::Issues)
            }),
            "grouping" => v.as_str().is_some_and(|s| grouping_ok(kind, s, my_issues)),
            "subGrouping" => v.as_str().is_some_and(|s| grouping_ok(kind, s, false)),
            "groupOrdering" => matches!(v.as_str(), Some("default" | "reverse")),
            "ordering" => v.as_str().is_some_and(|s| match kind {
                Kind::Issues => ISSUE_ORDERING.contains(&s) && (s != "importance" || my_issues),
                _ => PROJECT_ORDERING.contains(&s),
            }),
            "orderDirection" => matches!(v.as_str(), Some("asc" | "desc")),
            "showCompletedIssues" => matches!(
                v.as_str(),
                Some("all" | "pastDay" | "pastWeek" | "pastMonth" | "currentCycle" | "none")
            ),
            "nestedSubIssues" => matches!(v.as_str(), Some("showAll" | "showMatching" | "hide")),
            "showClosedProjects" => matches!(
                v.as_str(),
                Some("all" | "pastWeek" | "pastMonth" | "pastYear" | "none")
            ),
            "displayProperties" => v.as_array().is_some_and(|a| {
                a.len() <= DISPLAY_PROPERTIES.len()
                    && a.iter()
                        .all(|x| x.as_str().is_some_and(|s| DISPLAY_PROPERTIES.contains(&s)))
            }),
            "groupCountMode" => matches!(v.as_str(), Some("count" | "estimateSum")),
            "orderCompletedByRecency"
            | "showSubIssues"
            | "showEmptyGroups"
            | "showEmptySubGroups"
            | "showTriageIssues" => v.is_boolean(),
            _ => return Err(bad(k, "unknown option")),
        };
        if !ok {
            return Err(bad(k, format!("value {v} is not allowed here")));
        }
        out.insert(k.clone(), v.clone());
    }
    Ok(out)
}

/// `shallowMerge(sys, org, mine)` (später gewinnt, Schlüssel für Schlüssel) und `sanitize`:
/// unzulässige Kombinationen fallen auf den Default zurück.
pub fn effective(
    sys: Map<String, Value>,
    org: Option<&Value>,
    mine: Option<&Value>,
) -> Map<String, Value> {
    let mut m = sys;
    for layer in [org, mine].into_iter().flatten() {
        if let Some(o) = layer.as_object() {
            for (k, v) in o {
                m.insert(k.clone(), v.clone());
            }
        }
    }
    // Sub-Gruppierung muss sich von der Gruppierung unterscheiden.
    if enum_of(&m, "subGrouping") == enum_of(&m, "grouping")
        && enum_of(&m, "subGrouping") != Some("none")
    {
        m.insert("subGrouping".into(), json!("none"));
    }
    // Manuelle Sortierung hat keine Richtung (docs/05 §2.1).
    if enum_of(&m, "ordering") == Some("manual") {
        m.remove("orderDirection");
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_table() {
        let issues = system_defaults(None, Kind::Issues);
        assert_eq!(issues["layout"], "list");
        assert_eq!(issues["grouping"], "status");
        assert_eq!(issues["ordering"], "priority");
        assert_eq!(issues["showCompletedIssues"], "all");
        assert_eq!(issues["showSubIssues"], true);
        let board = system_defaults(Some("teamActive"), Kind::Issues);
        assert_eq!(
            (board["layout"].as_str(), board["ordering"].as_str()),
            (Some("board"), Some("manual"))
        );
        let my = system_defaults(Some("myIssuesAssigned"), Kind::Issues);
        assert_eq!(my["ordering"], "importance");
        assert_eq!(my["showCompletedIssues"], "pastDay");
        assert_eq!(system_defaults(None, Kind::Projects)["grouping"], "none");
    }

    #[test]
    fn validation_rejects_unknown_keys_and_values() {
        let ok = |p: Value, k: Kind, vt: Option<&str>| validate_patch(&p, k, vt).is_ok();
        assert!(ok(
            json!({"layout": "board", "grouping": "assignee"}),
            Kind::Issues,
            None
        ));
        assert!(!ok(json!({"layout": "timeline"}), Kind::Issues, None));
        assert!(ok(json!({"layout": "timeline"}), Kind::Projects, None));
        assert!(!ok(json!({"bogus": 1}), Kind::Issues, None));
        assert!(ok(
            json!({"displayProperties": ["id", "labels"]}),
            Kind::Issues,
            None
        ));
        assert!(!ok(
            json!({"displayProperties": ["id", "nope"]}),
            Kind::Issues,
            None
        ));
        assert!(!ok(json!({"displayProperties": "id"}), Kind::Issues, None));
        assert!(!ok(json!({"showSubIssues": "yes"}), Kind::Issues, None));
        // `focus` und `importance` nur in My Issues.
        assert!(!ok(json!({"grouping": "focus"}), Kind::Issues, None));
        assert!(ok(
            json!({"grouping": "focus"}),
            Kind::Issues,
            Some("myIssuesAssigned")
        ));
        assert!(!ok(
            json!({"ordering": "importance"}),
            Kind::Issues,
            Some("teamAll")
        ));
        // Label-Gruppe braucht eine UUID.
        assert!(ok(
            json!({"grouping": "labelGroup:5d0a5d0a-0000-4000-8000-000000000001"}),
            Kind::Issues,
            None
        ));
        assert!(!ok(
            json!({"grouping": "labelGroup:abc"}),
            Kind::Issues,
            None
        ));
        // Projekt-Gruppierungen gelten nicht für Issues.
        assert!(!ok(json!({"grouping": "health"}), Kind::Issues, None));
        assert!(ok(json!({"grouping": "health"}), Kind::Projects, None));
    }

    #[test]
    fn later_layers_win_and_sanitize_resets_invalid_combinations() {
        let sys = system_defaults(None, Kind::Issues);
        let org = json!({"grouping": "assignee", "ordering": "updatedAt", "orderDirection": "asc"});
        let mine = json!({"ordering": "manual", "subGrouping": "assignee"});
        let m = effective(sys, Some(&org), Some(&mine));
        assert_eq!(m["grouping"], "assignee");
        // Sub-Gruppierung gleich Gruppierung → none.
        assert_eq!(m["subGrouping"], "none");
        // manual: keine Richtung.
        assert!(!m.contains_key("orderDirection"));
        assert_eq!(m["ordering"], "manual");
    }
}
