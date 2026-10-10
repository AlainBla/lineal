//! Phase 3: Cycles, Initiativen (DAG), Projekte (inkl. Teams, Mitglieder, Labels), Meilensteine.

use super::node::{N, clean_text, clean_title, color_or, icon_or_none, jts, one_of, row};
use super::{HResult, MAX_DESCRIPTION, MAX_DOCUMENT, Outcome, Session, Step};
use crate::port::SourceEntity;
use lineal_app::error::{AppError, AppResult, ErrorKind};
use lineal_app::ports::repos::import::{ImportCounter, ImportTable};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{CycleId, InitiativeId, MilestoneId, ProjectId, TeamId};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

const HEALTH: &[&str] = &["onTrack", "atRisk", "offTrack"];
const INITIATIVE_STATUS: &[&str] = &["Proposed", "Planned", "Active", "Completed", "Canceled"];

fn valid_slug(s: Option<&str>) -> Option<String> {
    let s = s?.to_ascii_lowercase();
    (s.len() == 12 && s.chars().all(|c| c.is_ascii_hexdigit())).then_some(s)
}

impl Session<'_, '_> {
    pub(crate) async fn step_cycles(&mut self, cursor: Option<String>) -> HResult<()> {
        let mut cursor = cursor;
        loop {
            let page = self.fetch(SourceEntity::Cycles, cursor).await?;
            self.begin_unit().await?;
            for v in &page.items {
                let n = N(v);
                let r = self.cycle(n).await;
                self.item_result("Cycle", n.id(), r)?;
            }
            let next = page.next_cursor;
            self.end_unit(Some(self.cp(Step::Cycles, next.clone())))
                .await?;
            match next {
                Some(c) => cursor = Some(c),
                None => return Ok(()),
            }
        }
    }

    async fn cycle(&mut self, n: N<'_>) -> AppResult<()> {
        let Some(team_src) = n.ref_id("team") else {
            return Err(AppError::invalid_request("cycle without team"));
        };
        if !self.selected.contains(team_src) {
            self.counters("Cycle").skipped += 1;
            return Ok(());
        }
        let Some(team) = self.target("Team", team_src).await? else {
            self.counters("Cycle").skipped += 1;
            return Ok(());
        };
        let number = n.int("number").filter(|x| *x > 0 && *x < i32::MAX as i64);
        let (Some(number), Some(starts), Some(ends)) = (number, n.ts("startsAt"), n.ts("endsAt"))
        else {
            return Err(AppError::invalid_request(
                "cycle without number or valid period",
            ));
        };
        if ends <= starts {
            return Err(AppError::invalid_request("cycle ends before it starts"));
        }
        let now = self.now;
        let created = n.ts("createdAt").unwrap_or(now).min(now);
        let arr = |k: &str| json!(n.floats(k, 1000));
        let r = row(vec![
            ("team_id", json!(team)),
            ("number", json!(number)),
            (
                "name",
                json!(n.text("name").map(|s| clean_text(s, 255, false))),
            ),
            (
                "description",
                json!(n.text("description").map(|s| clean_text(s, 5000, true))),
            ),
            ("starts_at", jts(Some(starts))),
            ("ends_at", jts(Some(ends))),
            ("completed_at", jts(n.ts("completedAt"))),
            ("scope_history", arr("scopeHistory")),
            ("completed_scope_history", arr("completedScopeHistory")),
            ("in_progress_scope_history", arr("inProgressScopeHistory")),
            ("issue_count_history", arr("issueCountHistory")),
            (
                "completed_issue_count_history",
                arr("completedIssueCountHistory"),
            ),
            ("archived_at", jts(n.ts("archivedAt"))),
            ("created_at", jts(Some(created))),
            (
                "updated_at",
                jts(n.ts("updatedAt").map(|t| t.min(now)).or(Some(created))),
            ),
        ]);
        let up = self
            .upsert("Cycle", n, ImportTable::Cycle, r, false)
            .await?;
        if up.outcome == Outcome::Created {
            self.event(DomainEvent::CycleCreated {
                cycle_id: CycleId(up.id),
                team_id: TeamId(team),
            })
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn step_initiatives(&mut self) -> HResult<()> {
        let nodes = self.fetch_all(SourceEntity::Initiatives).await?;
        self.begin_unit().await?;
        for v in &nodes {
            let n = N(v);
            let r = self.initiative(n).await;
            self.item_result("Initiative", n.id(), r)?;
        }
        // Kanten des DAG (Eltern-Initiativen) nach allen Initiativen; Zyklen werden abgelehnt.
        let all = self.fetch_all_unfiltered(SourceEntity::Initiatives).await?;
        let mut children: HashMap<String, Vec<String>> = HashMap::new();
        for v in &all {
            let n = N(v);
            let Some(child) = n.id() else { continue };
            let mut parents: Vec<&str> = n.ref_ids("parentInitiatives", 20);
            if let Some(p) = n.ref_id("parentInitiative") {
                parents.push(p);
            }
            for p in parents {
                if p == child || reachable(&children, child, p) {
                    self.note(
                        "warning",
                        "Initiative",
                        Some(child),
                        "the parent link would create a cycle and was skipped",
                    );
                    continue;
                }
                children
                    .entry(p.to_owned())
                    .or_default()
                    .push(child.to_owned());
                let (pt, ct) = (
                    self.target("Initiative", p).await?,
                    self.target("Initiative", child).await?,
                );
                if let (Some(pt), Some(ct)) = (pt, ct) {
                    let r = row(vec![
                        ("parent_id", json!(pt)),
                        ("child_id", json!(ct)),
                        ("created_at", jts(Some(self.now))),
                    ]);
                    self.tx()?
                        .imports()
                        .insert_row_ignore(ImportTable::InitiativeParent, &Value::Object(r))
                        .await?;
                }
            }
        }
        self.end_unit(None).await?;
        Ok(())
    }

    async fn initiative(&mut self, n: N<'_>) -> AppResult<()> {
        let src = n
            .id()
            .ok_or_else(|| AppError::invalid_request("initiative without id"))?
            .to_owned();
        let now = self.now;
        let created = n.ts("createdAt").unwrap_or(now).min(now);
        let owner = self.target_opt("User", n.ref_id("owner")).await?;
        let creator = self.target_opt("User", n.ref_id("creator")).await?;
        let id_for_md = src.clone();
        let content = self.md("Initiative", &id_for_md, n.str("content"), MAX_DOCUMENT);
        let status = match one_of(n.str("status"), INITIATIVE_STATUS) {
            Some(s) => s,
            None => {
                if n.str("status").is_some() {
                    self.note(
                        "warning",
                        "Initiative",
                        Some(&src),
                        "unknown initiative status; imported as Planned",
                    );
                }
                "Planned"
            }
        };
        let mut r = row(vec![
            ("name", json!(clean_title(n.str("name"), 255, "Initiative"))),
            (
                "description",
                json!(n.text("description").map(|s| clean_text(s, 255, false))),
            ),
            ("content", json!(content)),
            (
                "color",
                json!(n.str("color").map(|c| color_or(Some(c), "#5E6AD2"))),
            ),
            ("icon", json!(icon_or_none(n.str("icon")))),
            ("status", json!(status)),
            ("owner_id", json!(owner)),
            ("creator_id", json!(creator)),
            ("sort_order", json!(n.float("sortOrder").unwrap_or(0.0))),
            ("target_date", json!(n.date("targetDate"))),
            ("started_at", jts(n.ts("startedAt"))),
            ("completed_at", jts(n.ts("completedAt"))),
            ("archived_at", jts(n.ts("archivedAt"))),
            ("created_at", jts(Some(created))),
            (
                "updated_at",
                jts(n.ts("updatedAt").map(|t| t.min(now)).or(Some(created))),
            ),
        ]);
        if let Some(slug) = valid_slug(n.str("slugId")) {
            r.insert("slug_id".into(), json!(slug));
        }
        let is_new = self.mapping("Initiative", &src).await?.is_none();
        if is_new {
            let number = self
                .tx()?
                .imports()
                .next_import_number(ImportCounter::Initiative, None)
                .await?;
            r.insert("number".into(), json!(number));
        }
        let up = self
            .upsert("Initiative", n, ImportTable::Initiative, r.clone(), false)
            .await;
        let up = match up {
            Err(e) if e.kind == ErrorKind::Conflict && is_new => {
                // Name oder Slug schon vergeben.
                let alt = format!(
                    "{} (Linear)",
                    clean_text(n.str("name").unwrap_or("Initiative"), 240, false)
                );
                self.note(
                    "warning",
                    "Initiative",
                    Some(&src),
                    format!("initiative name already exists; imported as `{alt}`"),
                );
                r.insert("name".into(), json!(alt));
                r.remove("slug_id");
                self.upsert("Initiative", n, ImportTable::Initiative, r, false)
                    .await?
            }
            other => other?,
        };
        if up.outcome == Outcome::Created {
            self.event(DomainEvent::InitiativeCreated {
                initiative_id: InitiativeId(up.id),
            })
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn step_projects(&mut self, cursor: Option<String>) -> HResult<()> {
        let mut cursor = cursor;
        loop {
            let page = self.fetch(SourceEntity::Projects, cursor).await?;
            self.begin_unit().await?;
            for v in &page.items {
                let n = N(v);
                let r = self.project(n).await;
                self.item_result("Project", n.id(), r)?;
            }
            let next = page.next_cursor;
            self.end_unit(Some(self.cp(Step::Projects, next.clone())))
                .await?;
            match next {
                Some(c) => cursor = Some(c),
                None => return Ok(()),
            }
        }
    }

    async fn project(&mut self, n: N<'_>) -> AppResult<()> {
        let src = n
            .id()
            .ok_or_else(|| AppError::invalid_request("project without id"))?
            .to_owned();
        // Teams: nur importierte; das erste ist das Lead-Team.
        let mut teams: Vec<Uuid> = Vec::new();
        for t in n.ref_ids("teams", 50) {
            if self.selected.contains(t)
                && let Some(id) = self.target("Team", t).await?
            {
                teams.push(id);
            }
        }
        let Some(&lead_team) = teams.first() else {
            self.counters("Project").skipped += 1;
            self.note(
                "info",
                "Project",
                Some(&src),
                "none of the project's teams is part of this import; project skipped",
            );
            return Ok(());
        };
        let Some(status) = self.target_opt("ProjectStatus", n.ref_id("status")).await? else {
            return Err(AppError::invalid_request("project status was not imported"));
        };
        let now = self.now;
        let created = n.ts("createdAt").unwrap_or(now).min(now);
        let lead = self.target_opt("User", n.ref_id("lead")).await?;
        let creator = self.target_opt("User", n.ref_id("creator")).await?;
        let content = self.md("Project", &src, n.str("content"), MAX_DOCUMENT);
        let (start, target) = (n.date("startDate"), n.date("targetDate"));
        let start = match (&start, &target) {
            (Some(s), Some(t)) if s > t => {
                self.note(
                    "warning",
                    "Project",
                    Some(&src),
                    "start date is after the target date; start date dropped",
                );
                None
            }
            _ => start,
        };
        let mut r: Map<String, Value> = row(vec![
            ("name", json!(clean_title(n.str("name"), 255, "Project"))),
            ("lead_team_id", json!(lead_team)),
            (
                "description",
                json!(n.text("description").map(|s| clean_text(s, 255, false))),
            ),
            ("content", json!(content)),
            ("color", json!(color_or(n.str("color"), "#5E6AD2"))),
            ("icon", json!(icon_or_none(n.str("icon")))),
            ("status_id", json!(status)),
            (
                "priority",
                json!(
                    n.int("priority")
                        .filter(|p| (0..=4).contains(p))
                        .unwrap_or(0)
                ),
            ),
            ("sort_order", json!(n.float("sortOrder").unwrap_or(0.0))),
            ("lead_id", json!(lead)),
            ("creator_id", json!(creator)),
            ("start_date", json!(start)),
            ("target_date", json!(target)),
            ("started_at", jts(n.ts("startedAt"))),
            ("completed_at", jts(n.ts("completedAt"))),
            ("canceled_at", jts(n.ts("canceledAt"))),
            ("health", json!(one_of(n.str("health"), HEALTH))),
            ("archived_at", jts(n.ts("archivedAt"))),
            ("created_at", jts(Some(created))),
            (
                "updated_at",
                jts(n.ts("updatedAt").map(|t| t.min(now)).or(Some(created))),
            ),
        ]);
        if let Some(slug) = valid_slug(n.str("slugId")) {
            r.insert("slug_id".into(), json!(slug));
        }
        if self.mapping("Project", &src).await?.is_none() {
            let number = self
                .tx()?
                .imports()
                .next_import_number(ImportCounter::Project, Some(lead_team))
                .await?;
            r.insert("number".into(), json!(number));
        }
        let up = self
            .upsert("Project", n, ImportTable::Project, r, false)
            .await?;
        if !up.wrote() {
            return Ok(());
        }
        let ignore = |t: ImportTable, pairs: Vec<(&str, Value)>| (t, Value::Object(row(pairs)));
        let mut side: Vec<(ImportTable, Value)> = Vec::new();
        for t in &teams {
            side.push(ignore(
                ImportTable::ProjectTeam,
                vec![
                    ("project_id", json!(up.id)),
                    ("team_id", json!(t)),
                    ("created_at", jts(Some(now))),
                ],
            ));
        }
        for u in n.ref_ids("members", 200) {
            if let Some(uid) = self.target("User", u).await? {
                side.push(ignore(
                    ImportTable::ProjectMember,
                    vec![
                        ("project_id", json!(up.id)),
                        ("user_id", json!(uid)),
                        ("created_at", jts(Some(now))),
                    ],
                ));
            }
        }
        for l in n.ref_ids("labels", 50) {
            if let Some(lid) = self.target("ProjectLabel", l).await? {
                side.push(ignore(
                    ImportTable::ProjectToLabel,
                    vec![
                        ("project_id", json!(up.id)),
                        ("label_id", json!(lid)),
                        ("created_at", jts(Some(now))),
                    ],
                ));
            }
        }
        for i in n.ref_ids("initiatives", 50) {
            if let Some(iid) = self.target("Initiative", i).await? {
                side.push(ignore(
                    ImportTable::InitiativeToProject,
                    vec![
                        ("initiative_id", json!(iid)),
                        ("project_id", json!(up.id)),
                        ("created_at", jts(Some(now))),
                    ],
                ));
            }
        }
        for (t, v) in side {
            self.tx()?.imports().insert_row_ignore(t, &v).await?;
        }
        if up.outcome == Outcome::Created {
            self.event(DomainEvent::ProjectCreated {
                project_id: ProjectId(up.id),
            })
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn step_milestones(&mut self, cursor: Option<String>) -> HResult<()> {
        let mut cursor = cursor;
        loop {
            let page = self.fetch(SourceEntity::ProjectMilestones, cursor).await?;
            self.begin_unit().await?;
            for v in &page.items {
                let n = N(v);
                let r = self.milestone(n).await;
                self.item_result("ProjectMilestone", n.id(), r)?;
            }
            let next = page.next_cursor;
            self.end_unit(Some(self.cp(Step::Milestones, next.clone())))
                .await?;
            match next {
                Some(c) => cursor = Some(c),
                None => return Ok(()),
            }
        }
    }

    async fn milestone(&mut self, n: N<'_>) -> AppResult<()> {
        let src = n
            .id()
            .ok_or_else(|| AppError::invalid_request("milestone without id"))?
            .to_owned();
        let Some(project) = self.target_opt("Project", n.ref_id("project")).await? else {
            self.counters("ProjectMilestone").skipped += 1;
            return Ok(());
        };
        let now = self.now;
        let created = n.ts("createdAt").unwrap_or(now).min(now);
        let description = self.md(
            "ProjectMilestone",
            &src,
            n.str("description"),
            MAX_DESCRIPTION,
        );
        let r = row(vec![
            ("project_id", json!(project)),
            ("name", json!(clean_title(n.str("name"), 255, "Milestone"))),
            ("description", json!(description)),
            ("target_date", json!(n.date("targetDate"))),
            ("sort_order", json!(n.float("sortOrder").unwrap_or(0.0))),
            ("archived_at", jts(n.ts("archivedAt"))),
            ("created_at", jts(Some(created))),
            (
                "updated_at",
                jts(n.ts("updatedAt").map(|t| t.min(now)).or(Some(created))),
            ),
        ]);
        let up = self
            .upsert(
                "ProjectMilestone",
                n,
                ImportTable::ProjectMilestone,
                r,
                false,
            )
            .await?;
        if up.outcome == Outcome::Created {
            self.event(DomainEvent::ProjectMilestoneCreated {
                milestone_id: MilestoneId(up.id),
                project_id: ProjectId(project),
            })
            .await?;
        }
        Ok(())
    }
}

/// Ist `target` von `from` aus über Kind-Kanten erreichbar?
fn reachable(children: &HashMap<String, Vec<String>>, from: &str, target: &str) -> bool {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut stack = vec![from];
    while let Some(cur) = stack.pop() {
        if cur == target {
            return true;
        }
        if !seen.insert(cur) {
            continue;
        }
        if let Some(kids) = children.get(cur) {
            stack.extend(kids.iter().map(String::as_str));
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_detection() {
        let mut g: HashMap<String, Vec<String>> = HashMap::new();
        g.entry("a".into()).or_default().push("b".into());
        g.entry("b".into()).or_default().push("c".into());
        // Kante c -> a (parent c, child a) würde einen Zyklus bilden: a ist von c aus?? Prüfung: child=a, parent=c
        assert!(reachable(&g, "a", "c"));
        assert!(!reachable(&g, "c", "a"));
    }

    #[test]
    fn slug_validation() {
        assert_eq!(
            valid_slug(Some("0123456789AB")).as_deref(),
            Some("0123456789ab")
        );
        assert_eq!(valid_slug(Some("short")), None);
        assert_eq!(valid_slug(Some("zzzzzzzzzzzz")), None);
    }
}
