//! `templateData` je Typ (docs/06 §2.4): Strukturen mit `deny_unknown_fields`, Validierung und
//! Normalisierung der Referenzen (UUID statt Name) beim Speichern.

use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::PolicySnapshot;
use crate::ports::Tx;
use crate::ports::repos::label::LabelKind;
use crate::ports::repos::team::Team;
use crate::resolve;
use chrono::NaiveDate;
use lineal_domain::enums::{StateType, TemplateType};
use lineal_domain::ids::{LabelId, StateId, TeamId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

/// Maximale Tiefe verschachtelter Sub-Issues unter dem Wurzel-Issue.
pub const MAX_SUB_DEPTH: usize = 3;
/// Maximale Gesamtzahl der Sub-Issues eines Templates.
pub const MAX_SUB_TOTAL: usize = 50;
/// Maximale Zahl der Meilensteine, Projekt-Issues und -Dokumente.
pub const MAX_MILESTONES: usize = 50;
pub const MAX_PROJECT_ISSUES: usize = 100;
pub const MAX_PROJECT_DOCS: usize = 20;
/// Obergrenze für relative Fälligkeiten in Tagen (verhindert Überläufe).
pub const MAX_RELATIVE_DAYS: i64 = 36_500;
/// Maximale Gesamtgröße von `templateData` (serialisiert, Bytes).
pub const MAX_DATA_BYTES: usize = 256 * 1024;
/// Maximale Länge von Beschreibungen/Inhalten (Zeichen, wie bei Issues).
pub const MAX_TEXT_CHARS: usize = 500_000;
/// Maximale Zahl von Labels und Abonnenten je Issue-Eintrag.
pub const MAX_REFS: usize = 50;
/// Maximale Zahl von Platzhaltern je Text.
pub const MAX_PLACEHOLDERS: usize = 200;

/// Issue-Template (`type = issue`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct IssueData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_milestone_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_in: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscriber_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_issues: Option<Vec<SubIssueData>>,
    /// Form-Template (Erweiterung, docs/06 §2.8): Feldliste, nur gespeichert.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_fields: Option<Value>,
}

/// Ein Sub-Issue im Issue-Template.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SubIssueData {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_in: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_issues: Option<Vec<SubIssueData>>,
}

/// Dokument-Template (`type = document`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct DocData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Meilenstein im Projekt-Template.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MilestoneData {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_in: Option<String>,
}

/// Issue-Eintrag im Projekt-Template.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProjectIssueData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<IssueData>,
}

/// Dokument-Eintrag im Projekt-Template.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProjectDocData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<DocData>,
}

/// Projekt-Template (`type = project`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProjectData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiative_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_in: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_in: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_date_resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestones: Option<Vec<MilestoneData>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issues: Option<Vec<ProjectIssueData>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub documents: Option<Vec<ProjectDocData>>,
}

fn bad(field: &str, msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg).with_details(json!({ "field": field }))
}

/// Parst `templateData` strikt (unbekannte Felder → `invalid_request` mit `details.field`).
pub fn parse<T: for<'de> Deserialize<'de>>(data: &Value) -> AppResult<T> {
    if !data.is_object() {
        return Err(bad("templateData", "templateData must be an object"));
    }
    if data.to_string().len() > MAX_DATA_BYTES {
        return Err(AppError::limit_exceeded(format!(
            "templateData must not exceed {} KB",
            MAX_DATA_BYTES / 1024
        ))
        .with_details(json!({ "field": "templateData", "limit": MAX_DATA_BYTES })));
    }
    serde_json::from_value(data.clone())
        .map_err(|e| bad("templateData", format!("invalid templateData: {e}")))
}

/// `^P(\d+W|\d+D|\d+W\d+D)$` → Tage (mit Obergrenze).
pub fn parse_relative_days(field: &str, s: &str) -> AppResult<i64> {
    let err = || {
        bad(
            field,
            format!("`{field}` must be a duration like P5D, P2W or P1W3D"),
        )
    };
    let rest = s.strip_prefix('P').ok_or_else(err)?;
    if rest.is_empty() {
        return Err(err());
    }
    let mut days: i64 = 0;
    let mut num = String::new();
    let mut seen_w = false;
    let mut seen_d = false;
    for c in rest.chars() {
        match c {
            '0'..='9' => num.push(c),
            'W' | 'D' => {
                if num.is_empty() || num.len() > 6 {
                    return Err(err());
                }
                let n: i64 = num.parse().map_err(|_| err())?;
                num.clear();
                if c == 'W' {
                    if seen_w || seen_d {
                        return Err(err());
                    }
                    seen_w = true;
                    days = days
                        .checked_add(n.checked_mul(7).ok_or_else(err)?)
                        .ok_or_else(err)?;
                } else {
                    if seen_d {
                        return Err(err());
                    }
                    seen_d = true;
                    days = days.checked_add(n).ok_or_else(err)?;
                }
            }
            _ => return Err(err()),
        }
    }
    if !num.is_empty() || days > MAX_RELATIVE_DAYS {
        return Err(err());
    }
    Ok(days)
}

fn check_uuid(field: &str, s: &str) -> AppResult<()> {
    Uuid::parse_str(s)
        .map(|_| ())
        .map_err(|_| bad(field, format!("`{field}` must be a UUID")))
}

fn check_color(field: &str, c: &str) -> AppResult<()> {
    let ok = c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|x| x.is_ascii_hexdigit());
    if ok {
        Ok(())
    } else {
        Err(bad(field, "color must be #RRGGBB"))
    }
}

fn check_state_type(field: &str, s: &str) -> AppResult<()> {
    match s.parse::<StateType>() {
        Ok(StateType::Duplicate) | Err(_) => Err(bad(
            field,
            "stateType must be one of triage, backlog, unstarted, started, completed, canceled",
        )),
        Ok(_) => Ok(()),
    }
}

fn check_priority(field: &str, p: i64) -> AppResult<()> {
    if (0..=4).contains(&p) {
        Ok(())
    } else {
        Err(bad(field, "priority must be an integer between 0 and 4"))
    }
}

fn check_estimate(field: &str, e: f64) -> AppResult<()> {
    if e.is_finite() && e >= 0.0 {
        Ok(())
    } else {
        Err(bad(field, "estimate must be a non-negative number"))
    }
}

fn count_subs(subs: &[SubIssueData], depth: usize, total: &mut usize) -> AppResult<()> {
    if depth > MAX_SUB_DEPTH {
        return Err(AppError::limit_exceeded(format!(
            "sub-issues may be nested at most {MAX_SUB_DEPTH} levels deep"
        ))
        .with_details(json!({ "field": "subIssues", "limit": MAX_SUB_DEPTH })));
    }
    for s in subs {
        *total += 1;
        if *total > MAX_SUB_TOTAL {
            return Err(AppError::limit_exceeded(format!(
                "a template may contain at most {MAX_SUB_TOTAL} sub-issues"
            ))
            .with_details(json!({ "field": "subIssues", "limit": MAX_SUB_TOTAL })));
        }
        if let Some(children) = &s.sub_issues {
            count_subs(children, depth + 1, total)?;
        }
    }
    Ok(())
}

fn check_text(field: &str, t: &str) -> AppResult<()> {
    if t.chars().count() > MAX_TEXT_CHARS {
        return Err(AppError::limit_exceeded(format!(
            "`{field}` exceeds {MAX_TEXT_CHARS} characters"
        ))
        .with_details(json!({ "field": field })));
    }
    if t.matches("{{placeholder:").count() > MAX_PLACEHOLDERS {
        return Err(AppError::limit_exceeded("too many placeholders")
            .with_details(json!({ "field": field, "limit": MAX_PLACEHOLDERS })));
    }
    Ok(())
}

fn check_sub_limits(subs: &[SubIssueData], depth: usize, path: &str) -> AppResult<()> {
    if depth > MAX_SUB_DEPTH {
        return Err(AppError::limit_exceeded("sub-issues nested too deep")
            .with_details(json!({ "field": path, "limit": MAX_SUB_DEPTH })));
    }
    for (i, s) in subs.iter().enumerate() {
        let p = format!("{path}[{i}]");
        if s.title.chars().count() > 255 {
            return Err(AppError::limit_exceeded("title exceeds 255 characters")
                .with_details(json!({ "field": format!("{p}.title") })));
        }
        if let Some(d) = &s.description {
            check_text(&format!("{p}.description"), d)?;
        }
        if s.label_ids.as_ref().is_some_and(|l| l.len() > MAX_REFS) {
            return Err(AppError::limit_exceeded("too many labels")
                .with_details(json!({ "field": format!("{p}.labelIds"), "limit": MAX_REFS })));
        }
        if let Some(c) = &s.sub_issues {
            check_sub_limits(c, depth + 1, &format!("{p}.subIssues"))?;
        }
    }
    Ok(())
}

/// Grenzen eines Issue-Templates (Tiefe, Anzahl, Textlängen, Labels); gilt beim Speichern
/// und beim Anwenden.
pub fn check_issue_limits(d: &IssueData) -> AppResult<()> {
    if let Some(t) = &d.title
        && t.chars().count() > 255
    {
        return Err(AppError::limit_exceeded("title exceeds 255 characters")
            .with_details(json!({ "field": "title" })));
    }
    if let Some(t) = &d.description {
        check_text("description", t)?;
    }
    if d.label_ids.as_ref().is_some_and(|l| l.len() > MAX_REFS)
        || d.subscriber_ids
            .as_ref()
            .is_some_and(|l| l.len() > MAX_REFS)
    {
        return Err(AppError::limit_exceeded("too many labels or subscribers")
            .with_details(json!({ "limit": MAX_REFS })));
    }
    if let Some(subs) = &d.sub_issues {
        let mut total = 0;
        count_subs(subs, 1, &mut total)?;
        check_sub_limits(subs, 1, "subIssues")?;
    }
    Ok(())
}

/// Formale Prüfung eines Issue-Templates (ohne Datenbankzugriff).
pub fn check_issue_data(d: &IssueData, workspace: bool, path: &str) -> AppResult<()> {
    check_issue_limits(d)?;
    let f = |name: &str| format!("{path}{name}");
    if let Some(t) = &d.title
        && t.chars().count() > 255
    {
        return Err(bad(&f("title"), "title must not exceed 255 characters"));
    }
    if let Some(s) = &d.state_id {
        if workspace {
            return Err(bad(
                &f("stateId"),
                "workspace templates cannot contain a team-specific status; use stateType",
            ));
        }
        check_uuid(&f("stateId"), s)?;
    }
    if let Some(s) = &d.state_type {
        check_state_type(&f("stateType"), s)?;
    }
    if let Some(p) = d.priority {
        check_priority(&f("priority"), p)?;
    }
    if let Some(e) = d.estimate {
        check_estimate(&f("estimate"), e)?;
    }
    if let Some(c) = &d.cycle {
        if workspace {
            return Err(bad(
                &f("cycle"),
                "workspace templates cannot contain a cycle",
            ));
        }
        if c != "current" && c != "next" {
            return Err(bad(&f("cycle"), "cycle must be `current` or `next`"));
        }
    }
    if d.due_in.is_some() && d.due_date.is_some() {
        return Err(bad(&f("dueIn"), "dueIn and dueDate are mutually exclusive"));
    }
    if let Some(i) = &d.due_in {
        parse_relative_days(&f("dueIn"), i)?;
    }
    if let Some(dd) = &d.due_date {
        NaiveDate::parse_from_str(dd, "%Y-%m-%d")
            .map_err(|_| bad(&f("dueDate"), "dueDate must be YYYY-MM-DD"))?;
    }
    if let Some(p) = &d.project_id {
        check_uuid(&f("projectId"), p)?;
    }
    if let Some(p) = &d.project_milestone_id {
        check_uuid(&f("projectMilestoneId"), p)?;
    }
    if let Some(ff) = &d.form_fields
        && !ff.is_array()
    {
        return Err(bad(&f("formFields"), "formFields must be an array"));
    }
    if let Some(subs) = &d.sub_issues {
        let mut total = 0;
        count_subs(subs, 1, &mut total)?;
        check_subs(subs, workspace, &format!("{path}subIssues"))?;
    }
    Ok(())
}

fn check_subs(subs: &[SubIssueData], workspace: bool, path: &str) -> AppResult<()> {
    for (i, s) in subs.iter().enumerate() {
        let p = format!("{path}[{i}]");
        if s.title.trim().is_empty() || s.title.chars().count() > 255 {
            return Err(bad(&format!("{p}.title"), "title must be 1-255 characters"));
        }
        if let Some(x) = &s.state_id {
            if workspace {
                return Err(bad(
                    &format!("{p}.stateId"),
                    "workspace templates cannot contain a team-specific status; use stateType",
                ));
            }
            check_uuid(&format!("{p}.stateId"), x)?;
        }
        if let Some(x) = &s.state_type {
            check_state_type(&format!("{p}.stateType"), x)?;
        }
        if let Some(x) = s.priority {
            check_priority(&format!("{p}.priority"), x)?;
        }
        if let Some(x) = s.estimate {
            check_estimate(&format!("{p}.estimate"), x)?;
        }
        if let Some(x) = &s.due_in {
            parse_relative_days(&format!("{p}.dueIn"), x)?;
        }
        if let Some(x) = &s.template_id {
            check_uuid(&format!("{p}.templateId"), x)?;
        }
        if let Some(c) = &s.sub_issues {
            check_subs(c, workspace, &format!("{p}.subIssues"))?;
        }
    }
    Ok(())
}

/// Formale Prüfung eines Dokument-Templates.
pub fn check_doc_data(d: &DocData, path: &str) -> AppResult<()> {
    if let Some(t) = &d.title
        && t.chars().count() > 255
    {
        return Err(bad(
            &format!("{path}title"),
            "title must not exceed 255 characters",
        ));
    }
    if let Some(c) = &d.color {
        check_color(&format!("{path}color"), c)?;
    }
    if let Some(i) = &d.icon
        && (i.is_empty()
            || i.len() > 64
            || !i
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '-')))
    {
        return Err(bad(
            &format!("{path}icon"),
            "icon must match ^[A-Za-z0-9_:-]{1,64}$",
        ));
    }
    Ok(())
}

/// Formale Prüfung eines Projekt-Templates.
pub fn check_project_data(d: &ProjectData, workspace: bool) -> AppResult<()> {
    if let Some(n) = &d.name
        && n.chars().count() > 255
    {
        return Err(bad("name", "name must not exceed 255 characters"));
    }
    if let Some(s) = &d.summary
        && s.chars().count() > 255
    {
        return Err(bad("summary", "summary must not exceed 255 characters"));
    }
    if let Some(p) = d.priority {
        check_priority("priority", p)?;
    }
    if let Some(c) = &d.color {
        check_color("color", c)?;
    }
    if let Some(i) = &d.start_in {
        parse_relative_days("startIn", i)?;
    }
    if let Some(i) = &d.target_in {
        parse_relative_days("targetIn", i)?;
    }
    if let Some(r) = &d.target_date_resolution
        && r.parse::<lineal_domain::enums::DateResolution>().is_err()
    {
        return Err(bad(
            "targetDateResolution",
            "targetDateResolution must be month, quarter, halfYear or year",
        ));
    }
    if let Some(t) = &d.status_type
        && t.parse::<lineal_domain::enums::ProjectStatusType>()
            .is_err()
    {
        return Err(bad("statusType", "unknown project status category"));
    }
    if let Some(ms) = &d.milestones {
        if ms.len() > MAX_MILESTONES {
            return Err(bad("milestones", "at most 50 milestones"));
        }
        for (i, m) in ms.iter().enumerate() {
            if m.name.trim().is_empty() {
                return Err(bad(&format!("milestones[{i}].name"), "name is required"));
            }
            if let Some(t) = &m.target_in {
                parse_relative_days(&format!("milestones[{i}].targetIn"), t)?;
            }
        }
    }
    if let Some(is) = &d.issues {
        if is.len() > MAX_PROJECT_ISSUES {
            return Err(bad("issues", "at most 100 issues"));
        }
        for (i, e) in is.iter().enumerate() {
            let path = format!("issues[{i}].");
            if let Some(t) = &e.team_id {
                check_uuid(&format!("{path}teamId"), t)?;
            }
            if let Some(t) = &e.template_id {
                check_uuid(&format!("{path}templateId"), t)?;
            }
            if let Some(m) = &e.milestone
                && !d
                    .milestones
                    .as_ref()
                    .is_some_and(|ms| ms.iter().any(|x| x.name.eq_ignore_ascii_case(m)))
            {
                return Err(bad(
                    &format!("{path}milestone"),
                    "milestone must name an entry of `milestones`",
                ));
            }
            if let Some(issue) = &e.issue {
                check_issue_data(issue, workspace, &format!("{path}issue."))?;
            }
        }
    }
    if let Some(ds) = &d.documents {
        if ds.len() > MAX_PROJECT_DOCS {
            return Err(bad("documents", "at most 20 documents"));
        }
        for (i, e) in ds.iter().enumerate() {
            let path = format!("documents[{i}].");
            if let Some(t) = &e.template_id {
                check_uuid(&format!("{path}templateId"), t)?;
            }
            if let Some(doc) = &e.document {
                check_doc_data(doc, &format!("{path}document."))?;
            }
        }
    }
    Ok(())
}

/// Löst ein Issue-Label einer Vorlage auf (UUID oder Name) und prüft den Geltungsbereich.
/// Workspace-Templates dürfen nur Workspace-Labels enthalten (docs/06 §2.3).
pub async fn normalize_label(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    team: Option<&Team>,
    field: &str,
    reference: &str,
) -> AppResult<String> {
    let found = match team {
        Some(t) => resolve::label::resolve_for_team(tx, &snap.tree, t.id, reference).await,
        None => {
            let found: Vec<_> = resolve::label::candidates(tx, LabelKind::Issue, reference)
                .await?
                .into_iter()
                .filter(|l| l.archived_at.is_none())
                .collect();
            resolve::label::exactly_one(LabelKind::Issue, reference, found)
        }
    }
    .map_err(|e| {
        if e.kind == crate::error::ErrorKind::NotFound {
            bad(field, format!("unknown label `{reference}`"))
        } else {
            e
        }
    })?;
    if found.is_group {
        return Err(bad(field, "label groups cannot be used in templates"));
    }
    if team.is_none() && found.team_id.is_some() {
        return Err(bad(field, "workspace templates cannot contain team labels"));
    }
    Ok(found.id.to_string())
}

/// Nutzer-Referenz (`creator`, UUID, Name, E-Mail) → UUID-String (oder `creator`).
pub async fn normalize_user(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    field: &str,
    reference: &str,
    allow_creator: bool,
) -> AppResult<String> {
    if allow_creator && reference == "creator" {
        return Ok("creator".into());
    }
    let u = resolve::user::resolve_in(tx, snap, ctx, reference)
        .await
        .map_err(|e| {
            if e.kind == crate::error::ErrorKind::NotFound {
                bad(field, format!("unknown user `{reference}`"))
            } else {
                e
            }
        })?;
    Ok(u.id.to_string())
}

/// Status-Referenz eines Team-Templates → UUID-String; der Status muss zum Team oder einem
/// Vorfahren gehören.
pub async fn normalize_state(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    team: &Team,
    field: &str,
    reference: &str,
) -> AppResult<String> {
    let allowed: Vec<TeamId> = std::iter::once(team.id)
        .chain(snap.tree.ancestors(team.id).iter().map(|t| t.id))
        .collect();
    if let Ok(u) = Uuid::parse_str(reference) {
        return match tx.states().get(StateId(u)).await? {
            Some(s) if allowed.contains(&s.team_id) => Ok(s.id.to_string()),
            _ => Err(bad(field, "status does not belong to the template's team")),
        };
    }
    let s = resolve::state::resolve(tx, team, reference)
        .await
        .map_err(|_| bad(field, format!("unknown status `{reference}`")))?;
    Ok(s.id.to_string())
}

async fn normalize_labels(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    team: Option<&Team>,
    field: &str,
    labels: &mut Option<Vec<String>>,
) -> AppResult<()> {
    if let Some(list) = labels {
        let mut out: Vec<String> = Vec::new();
        for (i, r) in list.iter().enumerate() {
            let id = normalize_label(tx, snap, team, &format!("{field}[{i}]"), r).await?;
            if !out.contains(&id) {
                out.push(id);
            }
        }
        *list = out;
    }
    Ok(())
}

/// Normalisiert die Referenzen eines Issue-Templates (rekursiv über Sub-Issues).
pub async fn normalize_issue(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    team: Option<&Team>,
    d: &mut IssueData,
    path: &str,
) -> AppResult<()> {
    if let Some(s) = d.state_id.clone() {
        let Some(t) = team else {
            return Err(bad(
                &format!("{path}stateId"),
                "workspace templates cannot contain a status",
            ));
        };
        d.state_id = Some(normalize_state(tx, snap, t, &format!("{path}stateId"), &s).await?);
    }
    normalize_labels(tx, snap, team, &format!("{path}labelIds"), &mut d.label_ids).await?;
    if let Some(a) = d.assignee_id.clone() {
        d.assignee_id =
            Some(normalize_user(tx, snap, ctx, &format!("{path}assigneeId"), &a, true).await?);
    }
    if let Some(subs) = d.subscriber_ids.clone() {
        let mut out = Vec::new();
        for (i, s) in subs.iter().enumerate() {
            let id = normalize_user(
                tx,
                snap,
                ctx,
                &format!("{path}subscriberIds[{i}]"),
                s,
                false,
            )
            .await?;
            if !out.contains(&id) {
                out.push(id);
            }
        }
        d.subscriber_ids = Some(out);
    }
    if let Some(p) = d.project_id.clone() {
        let r = resolve::project::resolve_in(tx, snap, &p)
            .await
            .map_err(|e| {
                if e.kind == crate::error::ErrorKind::NotFound {
                    bad(&format!("{path}projectId"), "unknown project")
                } else {
                    e
                }
            })?;
        d.project_id = Some(r.project.id.to_string());
    }
    if let Some(subs) = d.sub_issues.as_mut() {
        normalize_subs(tx, snap, ctx, team, subs, &format!("{path}subIssues")).await?;
    }
    Ok(())
}

fn normalize_subs<'a>(
    tx: &'a mut dyn Tx,
    snap: &'a PolicySnapshot,
    ctx: &'a RequestCtx,
    team: Option<&'a Team>,
    subs: &'a mut [SubIssueData],
    path: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<()>> + Send + 'a>> {
    Box::pin(async move {
        for (i, s) in subs.iter_mut().enumerate() {
            let p = format!("{path}[{i}]");
            if let Some(x) = s.state_id.clone() {
                let Some(t) = team else {
                    return Err(bad(
                        &format!("{p}.stateId"),
                        "workspace templates cannot contain a status",
                    ));
                };
                s.state_id = Some(normalize_state(tx, snap, t, &format!("{p}.stateId"), &x).await?);
            }
            normalize_labels(tx, snap, team, &format!("{p}.labelIds"), &mut s.label_ids).await?;
            if let Some(a) = s.assignee_id.clone() {
                s.assignee_id = Some(
                    normalize_user(tx, snap, ctx, &format!("{p}.assigneeId"), &a, true).await?,
                );
            }
            if let Some(c) = s.sub_issues.as_mut() {
                normalize_subs(tx, snap, ctx, team, c, &format!("{p}.subIssues")).await?;
            }
        }
        Ok(())
    })
}

/// UUID-String → `LabelId` (für die Anwendung).
pub fn label_id(s: &str) -> Option<LabelId> {
    Uuid::parse_str(s).ok().map(LabelId)
}

/// Normalisiert und prüft `templateData` eines Typs vollständig; liefert das gespeicherte JSON.
pub async fn validate_and_normalize(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    ty: TemplateType,
    team: Option<&Team>,
    data: &Value,
) -> AppResult<Value> {
    let workspace = team.is_none();
    match ty {
        TemplateType::Issue => {
            let mut d: IssueData = parse(data)?;
            check_issue_data(&d, workspace, "")?;
            sanitize_issue_data(&mut d);
            normalize_issue(tx, snap, ctx, team, &mut d, "").await?;
            to_value(&d)
        }
        TemplateType::Document => {
            let mut d: DocData = parse(data)?;
            check_doc_data(&d, "")?;
            sanitize_doc_data(&mut d);
            to_value(&d)
        }
        TemplateType::Project => {
            let mut d: ProjectData = parse(data)?;
            check_project_data(&d, workspace)?;
            clean_opt(&mut d.name);
            clean_opt(&mut d.summary);
            clean_opt(&mut d.description);
            for m in d.milestones.iter_mut().flatten() {
                clean_plain(&mut m.name);
                clean_opt(&mut m.description);
            }
            for e in d.issues.iter_mut().flatten() {
                if let Some(i) = e.issue.as_mut() {
                    sanitize_issue_data(i);
                }
            }
            for e in d.documents.iter_mut().flatten() {
                if let Some(doc) = e.document.as_mut() {
                    sanitize_doc_data(doc);
                }
            }
            normalize_project(tx, snap, ctx, team, &mut d).await?;
            to_value(&d)
        }
    }
}

fn clean(s: &mut String) {
    *s = lineal_domain::rules::markdown::sanitize(s);
}

/// Einfacher Text (Titel): Platzhalter bleiben ohne Markdown-Maskierung.
fn clean_plain(s: &mut String) {
    *s = lineal_domain::rules::markdown::sanitize_plain(s);
}

fn clean_plain_opt(s: &mut Option<String>) {
    if let Some(x) = s {
        clean_plain(x);
    }
}

fn clean_opt(s: &mut Option<String>) {
    if let Some(x) = s {
        clean(x);
    }
}

fn sanitize_subs(subs: &mut [SubIssueData]) {
    for s in subs {
        clean_plain(&mut s.title);
        clean_opt(&mut s.description);
        if let Some(c) = s.sub_issues.as_mut() {
            sanitize_subs(c);
        }
    }
}

/// Bereinigt alle Freitexte eines Issue-Templates (Titel, Beschreibungen, Sub-Issues).
pub fn sanitize_issue_data(d: &mut IssueData) {
    clean_plain_opt(&mut d.title);
    clean_opt(&mut d.description);
    if let Some(s) = d.sub_issues.as_mut() {
        sanitize_subs(s);
    }
}

fn sanitize_doc_data(d: &mut DocData) {
    clean_plain_opt(&mut d.title);
    clean_opt(&mut d.content);
}

fn to_value<T: Serialize>(d: &T) -> AppResult<Value> {
    serde_json::to_value(d).map_err(|e| AppError::internal(format!("template serialization: {e}")))
}

async fn normalize_project(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &RequestCtx,
    team: Option<&Team>,
    d: &mut ProjectData,
) -> AppResult<()> {
    if let Some(l) = d.lead_id.clone() {
        d.lead_id = Some(normalize_user(tx, snap, ctx, "leadId", &l, true).await?);
    }
    if let Some(ms) = d.member_ids.clone() {
        let mut out = Vec::new();
        for (i, m) in ms.iter().enumerate() {
            let id = normalize_user(tx, snap, ctx, &format!("memberIds[{i}]"), m, false).await?;
            if !out.contains(&id) {
                out.push(id);
            }
        }
        d.member_ids = Some(out);
    }
    if let Some(ts) = d.team_ids.clone() {
        let mut out = Vec::new();
        for (i, t) in ts.iter().enumerate() {
            let r = resolve::team::resolve_in(tx, snap, t, resolve::team::Visibility::Content)
                .await
                .map_err(|_| bad(&format!("teamIds[{i}]"), format!("unknown team `{t}`")))?;
            let id = r.id.to_string();
            if !out.contains(&id) {
                out.push(id);
            }
        }
        d.team_ids = Some(out);
    }
    if let Some(ls) = d.label_ids.clone() {
        let mut out = Vec::new();
        for (i, r) in ls.iter().enumerate() {
            let rows = if let Ok(u) = Uuid::parse_str(r) {
                tx.projects().get_label(u).await?.into_iter().collect()
            } else {
                tx.projects().find_labels_by_name(r).await?
            };
            let Some(l) = rows.into_iter().next() else {
                return Err(bad(
                    &format!("labelIds[{i}]"),
                    format!("unknown project label `{r}`"),
                ));
            };
            if l.is_group {
                return Err(bad(
                    &format!("labelIds[{i}]"),
                    "label groups cannot be used in templates",
                ));
            }
            if team.is_none() && l.team_id.is_some() {
                return Err(bad(
                    &format!("labelIds[{i}]"),
                    "workspace templates cannot contain team labels",
                ));
            }
            let id = l.id.to_string();
            if !out.contains(&id) {
                out.push(id);
            }
        }
        d.label_ids = Some(out);
    }
    if let Some(is) = d.initiative_ids.clone() {
        let mut out = Vec::new();
        for (i, r) in is.iter().enumerate() {
            let init = resolve::initiative::resolve_in(tx, snap, r)
                .await
                .map_err(|_| {
                    bad(
                        &format!("initiativeIds[{i}]"),
                        format!("unknown initiative `{r}`"),
                    )
                })?;
            let id = init.id.to_string();
            if !out.contains(&id) {
                out.push(id);
            }
        }
        d.initiative_ids = Some(out);
    }
    if let Some(s) = d.status_id.clone() {
        let all = tx.projects().list_statuses().await?;
        let set = crate::usecases::project::pure::effective_status_set(
            &all,
            &snap.tree,
            team.map(|t| t.id),
        );
        let st = crate::usecases::project::pure::resolve_status_ref(&set, &s)
            .ok_or_else(|| bad("statusId", format!("unknown project status `{s}`")))?;
        d.status_id = Some(st.id.to_string());
    }
    if let Some(issues) = d.issues.as_mut() {
        for (i, e) in issues.iter_mut().enumerate() {
            let path = format!("issues[{i}].");
            let mut issue_team: Option<Team> = team.cloned();
            if let Some(t) = e.team_id.clone() {
                let r = resolve::team::resolve_in(tx, snap, &t, resolve::team::Visibility::Content)
                    .await
                    .map_err(|_| bad(&format!("{path}teamId"), format!("unknown team `{t}`")))?;
                e.team_id = Some(r.id.to_string());
                issue_team = Some(r);
            }
            if let Some(issue) = e.issue.as_mut() {
                normalize_issue(
                    tx,
                    snap,
                    ctx,
                    issue_team.as_ref(),
                    issue,
                    &format!("{path}issue."),
                )
                .await?;
            }
        }
    }
    Ok(())
}
