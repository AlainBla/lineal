//! Phase 7: Custom Views und Favoriten (docs/13 §4.1, WP34).
//!
//! Linears `filterData` hat dieselbe Form wie das Lineal-Filtermodell (docs/05), enthält aber
//! Quell-IDs und Felder, die es in Lineal nicht gibt. Die Übersetzung
//!
//! 1. ersetzt Quell-IDs (`id`-Comparator unter Benutzer-, Team-, Status-, Label-, Projekt-,
//!    Cycle- … Feldern) durch Lineal-IDs; nicht importierte Objekte entfallen mit Warnung,
//! 2. entfernt Stelle für Stelle, was der Lineal-Filterparser ablehnt (unbekannte Felder,
//!    ungültige Operatoren/Werte), und meldet jede entfernte Stelle als Warnung,
//! 3. normalisiert das Ergebnis wie `parse_view_filter` beim normalen Anlegen.

use super::node::{N, clean_text, clean_title, color_or, icon_or_none, jts, row};
use super::{HResult, Session, Step};
use crate::port::SourceEntity;
use lineal_app::error::{AppError, AppResult};
use lineal_app::ports::repos::import::ImportTable;
use lineal_app::ports::repos::view::ViewModel;
use lineal_app::usecases::view::parse_view_filter;
use lineal_domain::rules::filter::{
    self, InitiativeFilter, IssueFilter, ProjectFilter, validate, validate_initiative,
    validate_project,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

/// Obergrenze der Korrekturschritte je Filter (jeder Schritt entfernt eine Stelle).
const MAX_REPAIRS: usize = 200;

/// Quelltyp der Objekte, auf die ein Relationsfeld verweist (für `id`-Comparatoren).
fn ref_type(model: ViewModel, key: &str) -> Option<&'static str> {
    Some(match key {
        "assignee" | "creator" | "delegate" | "subscribers" | "snoozedBy" | "lead" | "owner"
        | "members" => "User",
        "team" | "teams" => "Team",
        "state" => "WorkflowState",
        "labels" if model == ViewModel::Project => "ProjectLabel",
        "labels" => "IssueLabel",
        "project" | "projects" => "Project",
        "cycle" => "Cycle",
        "parent" if model == ViewModel::Initiative => "Initiative",
        "parent" => "Issue",
        "initiative" | "initiatives" => "Initiative",
        "projectMilestone" => "ProjectMilestone",
        "status" => "ProjectStatus",
        _ => return None,
    })
}

fn root_type(model: ViewModel) -> &'static str {
    match model {
        ViewModel::Issue => "Issue",
        ViewModel::Project => "Project",
        ViewModel::Initiative => "Initiative",
    }
}

/// Durchläuft den Filter und ruft für jede ID eines `id`-Comparators `f(Typ, Quell-ID)` auf.
/// `Some(neu)` ersetzt die ID, `None` entfernt sie. Gibt Warnungen für entfernte IDs zurück.
fn walk(
    model: ViewModel,
    v: &mut Value,
    ctx: Option<&'static str>,
    f: &mut dyn FnMut(&'static str, &str) -> Option<String>,
    warnings: &mut Vec<String>,
) {
    let Value::Object(map) = v else {
        if let Value::Array(items) = v {
            for i in items {
                walk(model, i, ctx, f, warnings);
            }
        }
        return;
    };
    for (k, child) in map.iter_mut() {
        if k == "id" {
            let Some(ty) = ctx else {
                continue;
            };
            let Value::Object(cmp) = child else {
                continue;
            };
            for (op, val) in cmp.iter_mut() {
                if matches!(op.as_str(), "eq" | "neq") {
                    let mapped = val.as_str().map(|s| f(ty, s));
                    match mapped {
                        Some(Some(n)) => *val = Value::String(n),
                        Some(None) => *val = Value::Null,
                        None => {}
                    }
                    continue;
                }
                if let ("in" | "nin", Value::Array(a)) = (op.as_str(), val) {
                    let before = a.len();
                    a.retain_mut(|x| match x {
                        Value::String(s) => match f(ty, s) {
                            Some(n) => {
                                *s = n;
                                true
                            }
                            None => false,
                        },
                        _ => true,
                    });
                    if a.len() != before {
                        warnings.push(format!(
                            "{} reference(s) of type {ty} in `{op}` were not imported and were left out",
                            before - a.len()
                        ));
                    }
                }
            }
            // `eq`/`neq` ohne Ziel (null) entfernen; leere Listen ebenfalls.
            cmp.retain(|op, val| match (op.as_str(), &*val) {
                ("eq" | "neq", Value::Null) => {
                    warnings.push(format!(
                        "a reference of type {ty} in `{op}` was not imported; the condition was dropped"
                    ));
                    false
                }
                ("in" | "nin", Value::Array(a)) => !a.is_empty(),
                _ => true,
            });
            continue;
        }
        let next = match k.as_str() {
            "some" | "every" | "none" | "and" | "or" | "not" => ctx,
            other => ref_type(model, other),
        };
        walk(model, child, next, f, warnings);
    }
}

/// Alle (Quelltyp, Quell-ID) des Filters (für das Auflösen der Mappings vor der Übersetzung).
pub(crate) fn collect_refs(model: ViewModel, filter: &Value) -> Vec<(&'static str, String)> {
    let mut v = filter.clone();
    let mut out = Vec::new();
    walk(
        model,
        &mut v,
        Some(root_type(model)),
        &mut |t, id| {
            out.push((t, id.to_owned()));
            Some(id.to_owned())
        },
        &mut Vec::new(),
    );
    out.sort();
    out.dedup();
    out
}

/// Ergebnis der Filterübersetzung.
#[derive(Debug)]
pub(crate) struct Translated {
    /// Normalisierter Lineal-Filter; `None`, wenn nichts Verwertbares übrig blieb.
    pub filter: Option<Value>,
    pub warnings: Vec<String>,
}

/// Entfernt die Stelle `pointer` (RFC 6901) aus `v`; `false`, wenn es sie nicht gibt.
fn remove_at(v: &mut Value, pointer: &str) -> bool {
    let segs: Vec<String> = pointer
        .split('/')
        .skip(1)
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect();
    let Some((last, parents)) = segs.split_last() else {
        return false;
    };
    let mut cur = v;
    for s in parents {
        let next = match cur {
            Value::Object(m) => m.get_mut(s),
            Value::Array(a) => s.parse::<usize>().ok().and_then(|i| a.get_mut(i)),
            _ => None,
        };
        match next {
            Some(n) => cur = n,
            None => return false,
        }
    }
    match cur {
        Value::Object(m) => m.remove(last).is_some(),
        Value::Array(a) => match last.parse::<usize>() {
            Ok(i) if i < a.len() => {
                a.remove(i);
                true
            }
            _ => false,
        },
        _ => false,
    }
}

fn check(model: ViewModel, v: &Value) -> Result<(), filter::FilterError> {
    match model {
        ViewModel::Issue => {
            let f: IssueFilter = filter::from_json_value(v)?;
            validate(&f)
        }
        ViewModel::Project => {
            let f: ProjectFilter = filter::from_json_value(v)?;
            validate_project(&f)
        }
        ViewModel::Initiative => {
            let f: InitiativeFilter = filter::from_json_value(v)?;
            validate_initiative(&f)
        }
    }
}

/// Übersetzt einen Quell-Filter in das Lineal-Modell (siehe Moduldoku). `map` enthält die
/// aufgelösten IDs `(Quelltyp, Quell-ID) -> Lineal-ID`.
pub(crate) fn translate_filter(
    model: ViewModel,
    source: &Value,
    map: &HashMap<(&'static str, String), Uuid>,
) -> Translated {
    let mut warnings = Vec::new();
    let mut v = source.clone();
    if !v.is_object() {
        return Translated {
            filter: None,
            warnings: vec!["the filter is not an object; the view filter was dropped".into()],
        };
    }
    walk(
        model,
        &mut v,
        Some(root_type(model)),
        &mut |t, id| map.get(&(t, id.to_owned())).map(Uuid::to_string),
        &mut warnings,
    );
    let mut dropped_all = false;
    for _ in 0..MAX_REPAIRS {
        match check(model, &v) {
            Ok(()) => {
                // Der Filter ist gültig; normalisieren wie beim normalen Anlegen.
                return match parse_view_filter(model, &v) {
                    Ok((_, norm)) => {
                        if dropped_all {
                            warnings.push(
                                "no usable condition remained; the view shows all items".into(),
                            );
                        }
                        Translated {
                            filter: Some(norm),
                            warnings,
                        }
                    }
                    Err(e) => {
                        warnings.push(format!("the filter was rejected: {}", e.message));
                        Translated {
                            filter: None,
                            warnings,
                        }
                    }
                };
            }
            Err(e) => {
                let field = e.path.rsplit('/').next().unwrap_or("").to_owned();
                if e.path.is_empty() || !remove_at(&mut v, &e.path) {
                    warnings.push(format!("the filter could not be translated: {}", e.reason));
                    return Translated {
                        filter: None,
                        warnings,
                    };
                }
                warnings.push(format!(
                    "`{}` (at {}) is not supported by Lineal and was left out: {}",
                    field, e.path, e.reason
                ));
                if v.as_object().is_some_and(serde_json::Map::is_empty) {
                    dropped_all = true;
                }
            }
        }
    }
    warnings.push("too many unsupported conditions; the filter was dropped".into());
    Translated {
        filter: None,
        warnings,
    }
}

const FAVORITE_TARGETS: &[(&str, &str, &str, &str)] = &[
    // (Favorit-Typ, Quellfeld, Quelltyp, Spalte)
    ("issue", "issue", "Issue", "issue_id"),
    ("project", "project", "Project", "project_id"),
    ("cycle", "cycle", "Cycle", "cycle_id"),
    ("customView", "customView", "CustomView", "custom_view_id"),
    ("document", "document", "Document", "document_id"),
    ("initiative", "initiative", "Initiative", "initiative_id"),
    ("label", "label", "IssueLabel", "label_id"),
    (
        "projectLabel",
        "projectLabel",
        "ProjectLabel",
        "project_label_id",
    ),
    ("team", "team", "Team", "team_id"),
    ("user", "user", "User", "user_id"),
];

impl Session<'_, '_> {
    pub(crate) async fn step_views(&mut self, cursor: Option<String>) -> HResult<()> {
        // Zuerst die Views (Favoriten verweisen auf sie), danach die Favoriten.
        let (stage, mut cursor): (u8, Option<String>) =
            match cursor.as_deref().and_then(|c| c.split_once(':')) {
                Some(("1", c)) => (1, Some(c.to_owned()).filter(|c| !c.is_empty())),
                Some((_, c)) => (0, Some(c.to_owned()).filter(|c| !c.is_empty())),
                None => (0, None),
            };
        if stage == 0 {
            loop {
                let page = self.fetch(SourceEntity::CustomViews, cursor).await?;
                self.begin_unit().await?;
                for v in &page.items {
                    let n = N(v);
                    let r = self.custom_view(n).await;
                    self.item_result("CustomView", n.id(), r)?;
                }
                let next = page.next_cursor;
                let cp = next
                    .clone()
                    .map(|c| format!("0:{c}"))
                    .unwrap_or("1:".into());
                self.end_unit(Some(self.cp(Step::Views, Some(cp)))).await?;
                match next {
                    Some(c) => cursor = Some(c),
                    None => break,
                }
            }
            cursor = None;
        }
        // Favoriten: Ordner zuerst (Kinder verweisen auf sie), dann der Rest in Quellreihenfolge.
        // Je Nutzer gibt es nur wenige Favoriten; die Seiten werden vollständig gelesen.
        let mut items = Vec::new();
        loop {
            let page = self.fetch(SourceEntity::Favorites, cursor).await?;
            items.extend(page.items);
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        items.sort_by_key(|v| N(v).str("type") != Some("folder"));
        self.begin_unit().await?;
        for v in &items {
            let n = N(v);
            let r = self.favorite(n).await;
            self.item_result("Favorite", n.id(), r)?;
        }
        self.end_unit(Some(self.cp(Step::Views, Some("1:".into()))))
            .await?;
        Ok(())
    }

    async fn custom_view(&mut self, n: N<'_>) -> AppResult<()> {
        let src = n
            .id()
            .ok_or_else(|| AppError::invalid_request("custom view without id"))?
            .to_owned();
        let Some(model) = n.str("modelName").and_then(ViewModel::parse) else {
            self.counters("CustomView").skipped += 1;
            self.note(
                "info",
                "CustomView",
                Some(&src),
                "views of this kind are not supported; skipped",
            );
            return Ok(());
        };
        let data_key = match model {
            ViewModel::Issue => "filterData",
            ViewModel::Project => "projectFilterData",
            ViewModel::Initiative => "initiativeFilterData",
        };
        let team = match n.ref_id("team") {
            Some(t) => match self.target("Team", t).await? {
                Some(id) => Some(id),
                None => {
                    self.counters("CustomView").skipped += 1;
                    self.note(
                        "info",
                        "CustomView",
                        Some(&src),
                        "the team of the view was not imported; skipped",
                    );
                    return Ok(());
                }
            },
            None => None,
        };
        if team.is_none() && self.opts.scoped() {
            self.counters("CustomView").skipped += 1;
            self.note(
                "info",
                "CustomView",
                Some(&src),
                "workspace views are not imported by a partial import",
            );
            return Ok(());
        }
        let shared = n.bool("shared").unwrap_or(false);
        let owner = match self.target_opt("User", n.ref_id("owner")).await? {
            Some(o) => o,
            None => match (shared, self.run.started_by) {
                (true, Some(s)) => s,
                _ => {
                    self.counters("CustomView").skipped += 1;
                    self.note(
                        "warning",
                        "CustomView",
                        Some(&src),
                        "the owner of the view was not imported; skipped",
                    );
                    return Ok(());
                }
            },
        };
        let creator = self
            .target_opt("User", n.ref_id("creator"))
            .await?
            .unwrap_or(owner);
        // Filter übersetzen.
        let source_filter = n.0.get(data_key).cloned().unwrap_or(json!({}));
        let source_filter = if source_filter.is_null() {
            json!({})
        } else {
            source_filter
        };
        let mut map: HashMap<(&'static str, String), Uuid> = HashMap::new();
        for (ty, id) in collect_refs(model, &source_filter) {
            if let Some(t) = self.target(ty, &id).await? {
                map.insert((ty, id), t);
            }
        }
        let tr = translate_filter(model, &source_filter, &map);
        for w in &tr.warnings {
            self.note("warning", "CustomView", Some(&src), w.clone());
        }
        let Some(filter_json) = tr.filter else {
            self.counters("CustomView").skipped += 1;
            self.note(
                "warning",
                "CustomView",
                Some(&src),
                "the view filter could not be translated; the view was skipped",
            );
            return Ok(());
        };
        let now = self.now;
        let created = n.ts("createdAt").unwrap_or(now).min(now);
        let mut cols = vec![
            (
                "name",
                json!(clean_title(n.str("name"), 80, "Imported view")),
            ),
            (
                "description",
                json!(n.text("description").map(|d| clean_text(d, 2000, true))),
            ),
            ("icon", json!(icon_or_none(n.text("icon")))),
            (
                "color",
                n.text("color")
                    .map(|c| json!(color_or(Some(c), "#6B6F76")))
                    .unwrap_or(Value::Null),
            ),
            ("model_name", json!(model.as_str())),
            ("owner_id", json!(owner)),
            ("creator_id", json!(creator)),
            ("shared", json!(shared)),
            ("team_id", json!(team)),
            ("archived_at", jts(n.ts("archivedAt"))),
            ("created_at", jts(Some(created))),
            (
                "updated_at",
                jts(n.ts("updatedAt").map(|t| t.min(now)).or(Some(created))),
            ),
        ];
        let col = match model {
            ViewModel::Issue => "filter_data",
            ViewModel::Project => "project_filter_data",
            ViewModel::Initiative => "initiative_filter_data",
        };
        cols.push((col, filter_json));
        self.upsert("CustomView", n, ImportTable::CustomView, row(cols), false)
            .await?;
        Ok(())
    }

    async fn favorite(&mut self, n: N<'_>) -> AppResult<()> {
        let src = n
            .id()
            .ok_or_else(|| AppError::invalid_request("favorite without id"))?
            .to_owned();
        let Some(owner) = self.target_opt("User", n.ref_id("owner")).await? else {
            self.counters("Favorite").skipped += 1;
            return Ok(());
        };
        let ty = n.str("type").unwrap_or("");
        let now = self.now;
        let created = n.ts("createdAt").unwrap_or(now).min(now);
        let mut cols = vec![
            ("owner_id", json!(owner)),
            ("sort_order", json!(n.float("sortOrder").unwrap_or(0.0))),
            ("archived_at", jts(n.ts("archivedAt"))),
            ("created_at", jts(Some(created))),
            (
                "updated_at",
                jts(n.ts("updatedAt").map(|t| t.min(now)).or(Some(created))),
            ),
        ];
        // Ordner-Zugehörigkeit (eine Ebene).
        if let Some(p) = n.ref_id("parent") {
            match self.target("Favorite", p).await? {
                Some(pid) => cols.push(("parent_id", json!(pid))),
                None => self.note(
                    "warning",
                    "Favorite",
                    Some(&src),
                    "the favorite folder was not imported; imported without a folder",
                ),
            }
        }
        match ty {
            "folder" => {
                let name = clean_title(n.str("folderName"), 80, "Folder");
                cols.push(("type", json!("folder")));
                cols.push(("folder_name", json!(name)));
            }
            "predefinedView" => {
                let Some(kind) = n
                    .text("predefinedViewType")
                    .map(|k| clean_text(k, 64, false))
                else {
                    self.counters("Favorite").skipped += 1;
                    return Ok(());
                };
                let team = self
                    .target_opt("Team", n.ref_id("predefinedViewTeam"))
                    .await?;
                cols.push(("type", json!("predefinedView")));
                cols.push(("predefined_view_type", json!(kind)));
                cols.push(("predefined_view_team_id", json!(team)));
            }
            other => {
                let Some((fav_ty, field, src_ty, column)) =
                    FAVORITE_TARGETS.iter().find(|(t, ..)| *t == other).copied()
                else {
                    self.counters("Favorite").skipped += 1;
                    self.note(
                        "info",
                        "Favorite",
                        Some(&src),
                        format!(
                            "favorites of type `{}` are not supported; skipped",
                            clean_text(other, 40, false)
                        ),
                    );
                    return Ok(());
                };
                let Some(target) = self.target_opt(src_ty, n.ref_id(field)).await? else {
                    self.counters("Favorite").skipped += 1;
                    self.note(
                        "info",
                        "Favorite",
                        Some(&src),
                        "the favorite target was not imported; skipped",
                    );
                    return Ok(());
                };
                cols.push(("type", json!(fav_ty)));
                cols.push((column, json!(target)));
            }
        }
        self.upsert("Favorite", n, ImportTable::Favorite, row(cols), false)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&'static str, &str, &str)]) -> HashMap<(&'static str, String), Uuid> {
        pairs
            .iter()
            .map(|(t, s, u)| ((*t, (*s).to_owned()), u.parse().unwrap()))
            .collect()
    }

    const A: &str = "11111111-1111-4111-8111-111111111111";
    const T: &str = "22222222-2222-4222-8222-222222222222";

    #[test]
    fn ids_are_rewritten_and_unknown_fields_are_dropped_with_warnings() {
        let src = json!({
            "assignee": { "id": { "eq": "u-alice" } },
            "team": { "id": { "in": ["t-eng", "t-gone"] } },
            "customerCount": { "gt": 1 },
            "priority": { "lte": 2 }
        });
        let m = map(&[("User", "u-alice", A), ("Team", "t-eng", T)]);
        let tr = translate_filter(ViewModel::Issue, &src, &m);
        let f = tr.filter.expect("translated");
        assert_eq!(
            f["assignee"]["id"]["in"],
            json!([A]),
            "eq is normalized to in"
        );
        assert_eq!(f["team"]["id"]["in"], json!([T]));
        assert!(f.get("customerCount").is_none());
        assert_eq!(f["priority"]["lte"], 2);
        let w = tr.warnings.join("\n");
        assert!(w.contains("customerCount"), "{w}");
        assert!(w.contains("not imported"), "{w}");
    }

    #[test]
    fn unmapped_eq_reference_is_dropped() {
        let src = json!({ "assignee": { "id": { "eq": "u-nobody" } }, "priority": { "eq": 1 } });
        let tr = translate_filter(ViewModel::Issue, &src, &HashMap::new());
        let f = tr.filter.unwrap();
        assert_eq!(f["priority"]["in"], json!([1]));
        assert!(f["assignee"].get("id").is_none() || f["assignee"]["id"] == json!({}));
        assert!(!tr.warnings.is_empty());
    }

    #[test]
    fn refs_are_collected_by_type() {
        let src = json!({ "and": [
            { "labels": { "some": { "id": { "in": ["l-1", "l-2"] } } } },
            { "cycle": { "id": { "eq": "c-1" } } }
        ]});
        let refs = collect_refs(ViewModel::Issue, &src);
        assert!(refs.contains(&("IssueLabel", "l-1".into())));
        assert!(refs.contains(&("IssueLabel", "l-2".into())));
        assert!(refs.contains(&("Cycle", "c-1".into())));
    }

    #[test]
    fn non_object_filter_is_rejected() {
        let tr = translate_filter(ViewModel::Issue, &json!("x"), &HashMap::new());
        assert!(tr.filter.is_none());
    }
}
