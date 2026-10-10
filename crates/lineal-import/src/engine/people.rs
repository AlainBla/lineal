//! Phase 1: User, Teams (Eltern vor Kindern), Mitgliedschaften.

use super::node::{N, clean_text, clean_title, color_or, icon_or_none, jts, one_of, row};
use super::{HResult, Outcome, Session};
use crate::port::SourceEntity;
use lineal_app::error::{AppError, AppResult};
use lineal_app::ports::repos::import::ImportTable;
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{TeamId, UserId};
use serde_json::{Value, json};
use uuid::Uuid;

const ESTIMATION: &[&str] = &["notUsed", "exponential", "fibonacci", "linear", "tShirt"];
const SORT_ON_STATE: &[&str] = &["top", "bottom", "none"];
const PERIODS: &[i64] = &[1, 3, 6, 9, 12];

impl Session<'_, '_> {
    /// Freier Anzeigename (`alice`, `alice2`, …).
    async fn free_display_name(&mut self, email: &str) -> AppResult<String> {
        let base = lineal_app::usecases::user::display_name_base(email);
        let mut candidate = base.clone();
        let mut i = 2;
        while self
            .tx()?
            .imports()
            .user_display_name_taken(&candidate)
            .await?
        {
            let suffix = i.to_string();
            let keep: String = base.chars().take(40 - suffix.len()).collect();
            candidate = format!("{keep}{suffix}");
            i += 1;
        }
        Ok(candidate)
    }

    pub(crate) async fn step_users(&mut self) -> HResult<()> {
        let nodes = self.fetch_all(SourceEntity::Users).await?;
        for chunk in nodes.chunks(self.eng.config.page_size.max(1) as usize) {
            self.begin_unit().await?;
            for v in chunk {
                let n = N(v);
                let r = self.user(n).await;
                self.item_result("User", n.id(), r)?;
            }
            self.end_unit(None).await?;
        }
        self.refresh_user_names().await?;
        Ok(())
    }

    async fn user(&mut self, n: N<'_>) -> AppResult<()> {
        let src = n
            .id()
            .ok_or_else(|| AppError::invalid_request("user without id"))?
            .to_owned();
        let checksum = super::node::checksum_of(n.0);
        if let Some(m) = self.mapping("User", &src).await? {
            self.cache.insert(("User", src), m.target_id);
            self.counters("User").unchanged += 1;
            return Ok(());
        }
        let email = n.text("email").map(str::to_lowercase);
        // 1. explizite Zuordnung, 2. E-Mail (case-insensitiv), 3. Platzhalter.
        let explicit = self.opts.user_mapping.get(&src).copied().flatten();
        let mut target: Option<Uuid> = None;
        if let Some(id) = explicit {
            if self.tx()?.users().get(UserId(id)).await?.is_some() {
                target = Some(id);
            } else {
                self.note(
                    "warning",
                    "User",
                    Some(&src),
                    "the mapped Lineal user does not exist; falling back to e-mail matching",
                );
            }
        }
        if target.is_none()
            && let Some(email) = &email
        {
            target = self.tx()?.imports().find_user_id_by_email(email).await?;
        }
        let target = match target {
            Some(t) => t,
            None => {
                let email = email
                    .ok_or_else(|| AppError::invalid_request("user without e-mail address"))?;
                if !email.contains('@') || email.len() > 254 || email.contains(char::is_whitespace)
                {
                    return Err(AppError::invalid_request(
                        "user has an invalid e-mail address",
                    ));
                }
                let id = self.new_id();
                let display = self.free_display_name(&email).await?;
                let role = if n.bool("admin").unwrap_or(false) {
                    "admin"
                } else if n.bool("guest").unwrap_or(false) {
                    "guest"
                } else {
                    "user"
                };
                let created = n.ts("createdAt").unwrap_or(self.now).min(self.now);
                let r = row(vec![
                    ("id", json!(id)),
                    ("name", json!(clean_title(n.str("name"), 255, &display))),
                    ("display_name", json!(display)),
                    ("email", json!(email)),
                    ("role", json!(role)),
                    ("active", json!(false)),
                    ("status", json!("imported")),
                    ("created_at", jts(Some(created))),
                    ("updated_at", jts(Some(created))),
                ]);
                self.tx()?
                    .imports()
                    .insert_row(ImportTable::User, &Value::Object(r))
                    .await?;
                self.event(DomainEvent::UserCreated {
                    user_id: UserId(id),
                })
                .await?;
                id
            }
        };
        self.put_mapping("User", &src, target, n, &checksum).await?;
        self.counters("User").created += 1;
        Ok(())
    }

    pub(crate) async fn step_teams(&mut self) -> HResult<()> {
        let nodes = self.fetch_all(SourceEntity::Teams).await?;
        // Eltern vor Kindern; Teams, deren Eltern nicht ausgewählt sind, werden Wurzeln.
        let mut pending: Vec<&Value> = nodes
            .iter()
            .filter(|v| N(v).id().is_some_and(|id| self.selected.contains(id)))
            .collect();
        let skipped = nodes.len() - pending.len();
        self.counters("Team").skipped += skipped as u64;
        self.begin_unit().await?;
        while !pending.is_empty() {
            let mut next = Vec::new();
            let before = pending.len();
            for v in pending {
                let n = N(v);
                let parent = n.ref_id("parent").filter(|p| self.selected.contains(*p));
                if let Some(p) = parent {
                    let mapped = self.target("Team", p).await?;
                    if mapped.is_none() {
                        next.push(v);
                        continue;
                    }
                }
                let r = self.team(n).await;
                self.item_result("Team", n.id(), r)?;
            }
            if next.len() == before {
                // Zyklus oder fehlende Eltern: Rest als Wurzeln importieren.
                for v in next.drain(..) {
                    let n = N(v);
                    self.note(
                        "warning",
                        "Team",
                        n.id(),
                        "parent team could not be resolved; imported as a top-level team",
                    );
                    let r = self.team_with_parent(n, None).await;
                    self.item_result("Team", n.id(), r)?;
                }
            }
            pending = next;
        }
        self.end_unit(None).await?;
        Ok(())
    }

    async fn team(&mut self, n: N<'_>) -> AppResult<()> {
        let parent_src = n.ref_id("parent").filter(|p| self.selected.contains(*p));
        let parent = match parent_src {
            Some(p) => self.target("Team", p).await?,
            None => {
                if n.ref_id("parent").is_some() {
                    self.note(
                        "warning",
                        "Team",
                        n.id(),
                        "the parent team is not part of this import; imported as a top-level team",
                    );
                }
                None
            }
        };
        self.team_with_parent(n, parent).await
    }

    async fn team_with_parent(&mut self, n: N<'_>, parent: Option<Uuid>) -> AppResult<()> {
        let now = self.now;
        let src_key = n
            .text("key")
            .ok_or_else(|| AppError::invalid_request("team without key"))?
            .to_owned();
        let key = self.opts.target_key(&src_key);
        let name = clean_title(n.str("name"), 80, &key);
        let tz = n
            .text("timezone")
            .filter(|t| t.parse::<chrono_tz::Tz>().is_ok())
            .unwrap_or("America/Los_Angeles");
        let period = |k: &str| n.int(k).filter(|p| PERIODS.contains(p));
        let archived = n.ts("archivedAt");
        let created = n.ts("createdAt").unwrap_or(now).min(now);
        let updated = n.ts("updatedAt").unwrap_or(created).min(now);
        let lock_until = now + self.eng.config.team_lock;
        let make_row = |name: &str| {
            row(vec![
                ("name", json!(name)),
                ("key", json!(key)),
                (
                    "description",
                    json!(n.text("description").map(|d| clean_text(d, 2000, true))),
                ),
                ("icon", json!(icon_or_none(n.str("icon")))),
                ("color", json!(color_or(n.str("color"), "#5E6AD2"))),
                ("parent_id", json!(parent)),
                (
                    "visibility",
                    json!(if n.bool("private").unwrap_or(false) {
                        "private"
                    } else {
                        "public"
                    }),
                ),
                ("timezone", json!(tz)),
                (
                    "triage_enabled",
                    json!(n.bool("triageEnabled").unwrap_or(false)),
                ),
                (
                    "require_priority_to_leave_triage",
                    json!(n.bool("requirePriorityToLeaveTriage").unwrap_or(false)),
                ),
                ("auto_close_period", json!(period("autoClosePeriod"))),
                (
                    "auto_archive_period",
                    json!(period("autoArchivePeriod").unwrap_or(6)),
                ),
                (
                    "group_issue_history",
                    json!(n.bool("groupIssueHistory").unwrap_or(true)),
                ),
                (
                    "set_issue_sort_order_on_state_change",
                    json!(
                        one_of(n.str("setIssueSortOrderOnStateChange"), SORT_ON_STATE)
                            .unwrap_or("top")
                    ),
                ),
                (
                    "issue_estimation_type",
                    json!(one_of(n.str("issueEstimationType"), ESTIMATION).unwrap_or("notUsed")),
                ),
                (
                    "issue_estimation_extended",
                    json!(n.bool("issueEstimationExtended").unwrap_or(false)),
                ),
                (
                    "issue_estimation_allow_zero",
                    json!(n.bool("issueEstimationAllowZero").unwrap_or(false)),
                ),
                (
                    "default_issue_estimate",
                    json!(
                        n.int("defaultIssueEstimate")
                            .filter(|e| *e >= 0)
                            .unwrap_or(1)
                    ),
                ),
                (
                    "cycles_enabled",
                    json!(n.bool("cyclesEnabled").unwrap_or(false)),
                ),
                (
                    "cycle_duration",
                    json!(
                        n.int("cycleDuration")
                            .filter(|d| (1..=8).contains(d))
                            .unwrap_or(2)
                    ),
                ),
                (
                    "cycle_cooldown_time",
                    json!(
                        n.int("cycleCooldownTime")
                            .filter(|d| (0..=4).contains(d))
                            .unwrap_or(0)
                    ),
                ),
                (
                    "cycle_start_day",
                    json!(
                        n.int("cycleStartDay")
                            .filter(|d| (0..=6).contains(d))
                            .unwrap_or(1)
                    ),
                ),
                (
                    "upcoming_cycle_count",
                    json!(
                        n.int("upcomingCycleCount")
                            .filter(|d| (1..=15).contains(d))
                            .unwrap_or(2)
                    ),
                ),
                ("retired_at", jts(archived)),
                ("import_locked_until", jts(Some(lock_until))),
                ("created_at", jts(Some(created))),
                ("updated_at", jts(Some(updated))),
            ])
        };
        let mut up = self
            .upsert("Team", n, ImportTable::Team, make_row(&name), false)
            .await;
        if let Err(e) = &up
            && e.kind == lineal_app::ErrorKind::Conflict
        {
            // Teamname bereits vergeben (Name ist je Workspace eindeutig): mit Zusatz erneut versuchen.
            let alt = format!("{} (Linear)", clean_text(&name, 70, false));
            self.note(
                "warning",
                "Team",
                n.id(),
                format!("team name already exists in Lineal; imported as `{alt}`"),
            );
            up = self
                .upsert("Team", n, ImportTable::Team, make_row(&alt), false)
                .await;
        }
        let up = up?;
        if up.outcome == Outcome::Created {
            self.event(DomainEvent::TeamCreated {
                team_id: TeamId(up.id),
            })
            .await?;
        }
        Ok(())
    }

    pub(crate) async fn step_memberships(&mut self, cursor: Option<String>) -> HResult<()> {
        let mut cursor = cursor;
        loop {
            let page = self.fetch(SourceEntity::TeamMemberships, cursor).await?;
            self.begin_unit().await?;
            for v in &page.items {
                let n = N(v);
                let r = self.membership(n).await;
                self.item_result("TeamMembership", n.id(), r)?;
            }
            let next = page.next_cursor;
            self.end_unit(Some(self.cp(super::Step::Memberships, next.clone())))
                .await?;
            match next {
                Some(c) => cursor = Some(c),
                None => return Ok(()),
            }
        }
    }

    async fn membership(&mut self, n: N<'_>) -> AppResult<()> {
        let Some(team_src) = n.ref_id("team") else {
            return Err(AppError::invalid_request("membership without team"));
        };
        if !self.selected.contains(team_src) {
            self.counters("TeamMembership").skipped += 1;
            return Ok(());
        }
        let team = self.target("Team", team_src).await?;
        let user = self.target_opt("User", n.ref_id("user")).await?;
        let (Some(team), Some(user)) = (team, user) else {
            self.counters("TeamMembership").skipped += 1;
            self.note(
                "warning",
                "TeamMembership",
                n.id(),
                "team or user was not imported; membership skipped",
            );
            return Ok(());
        };
        let created = n.ts("createdAt").unwrap_or(self.now).min(self.now);
        let r = row(vec![
            ("team_id", json!(team)),
            ("user_id", json!(user)),
            ("owner", json!(n.bool("owner").unwrap_or(false))),
            ("created_at", jts(Some(created))),
            (
                "updated_at",
                jts(n.ts("updatedAt").map(|t| t.min(self.now)).or(Some(created))),
            ),
        ]);
        let up = self
            .upsert("TeamMembership", n, ImportTable::TeamMembership, r, false)
            .await?;
        if up.outcome == Outcome::Created {
            self.event(DomainEvent::TeamMembershipCreated {
                team_id: TeamId(team),
                user_id: UserId(user),
            })
            .await?;
        }
        Ok(())
    }
}
