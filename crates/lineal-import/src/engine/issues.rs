//! Phase 4 und 5: Issues (Nummern aus Linear), Eltern-Bezüge, Relationen, History, Abos.

use super::node::{N, clean_text, clean_title, jts, one_of, row};
use super::{HResult, MAX_DESCRIPTION, Outcome, Session, Step};
use crate::port::SourceEntity;
use lineal_app::error::{AppError, AppResult, ErrorKind};
use lineal_app::ports::repos::import::{ImportTable, NewMapping};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{IssueId, IssueRelationId, TeamId, UserId};
use lineal_domain::rules::markdown::is_safe_url;
use serde_json::{Map, Value, json};
use uuid::Uuid;

const SLA_TYPES: &[&str] = &["all", "onlyBusinessDays"];
const RELATION_TYPES: &[&str] = &["blocks", "related", "duplicate", "similar"];
const LINK_TYPE: &str = "IssueParentLink";

impl Session<'_, '_> {
    pub(crate) async fn step_issues(&mut self, cursor: Option<String>) -> HResult<()> {
        let mut cursor = cursor;
        loop {
            let page = self.fetch(SourceEntity::Issues, cursor).await?;
            self.begin_unit().await?;
            for v in &page.items {
                let n = N(v);
                let r = self.issue(n).await;
                self.item_result("Issue", n.id(), r)?;
            }
            let next = page.next_cursor;
            self.end_unit(Some(self.cp(Step::Issues, next.clone())))
                .await?;
            match next {
                Some(c) => cursor = Some(c),
                None => return Ok(()),
            }
        }
    }

    async fn issue(&mut self, n: N<'_>) -> AppResult<()> {
        let src = n
            .id()
            .ok_or_else(|| AppError::invalid_request("issue without id"))?
            .to_owned();
        let Some(team_src) = n.ref_id("team") else {
            return Err(AppError::invalid_request("issue without team"));
        };
        if !self.selected.contains(team_src) {
            self.counters("Issue").skipped += 1;
            return Ok(());
        }
        let Some(team) = self.target("Team", team_src).await? else {
            self.counters("Issue").skipped += 1;
            return Ok(());
        };
        // Projektfilter: nur Issues der gewählten Projekte (Sub-Issues anderer Projekte bleiben draußen).
        if !self.opts.projects.is_empty()
            && !n
                .ref_id("project")
                .is_some_and(|p| self.opts.projects.contains(p))
        {
            self.counters("Issue").skipped += 1;
            return Ok(());
        }
        // Nummer aus Linear; ersatzweise aus der Kennung (`ENG-123`).
        let linear_number = n
            .int("number")
            .or_else(|| {
                n.str("identifier")
                    .and_then(|i| i.rsplit('-').next()?.parse().ok())
            })
            .filter(|x| *x > 0 && *x < i32::MAX as i64);
        // Bestehendes Zielteam: neue Nummern aus dem Zähler des Teams (die Linear-Nummer würde mit
        // vorhandenen Issues kollidieren); bereits importierte Issues behalten ihre Nummer.
        let into_existing = self.target_by_source.contains_key(team_src);
        let number = if into_existing {
            if self.mapping("Issue", &src).await?.is_none() {
                let first = self
                    .tx()?
                    .teams()
                    .allocate_issue_numbers(TeamId(team), 1)
                    .await?;
                Some(i64::from(first))
            } else {
                None
            }
        } else {
            Some(
                linear_number
                    .ok_or_else(|| AppError::invalid_request("issue without a valid number"))?,
            )
        };
        if n.ref_id("delegate").is_some() && self.reported_tags.insert("delegate".into()) {
            self.note(
                "warning",
                "Issue",
                Some(&src),
                "issues delegated to agents were imported without the delegate (not supported in Lineal)",
            );
        }
        let state = match self.target_opt("WorkflowState", n.ref_id("state")).await? {
            Some(s) => s,
            None => {
                self.note(
                    "warning",
                    "Issue",
                    Some(&src),
                    "workflow state was not imported; the team's default state was used",
                );
                self.tx()?
                    .imports()
                    .first_state_of_types(team, &["backlog", "unstarted"])
                    .await?
                    .ok_or_else(|| AppError::invalid_request("the team has no workflow state"))?
            }
        };
        let now = self.now;
        let created = n.ts("createdAt").unwrap_or(now).min(now);
        let updated = n.ts("updatedAt").unwrap_or(created).min(now);
        let assignee = self.target_opt("User", n.ref_id("assignee")).await?;
        let creator = self.target_opt("User", n.ref_id("creator")).await?;
        let project = self.target_opt("Project", n.ref_id("project")).await?;
        let milestone = if project.is_some() {
            self.target_opt("ProjectMilestone", n.ref_id("projectMilestone"))
                .await?
        } else {
            None
        };
        let cycle = self.target_opt("Cycle", n.ref_id("cycle")).await?;
        let parent_src = n.ref_id("parent");
        let parent = self.target_opt("Issue", parent_src).await?;
        let description = self.md("Issue", &src, n.str("description"), MAX_DESCRIPTION);
        let trashed = n.bool("trashed").unwrap_or(false);
        let state_entered = [n.ts("completedAt"), n.ts("canceledAt"), n.ts("startedAt")]
            .into_iter()
            .flatten()
            .max()
            .unwrap_or(created)
            .min(now);
        let sort = n.float("sortOrder").unwrap_or(0.0);
        if let Some(p) = project {
            // Ein Issue mit Projekt braucht die project_team-Zeile seines Teams (Linear fügt sie implizit hinzu).
            let r = row(vec![
                ("project_id", json!(p)),
                ("team_id", json!(team)),
                ("created_at", jts(Some(now))),
            ]);
            self.tx()?
                .imports()
                .insert_row_ignore(ImportTable::ProjectTeam, &Value::Object(r))
                .await?;
        }
        let mut r: Map<String, Value> = row(vec![
            ("team_id", json!(team)),
            ("number", json!(number)),
            (
                "title",
                json!(clean_title(n.str("title"), 255, "(untitled)")),
            ),
            ("description", json!(description)),
            ("state_id", json!(state)),
            ("state_entered_at", jts(Some(state_entered))),
            (
                "priority",
                json!(
                    n.int("priority")
                        .filter(|p| (0..=4).contains(p))
                        .unwrap_or(0)
                ),
            ),
            (
                "estimate",
                json!(
                    n.float("estimate")
                        .filter(|e| *e >= 0.0)
                        .map(|e| e.round() as i64)
                ),
            ),
            ("assignee_id", json!(assignee)),
            ("creator_id", json!(creator)),
            ("parent_id", json!(parent)),
            ("project_id", json!(project)),
            ("project_milestone_id", json!(milestone)),
            ("cycle_id", json!(cycle)),
            ("due_date", json!(n.date("dueDate"))),
            ("sort_order", json!(sort)),
            ("priority_sort_order", json!(sort)),
            ("started_at", jts(n.ts("startedAt"))),
            ("completed_at", jts(n.ts("completedAt"))),
            ("canceled_at", jts(n.ts("canceledAt"))),
            ("triaged_at", jts(n.ts("triagedAt"))),
            ("added_to_team_at", jts(Some(created))),
            ("added_to_cycle_at", jts(cycle.map(|_| created))),
            ("added_to_project_at", jts(project.map(|_| created))),
            ("auto_closed_at", jts(n.ts("autoClosedAt"))),
            ("auto_archived_at", jts(n.ts("autoArchivedAt"))),
            ("snoozed_until_at", jts(n.ts("snoozedUntilAt"))),
            ("sla_type", json!(one_of(n.str("slaType"), SLA_TYPES))),
            ("sla_started_at", jts(n.ts("slaStartedAt"))),
            ("sla_medium_risk_at", jts(n.ts("slaMediumRiskAt"))),
            ("sla_high_risk_at", jts(n.ts("slaHighRiskAt"))),
            ("sla_breaches_at", jts(n.ts("slaBreachesAt"))),
            ("trashed", json!(trashed)),
            ("trashed_at", jts(trashed.then_some(updated))),
            ("archived_at", jts(n.ts("archivedAt"))),
            ("created_at", jts(Some(created))),
            ("updated_at", jts(Some(updated))),
        ]);
        if number.is_none() {
            r.remove("number");
        }
        let up = self
            .upsert("Issue", n, ImportTable::Issue, r, false)
            .await?;
        if !up.wrote() {
            return Ok(());
        }
        if into_existing && up.outcome == Outcome::Created {
            self.linear_identifier_link(n, team_src, up.id, created)
                .await?;
        }
        // Eltern-Bezug, dessen Elternteil noch nicht importiert ist: vormerken (Phase 4b).
        if let Some(p) = parent_src
            && parent.is_none()
        {
            let link = NewMapping {
                import_run_id: self.run.id,
                source: self.source().to_owned(),
                source_type: LINK_TYPE.to_owned(),
                source_id: src.clone(),
                target_id: up.id,
                source_updated_at: None,
                checksum: Some(p.to_owned()),
                imported_at: now,
            };
            self.tx()?.imports().upsert_mapping(&link).await?;
        }
        // Labels und frühere Kennungen neu schreiben.
        if up.outcome == Outcome::Updated {
            self.tx()?
                .imports()
                .delete_rows_where(ImportTable::IssueToLabel, "issue_id", up.id)
                .await?;
        }
        for l in n.ref_ids("labels", 50) {
            let Some(label) = self.label_target(l).await? else {
                continue;
            };
            let group = match self.labels.get(l).and_then(|m| m.parent.clone()) {
                Some(g) if self.labels.get(&g).is_some_and(|m| m.is_group) => {
                    self.target("IssueLabel", &g).await?
                }
                _ => None,
            };
            let r = row(vec![
                ("issue_id", json!(up.id)),
                ("label_id", json!(label)),
                ("single_select_group_id", json!(group)),
                ("created_at", jts(Some(created))),
            ]);
            let inserted = self
                .tx()?
                .imports()
                .insert_row_ignore(ImportTable::IssueToLabel, &Value::Object(r))
                .await?;
            if !inserted {
                self.note("info", "Issue", Some(&src), "a label of an exclusive group was skipped (group already has a label on this issue)");
            }
        }
        if let Some(prev) = n.0.get("previousIdentifiers").and_then(Value::as_array) {
            for p in prev.iter().filter_map(Value::as_str).take(20) {
                let ident = p.trim().to_uppercase();
                let Some((prefix, num)) = ident.rsplit_once('-') else {
                    continue;
                };
                if prefix.is_empty() || num.parse::<u32>().is_err() || ident.len() > 32 {
                    continue;
                }
                let r = row(vec![
                    ("identifier", json!(ident)),
                    ("entity_type", json!("issue")),
                    ("issue_id", json!(up.id)),
                    ("team_key", json!(prefix)),
                    ("created_at", jts(Some(created))),
                ]);
                self.tx()?
                    .imports()
                    .insert_row_ignore(ImportTable::IssueIdentifierAlias, &Value::Object(r))
                    .await?;
            }
        }
        let ev = if up.outcome == Outcome::Created {
            DomainEvent::IssueCreated {
                issue_id: IssueId(up.id),
                team_id: TeamId(team),
                mentioned_user_ids: vec![],
            }
        } else {
            DomainEvent::IssueUpdated {
                previous: None,
                issue_id: IssueId(up.id),
                team_id: TeamId(team),
                changed: [
                    "title",
                    "description",
                    "stateId",
                    "priority",
                    "assigneeId",
                    "parentId",
                    "projectId",
                    "cycleId",
                ]
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
                mentioned_user_ids: vec![],
            }
        };
        self.event(ev).await?;
        Ok(())
    }

    /// Import in ein bestehendes Team: die Linear-Kennung (`KOL-12`) bleibt auffindbar, als Alias
    /// (löst im Suchfeld/Links auf, sofern sie nicht mit dem Key des Zielteams übereinstimmt) und
    /// immer als Anhang `Linear: KOL-12` mit Link in den Quell-Workspace.
    async fn linear_identifier_link(
        &mut self,
        n: N<'_>,
        team_src: &str,
        issue: Uuid,
        created: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<()> {
        let Some(ident) = n.text("identifier").map(str::to_uppercase) else {
            return Ok(());
        };
        let Some((prefix, num)) = ident.rsplit_once('-') else {
            return Ok(());
        };
        if prefix.is_empty() || num.parse::<u32>().is_err() || ident.len() > 32 {
            return Ok(());
        }
        let linear_key = self.team_keys.get(team_src).cloned().unwrap_or_default();
        let target_key = self
            .rewrite
            .key_map
            .get(&linear_key)
            .cloned()
            .unwrap_or_else(|| linear_key.clone());
        if !prefix.eq_ignore_ascii_case(&target_key) {
            let r = row(vec![
                ("identifier", json!(ident)),
                ("entity_type", json!("issue")),
                ("issue_id", json!(issue)),
                ("team_key", json!(prefix)),
                ("created_at", jts(Some(created))),
            ]);
            self.tx()?
                .imports()
                .insert_row_ignore(ImportTable::IssueIdentifierAlias, &Value::Object(r))
                .await?;
            self.plan_add("aliasesCreated");
        }
        if let Some(url) = n.text("url").filter(|u| u.len() <= 2048 && is_safe_url(u)) {
            let r = row(vec![
                ("id", json!(self.new_id())),
                ("issue_id", json!(issue)),
                ("url", json!(url)),
                ("title", json!(format!("Linear: {ident}"))),
                ("source_type", json!("linear")),
                ("created_at", jts(Some(self.now))),
                ("updated_at", jts(Some(self.now))),
            ]);
            self.tx()?
                .imports()
                .insert_row_ignore(ImportTable::Attachment, &Value::Object(r))
                .await?;
        }
        Ok(())
    }

    /// Relation, deren Gegenstück nicht importiert wird: als Anhang `Linear: blocks KOL-33` mit Link.
    async fn outside_relation_link(
        &mut self,
        n: N<'_>,
        ty: &str,
        mapped_issue: Uuid,
        mapped_is_source: bool,
    ) -> AppResult<()> {
        let other = if mapped_is_source {
            n.0.get("relatedIssue")
        } else {
            n.0.get("issue")
        };
        let other = other.map(N);
        let ident = other
            .and_then(|o| o.text("identifier"))
            .map(|i| clean_text(i, 40, false))
            .unwrap_or_else(|| "(unknown issue)".into());
        let verb = match (ty, mapped_is_source) {
            ("blocks", true) => "blocks",
            ("blocks", false) => "blocked by",
            ("duplicate", true) => "duplicate of",
            ("duplicate", false) => "duplicated by",
            ("similar", _) => "similar to",
            _ => "related to",
        };
        self.counters("IssueRelation").skipped += 1;
        self.plan_add("relationsOutside");
        self.note(
            "warning",
            "IssueRelation",
            n.id(),
            format!("the related issue {ident} is not part of this import; kept as a link (`{verb}`) on the issue"),
        );
        let Some(url) = other
            .and_then(|o| o.text("url"))
            .filter(|u| u.len() <= 2048 && is_safe_url(u))
        else {
            return Ok(());
        };
        let r = row(vec![
            ("id", json!(self.new_id())),
            ("issue_id", json!(mapped_issue)),
            ("url", json!(url)),
            ("title", json!(format!("Linear: {verb} {ident}"))),
            ("source_type", json!("linear")),
            ("created_at", jts(Some(self.now))),
            ("updated_at", jts(Some(self.now))),
        ]);
        self.tx()?
            .imports()
            .insert_row_ignore(ImportTable::Attachment, &Value::Object(r))
            .await?;
        Ok(())
    }

    /// Phase 4b: vorgemerkte Eltern-Bezüge auflösen.
    pub(crate) async fn step_issue_parents(&mut self) -> HResult<()> {
        let source = self.source().to_owned();
        loop {
            self.begin_unit().await?;
            let links = self
                .tx()?
                .imports()
                .list_mappings(&source, LINK_TYPE, None, 100)
                .await?;
            if links.is_empty() {
                self.end_unit(None).await?;
                return Ok(());
            }
            for m in &links {
                let parent_src = m.checksum.clone().unwrap_or_default();
                match self.target("Issue", &parent_src).await? {
                    Some(parent) if parent != m.target_id => {
                        self.tx()?
                            .imports()
                            .set_row_uuid(
                                ImportTable::Issue,
                                "parent_id",
                                m.target_id,
                                Some(parent),
                            )
                            .await?;
                    }
                    _ => {
                        self.note(
                            "warning",
                            "Issue",
                            Some(&m.source_id),
                            "the parent issue was not imported; the sub-issue link was dropped",
                        );
                    }
                }
                self.tx()?
                    .imports()
                    .delete_mapping(&source, LINK_TYPE, &m.source_id)
                    .await?;
            }
            self.end_unit(None).await?;
            if self.dry {
                // Im Dry-Run läuft alles in einer Transaktion; die Schleife endet, sobald alles aufgelöst ist.
                continue;
            }
        }
    }

    pub(crate) async fn step_relations(&mut self, cursor: Option<String>) -> HResult<()> {
        let mut cursor = cursor;
        loop {
            let page = self.fetch(SourceEntity::IssueRelations, cursor).await?;
            self.begin_unit().await?;
            for v in &page.items {
                let n = N(v);
                let r = self.relation(n).await;
                self.item_result("IssueRelation", n.id(), r)?;
            }
            let next = page.next_cursor;
            self.end_unit(Some(self.cp(Step::IssueRelations, next.clone())))
                .await?;
            match next {
                Some(c) => cursor = Some(c),
                None => return Ok(()),
            }
        }
    }

    async fn relation(&mut self, n: N<'_>) -> AppResult<()> {
        let Some(ty) = one_of(n.str("type"), RELATION_TYPES) else {
            self.counters("IssueRelation").errors += 1;
            self.note(
                "warning",
                "IssueRelation",
                n.id(),
                format!(
                    "unknown relation type `{}`; skipped",
                    clean_text(n.str("type").unwrap_or(""), 40, false)
                ),
            );
            return Ok(());
        };
        let a = self.target_opt("Issue", n.ref_id("issue")).await?;
        let b = self.target_opt("Issue", n.ref_id("relatedIssue")).await?;
        let (a, b) = match (a, b) {
            (Some(a), Some(b)) => (a, b),
            (Some(a), None) => return self.outside_relation_link(n, ty, a, true).await,
            (None, Some(b)) => return self.outside_relation_link(n, ty, b, false).await,
            (None, None) => {
                self.counters("IssueRelation").skipped += 1;
                return Ok(());
            }
        };
        if a == b {
            self.counters("IssueRelation").skipped += 1;
            return Ok(());
        }
        let created = n.ts("createdAt").unwrap_or(self.now).min(self.now);
        let r = row(vec![
            ("issue_id", json!(a)),
            ("related_issue_id", json!(b)),
            ("type", json!(ty)),
            ("created_at", jts(Some(created))),
            (
                "updated_at",
                jts(n.ts("updatedAt").map(|t| t.min(self.now)).or(Some(created))),
            ),
        ]);
        match self
            .upsert("IssueRelation", n, ImportTable::IssueRelation, r, true)
            .await
        {
            Ok(up) => {
                if up.outcome == Outcome::Created {
                    self.event(DomainEvent::IssueRelationCreated {
                        relation_id: IssueRelationId(up.id),
                        issue_id: IssueId(a),
                    })
                    .await?;
                }
                Ok(())
            }
            Err(e) if e.kind == ErrorKind::Conflict => {
                // Gegenrichtung bzw. zweite Duplicate-Relation existiert bereits.
                self.counters("IssueRelation").skipped += 1;
                self.note(
                    "info",
                    "IssueRelation",
                    n.id(),
                    "an equivalent relation already exists; skipped",
                );
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub(crate) async fn step_history(&mut self, cursor: Option<String>) -> HResult<()> {
        let mut cursor = cursor;
        loop {
            let page = self.fetch(SourceEntity::IssueHistory, cursor).await?;
            self.begin_unit().await?;
            for v in &page.items {
                let n = N(v);
                let r = self.history(n).await;
                self.item_result("IssueHistory", n.id(), r)?;
            }
            let next = page.next_cursor;
            self.end_unit(Some(self.cp(Step::IssueHistory, next.clone())))
                .await?;
            match next {
                Some(c) => cursor = Some(c),
                None => return Ok(()),
            }
        }
    }

    /// `from`/`to`-Paar eines Feldes (IDs über das Mapping übersetzt).
    async fn change(
        &mut self,
        changes: &mut Map<String, Value>,
        field: &str,
        from: Option<Uuid>,
        to: Option<Uuid>,
    ) {
        changes.insert(field.to_owned(), json!({ "from": from, "to": to }));
    }

    async fn history(&mut self, n: N<'_>) -> AppResult<()> {
        let Some(issue) = self.target_opt("Issue", n.ref_id("issue")).await? else {
            self.counters("IssueHistory").skipped += 1;
            return Ok(());
        };
        let mut changes: Map<String, Value> = Map::new();
        if n.ref_id("toState").is_some() || n.ref_id("fromState").is_some() {
            let (f, t) = (
                self.target_opt("WorkflowState", n.ref_id("fromState"))
                    .await?,
                self.target_opt("WorkflowState", n.ref_id("toState"))
                    .await?,
            );
            self.change(&mut changes, "stateId", f, t).await;
        }
        if n.ref_id("toAssignee").is_some() || n.ref_id("fromAssignee").is_some() {
            let (f, t) = (
                self.target_opt("User", n.ref_id("fromAssignee")).await?,
                self.target_opt("User", n.ref_id("toAssignee")).await?,
            );
            self.change(&mut changes, "assigneeId", f, t).await;
        }
        if n.int("toPriority").is_some() {
            changes.insert(
                "priority".into(),
                json!({ "from": n.int("fromPriority"), "to": n.int("toPriority") }),
            );
        }
        if n.ref_id("toTeam").is_some() || n.ref_id("fromTeam").is_some() {
            let (f, t) = (
                self.target_opt("Team", n.ref_id("fromTeam")).await?,
                self.target_opt("Team", n.ref_id("toTeam")).await?,
            );
            self.change(&mut changes, "teamId", f, t).await;
        }
        if n.ref_id("toProject").is_some() || n.ref_id("fromProject").is_some() {
            let (f, t) = (
                self.target_opt("Project", n.ref_id("fromProject")).await?,
                self.target_opt("Project", n.ref_id("toProject")).await?,
            );
            self.change(&mut changes, "projectId", f, t).await;
        }
        if n.ref_id("toCycle").is_some() || n.ref_id("fromCycle").is_some() {
            let (f, t) = (
                self.target_opt("Cycle", n.ref_id("fromCycle")).await?,
                self.target_opt("Cycle", n.ref_id("toCycle")).await?,
            );
            self.change(&mut changes, "cycleId", f, t).await;
        }
        if changes.is_empty() {
            self.counters("IssueHistory").skipped += 1;
            return Ok(());
        }
        let actor = self.target_opt("User", n.ref_id("actor")).await?;
        let created = n.ts("createdAt").unwrap_or(self.now).min(self.now);
        let r = row(vec![
            ("issue_id", json!(issue)),
            ("actor_id", json!(actor)),
            ("source", json!("import")),
            ("changes", Value::Object(changes)),
            ("created_at", jts(Some(created))),
        ]);
        self.upsert("IssueHistory", n, ImportTable::IssueHistory, r, true)
            .await?;
        Ok(())
    }

    pub(crate) async fn step_subscriptions(&mut self, cursor: Option<String>) -> HResult<()> {
        let mut cursor = cursor;
        loop {
            let page = self.fetch(SourceEntity::IssueSubscriptions, cursor).await?;
            self.begin_unit().await?;
            for v in &page.items {
                let n = N(v);
                let r = self.subscription(n).await;
                self.item_result("IssueSubscription", n.id(), r)?;
            }
            let next = page.next_cursor;
            self.end_unit(Some(self.cp(Step::IssueSubscriptions, next.clone())))
                .await?;
            match next {
                Some(c) => cursor = Some(c),
                None => return Ok(()),
            }
        }
    }

    async fn subscription(&mut self, n: N<'_>) -> AppResult<()> {
        let issue = self.target_opt("Issue", n.ref_id("issue")).await?;
        let user = self.target_opt("User", n.ref_id("user")).await?;
        let (Some(issue), Some(user)) = (issue, user) else {
            self.counters("IssueSubscription").skipped += 1;
            return Ok(());
        };
        let r = row(vec![
            ("issue_id", json!(issue)),
            ("user_id", json!(user)),
            ("active", json!(true)),
            ("reason", json!("manual")),
            ("created_at", jts(Some(self.now))),
        ]);
        let up = self
            .upsert(
                "IssueSubscription",
                n,
                ImportTable::IssueSubscription,
                r,
                true,
            )
            .await?;
        if up.outcome == Outcome::Created {
            self.event(DomainEvent::IssueSubscriptionCreated {
                issue_id: IssueId(issue),
                user_id: UserId(user),
            })
            .await?;
        }
        Ok(())
    }
}
