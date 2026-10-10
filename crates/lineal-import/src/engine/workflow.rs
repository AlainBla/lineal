//! Phase 2: Workflow-States, Labels (Gruppen vor Labels), Projektstatus, Projekt-Labels, Templates.

use super::node::{N, clean_text, clean_title, color_or, jts, one_of, row};
use super::{HResult, Outcome, Session, Step};
use crate::port::SourceEntity;
use lineal_app::error::{AppError, AppResult, ErrorKind};
use lineal_app::ports::repos::import::ImportTable;
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{LabelId, StateId, TeamId, TemplateId};
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use uuid::Uuid;

const STATE_TYPES: &[&str] = &[
    "triage",
    "backlog",
    "unstarted",
    "started",
    "completed",
    "canceled",
    "duplicate",
];
const PROJECT_STATUS_TYPES: &[&str] = &[
    "backlog",
    "planned",
    "started",
    "paused",
    "completed",
    "canceled",
];
const RESERVED_LABELS: &[&str] = &[
    "assignee", "cycle", "effort", "estimate", "hours", "priority", "project", "state", "status",
];

impl Session<'_, '_> {
    pub(crate) async fn step_states(&mut self, cursor: Option<String>) -> HResult<()> {
        let mut cursor = cursor;
        loop {
            let page = self.fetch(SourceEntity::WorkflowStates, cursor).await?;
            self.begin_unit().await?;
            for v in &page.items {
                let n = N(v);
                let r = self.state(n).await;
                self.item_result("WorkflowState", n.id(), r)?;
            }
            let next = page.next_cursor;
            self.end_unit(Some(self.cp(Step::States, next.clone())))
                .await?;
            match next {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        self.assign_team_states().await
    }

    async fn state(&mut self, n: N<'_>) -> AppResult<()> {
        let now = self.now;
        let Some(team_src) = n.ref_id("team") else {
            return Err(AppError::invalid_request("workflow state without team"));
        };
        if !self.selected.contains(team_src) {
            self.counters("WorkflowState").skipped += 1;
            return Ok(());
        }
        let Some(team) = self.target("Team", team_src).await? else {
            self.counters("WorkflowState").skipped += 1;
            return Ok(());
        };
        let name = clean_title(n.str("name"), 80, "State");
        let mut ty = one_of(n.str("type"), STATE_TYPES).unwrap_or("backlog");
        if n.str("type").is_some() && one_of(n.str("type"), STATE_TYPES).is_none() {
            self.note(
                "warning",
                "WorkflowState",
                n.id(),
                format!(
                    "unknown state type `{}`; imported as backlog",
                    clean_text(n.str("type").unwrap_or(""), 40, false)
                ),
            );
        }
        // Linear führt den Duplicate-Status als `canceled`; Lineal hat dafür einen eigenen Typ.
        if ty == "canceled" && name.eq_ignore_ascii_case("duplicate") {
            ty = "duplicate";
        }
        let created = n.ts("createdAt").unwrap_or(now).min(now);
        let make = |name: &str| {
            row(vec![
                ("team_id", json!(team)),
                ("name", json!(name)),
                ("type", json!(ty)),
                ("color", json!(color_or(n.str("color"), "#95A2B3"))),
                (
                    "description",
                    json!(n.text("description").map(|d| clean_text(d, 2000, true))),
                ),
                ("position", json!(n.float("position").unwrap_or(0.0))),
                ("archived_at", jts(n.ts("archivedAt"))),
                ("created_at", jts(Some(created))),
                (
                    "updated_at",
                    jts(n.ts("updatedAt").map(|t| t.min(now)).or(Some(created))),
                ),
            ])
        };
        let mut up = self
            .upsert(
                "WorkflowState",
                n,
                ImportTable::WorkflowState,
                make(&name),
                false,
            )
            .await;
        if let Err(e) = &up
            && e.kind == ErrorKind::Conflict
        {
            let alt = format!("{} (2)", clean_text(&name, 70, false));
            self.note(
                "warning",
                "WorkflowState",
                n.id(),
                format!("state name already exists in the team; imported as `{alt}`"),
            );
            up = self
                .upsert(
                    "WorkflowState",
                    n,
                    ImportTable::WorkflowState,
                    make(&alt),
                    false,
                )
                .await;
        }
        let up = up?;
        if up.outcome == Outcome::Created {
            self.event(DomainEvent::WorkflowStateCreated {
                state_id: StateId(up.id),
                team_id: TeamId(team),
            })
            .await?;
        }
        Ok(())
    }

    /// Zirkelreferenz Team <-> Status auflösen: Standard-, Triage- und Auto-Close-Status je Team.
    async fn assign_team_states(&mut self) -> HResult<()> {
        let teams = self.fetch_all_unfiltered(SourceEntity::Teams).await?;
        self.begin_unit().await?;
        for v in &teams {
            let n = N(v);
            let Some(src) = n.id().filter(|id| self.selected.contains(*id)) else {
                continue;
            };
            let Some(team) = self.target("Team", src).await? else {
                continue;
            };
            let mut default = self
                .target_opt("WorkflowState", n.ref_id("defaultIssueState"))
                .await?;
            if default.is_none() {
                default = self
                    .tx()?
                    .imports()
                    .first_state_of_types(team, &["backlog", "unstarted"])
                    .await?;
            }
            let mut triage = self
                .target_opt("WorkflowState", n.ref_id("triageIssueState"))
                .await?;
            if triage.is_none() && n.bool("triageEnabled").unwrap_or(false) {
                triage = self
                    .tx()?
                    .imports()
                    .first_state_of_types(team, &["triage"])
                    .await?;
            }
            let close = self
                .target_opt("WorkflowState", n.ref_id("autoCloseState"))
                .await?;
            self.tx()?
                .imports()
                .set_team_states(team, default, triage, close)
                .await?;
        }
        self.end_unit(None).await?;
        Ok(())
    }

    pub(crate) async fn step_labels(&mut self) -> HResult<()> {
        let nodes = self.fetch_all(SourceEntity::IssueLabels).await?;
        // Gruppen (und Wurzeln) vor ihren Kindern.
        let (roots, children): (Vec<&Value>, Vec<&Value>) =
            nodes.iter().partition(|v| N(v).ref_id("parent").is_none());
        self.begin_unit().await?;
        for v in roots.into_iter().chain(children) {
            let n = N(v);
            let r = self
                .label(n, "IssueLabel", ImportTable::IssueLabel, true)
                .await;
            self.item_result("IssueLabel", n.id(), r)?;
        }
        self.end_unit(None).await?;
        Ok(())
    }

    pub(crate) async fn step_project_labels(&mut self) -> HResult<()> {
        let nodes = self.fetch_all(SourceEntity::ProjectLabels).await?;
        let (roots, children): (Vec<&Value>, Vec<&Value>) =
            nodes.iter().partition(|v| N(v).ref_id("parent").is_none());
        self.begin_unit().await?;
        for v in roots.into_iter().chain(children) {
            let n = N(v);
            let r = self
                .label(n, "ProjectLabel", ImportTable::ProjectLabel, false)
                .await;
            self.item_result("ProjectLabel", n.id(), r)?;
        }
        self.end_unit(None).await?;
        Ok(())
    }

    async fn label(
        &mut self,
        n: N<'_>,
        ty: &'static str,
        table: ImportTable,
        issue_label: bool,
    ) -> AppResult<()> {
        let now = self.now;
        let team = match n.ref_id("team") {
            Some(t) if !self.selected.contains(t) => {
                self.counters(ty).skipped += 1;
                return Ok(());
            }
            Some(t) => match self.target("Team", t).await? {
                Some(id) => Some(id),
                None => {
                    self.counters(ty).skipped += 1;
                    return Ok(());
                }
            },
            None => None,
        };
        let parent = match n.ref_id("parent") {
            Some(p) => match self.target(ty, p).await? {
                Some(id) => Some(id),
                None => {
                    self.counters(ty).skipped += 1;
                    self.note(
                        "warning",
                        ty,
                        n.id(),
                        "the label group was not imported; label skipped",
                    );
                    return Ok(());
                }
            },
            None => None,
        };
        let is_group = n.bool("isGroup").unwrap_or(false);
        let mut name = clean_title(n.str("name"), 80, "Label");
        if issue_label && RESERVED_LABELS.contains(&name.to_lowercase().as_str()) {
            name = format!("{name} label");
            self.note(
                "info",
                ty,
                n.id(),
                "the label name is reserved in Lineal and was renamed",
            );
        }
        let created = n.ts("createdAt").unwrap_or(now).min(now);
        let creator = self.target_opt("User", n.ref_id("creator")).await?;
        let make = |name: &str| {
            let mut m: Map<String, Value> = row(vec![
                ("name", json!(name)),
                ("color", json!(color_or(n.str("color"), "#95A2B3"))),
                (
                    "description",
                    json!(n.text("description").map(|d| clean_text(d, 2000, true))),
                ),
                ("parent_id", json!(parent)),
                ("is_group", json!(is_group)),
                ("creator_id", json!(creator)),
                ("retired_at", jts(n.ts("retiredAt"))),
                ("created_at", jts(Some(created))),
                (
                    "updated_at",
                    jts(n.ts("updatedAt").map(|t| t.min(now)).or(Some(created))),
                ),
            ]);
            if issue_label {
                m.insert("team_id".into(), json!(team));
                m.insert(
                    "group_type".into(),
                    if is_group {
                        json!(
                            one_of(n.str("groupType"), &["singleSelect", "multiSelect"])
                                .unwrap_or("singleSelect")
                        )
                    } else {
                        Value::Null
                    },
                );
            } else {
                m.insert("team_id".into(), json!(team));
            }
            m
        };
        let mut up = self.upsert(ty, n, table, make(&name), false).await;
        if let Err(e) = &up
            && e.kind == ErrorKind::Conflict
        {
            let alt = format!("{} (Linear)", clean_text(&name, 68, false));
            self.note(
                "warning",
                ty,
                n.id(),
                format!("a label with this name already exists; imported as `{alt}`"),
            );
            up = self.upsert(ty, n, table, make(&alt), false).await;
        }
        let up = up?;
        if issue_label && up.outcome == Outcome::Created {
            self.event(DomainEvent::IssueLabelCreated {
                label_id: LabelId(up.id),
            })
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn step_project_statuses(&mut self, cursor: Option<String>) -> HResult<()> {
        let mut cursor = cursor;
        loop {
            let page = self.fetch(SourceEntity::ProjectStatuses, cursor).await?;
            self.begin_unit().await?;
            for v in &page.items {
                let n = N(v);
                let r = self.project_status(n).await;
                self.item_result("ProjectStatus", n.id(), r)?;
            }
            let next = page.next_cursor;
            self.end_unit(Some(self.cp(Step::ProjectStatuses, next.clone())))
                .await?;
            match next {
                Some(c) => cursor = Some(c),
                None => return Ok(()),
            }
        }
    }

    async fn project_status(&mut self, n: N<'_>) -> AppResult<()> {
        let now = self.now;
        let name = clean_title(n.str("name"), 80, "Status");
        let ty = one_of(n.str("type"), PROJECT_STATUS_TYPES).unwrap_or("backlog");
        let created = n.ts("createdAt").unwrap_or(now).min(now);
        let r = row(vec![
            ("name", json!(name)),
            ("type", json!(ty)),
            ("color", json!(color_or(n.str("color"), "#95A2B3"))),
            (
                "description",
                json!(n.text("description").map(|d| clean_text(d, 2000, true))),
            ),
            ("position", json!(n.float("position").unwrap_or(0.0))),
            ("indefinite", json!(n.bool("indefinite").unwrap_or(true))),
            ("archived_at", jts(n.ts("archivedAt"))),
            ("created_at", jts(Some(created))),
            (
                "updated_at",
                jts(n.ts("updatedAt").map(|t| t.min(now)).or(Some(created))),
            ),
        ]);
        match self
            .upsert("ProjectStatus", n, ImportTable::ProjectStatus, r, false)
            .await
        {
            Ok(_) => Ok(()),
            Err(e) if e.kind == ErrorKind::Conflict => {
                // Projektstatus sind Workspace-Konfiguration: ein gleichnamiger vorhandener Status
                // wird zugeordnet statt doppelt angelegt (wie der Duplicate-State).
                let existing = self
                    .tx()?
                    .imports()
                    .find_row_id_by_name(ImportTable::ProjectStatus, &name, None)
                    .await?;
                match (existing, n.id()) {
                    (Some(id), Some(src)) => {
                        let checksum = super::node::checksum_of(n.0);
                        self.put_mapping("ProjectStatus", src, id, n, &checksum)
                            .await?;
                        self.counters("ProjectStatus").unchanged += 1;
                        self.note(
                            "info",
                            "ProjectStatus",
                            Some(src),
                            "mapped to the existing project status of the same name",
                        );
                        Ok(())
                    }
                    _ => Err(e),
                }
            }
            Err(e) => Err(e),
        }
    }

    pub(crate) async fn step_templates(&mut self) -> HResult<()> {
        let nodes = self.fetch_all(SourceEntity::Templates).await?;
        self.begin_unit().await?;
        for v in &nodes {
            let n = N(v);
            let r = self.template(n).await;
            self.item_result("Template", n.id(), r)?;
        }
        self.end_unit(None).await?;
        Ok(())
    }

    async fn template(&mut self, n: N<'_>) -> AppResult<()> {
        let now = self.now;
        let ty = match n.str("type") {
            Some(t @ ("issue" | "project" | "document")) => t,
            other => {
                self.counters("Template").skipped += 1;
                self.note(
                    "info",
                    "Template",
                    n.id(),
                    format!(
                        "template type `{}` is not supported; skipped",
                        clean_text(other.unwrap_or(""), 40, false)
                    ),
                );
                return Ok(());
            }
        };
        let team = match n.ref_id("team") {
            Some(t) if !self.selected.contains(t) => {
                self.counters("Template").skipped += 1;
                return Ok(());
            }
            Some(t) => self.target("Team", t).await?,
            None => None,
        };
        let (data, dropped) = self.template_data(n, ty).await?;
        if !dropped.is_empty() {
            self.note(
                "info",
                "Template",
                n.id(),
                format!(
                    "template fields without a Lineal equivalent were discarded: {}",
                    dropped.join(", ")
                ),
            );
        }
        let creator = self.target_opt("User", n.ref_id("creator")).await?;
        let created = n.ts("createdAt").unwrap_or(now).min(now);
        let r = row(vec![
            ("type", json!(ty)),
            ("name", json!(clean_title(n.str("name"), 200, "Template"))),
            (
                "description",
                json!(n.text("description").map(|d| clean_text(d, 2000, true))),
            ),
            ("team_id", json!(team)),
            ("template_data", data),
            ("creator_id", json!(creator)),
            ("created_at", jts(Some(created))),
            (
                "updated_at",
                jts(n.ts("updatedAt").map(|t| t.min(now)).or(Some(created))),
            ),
        ]);
        let up = self
            .upsert("Template", n, ImportTable::Template, r, false)
            .await?;
        if up.outcome == Outcome::Created {
            self.event(DomainEvent::TemplateCreated {
                template_id: TemplateId(up.id),
            })
            .await?;
        }
        Ok(())
    }

    /// Übersetzt `templateData` in das Lineal-Format; liefert die verworfenen Schlüssel.
    async fn template_data(&mut self, n: N<'_>, ty: &str) -> AppResult<(Value, Vec<String>)> {
        let src = n.0.get("templateData").cloned().unwrap_or(Value::Null);
        // Linear liefert `templateData` teils als JSON-Text.
        let src = match src {
            Value::String(s) => serde_json::from_str::<Value>(&s).unwrap_or(Value::Null),
            other => other,
        };
        let Some(obj) = src.as_object() else {
            return Ok((json!({}), vec![]));
        };
        let mut out = Map::new();
        let mut dropped = Vec::new();
        for (k, v) in obj {
            let key = k.as_str();
            let ok = match (ty, key) {
                ("issue" | "document", "title") => v.as_str().map(|s| {
                    out.insert("title".into(), json!(clean_title(Some(s), 255, "")));
                }),
                ("issue", "description") | ("document", "content") => {
                    let id = n.id().unwrap_or("").to_owned();
                    v.as_str().map(|s| {
                        let t = self
                            .md("Template", &id, Some(s), super::MAX_DESCRIPTION)
                            .unwrap_or_default();
                        out.insert(
                            if ty == "issue" {
                                "description"
                            } else {
                                "content"
                            }
                            .into(),
                            json!(t),
                        );
                    })
                }
                ("issue", "priority") => v.as_i64().filter(|p| (0..=4).contains(p)).map(|p| {
                    out.insert("priority".into(), json!(p));
                }),
                ("issue", "estimate") => {
                    v.as_f64().filter(|e| *e >= 0.0 && e.is_finite()).map(|e| {
                        out.insert("estimate".into(), json!(e));
                    })
                }
                ("issue", "labelIds") => {
                    let mut ids = Vec::new();
                    for l in v
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .take(50)
                    {
                        if let Some(t) = self.target("IssueLabel", l).await? {
                            ids.push(t.to_string());
                        }
                    }
                    out.insert("labelIds".into(), json!(ids));
                    Some(())
                }
                ("issue", "assigneeId") => match v.as_str() {
                    Some(u) => self.target("User", u).await?.map(|t| {
                        out.insert("assigneeId".into(), json!(t.to_string()));
                    }),
                    None => None,
                },
                ("issue", "stateId") => match v.as_str() {
                    Some(s) => self.target("WorkflowState", s).await?.map(|t| {
                        out.insert("stateId".into(), json!(t.to_string()));
                    }),
                    None => None,
                },
                _ => None,
            };
            if ok.is_none() {
                dropped.push(clean_text(k, 40, false));
            }
        }
        // Gleiche Namen wie in docs/06 §2.4; `Value::Null`-Werte erzeugen keine Schlüssel.
        let unique: HashSet<String> = dropped.drain(..).collect();
        let mut dropped: Vec<String> = unique.into_iter().collect();
        dropped.sort();
        Ok((Value::Object(out), dropped))
    }

    /// Eindeutige Ziel-ID eines Labels (für `issue_to_label`).
    pub(crate) async fn label_target(&mut self, src: &str) -> AppResult<Option<Uuid>> {
        self.target("IssueLabel", src).await
    }
}
