//! Abgleich (Sync): in Linear gelöschte, archivierte oder aus dem Umfang verschobene Objekte
//! werden in Lineal **archiviert** (nie hart gelöscht), sofern sie lokal seit dem Import nicht
//! geändert wurden; außerdem optional das Aufräumen früherer `Name (Linear)`-Duplikat-Labels.
//!
//! Nur mit `options.detectRemoved` (liest dafür alle IDs der Entität) bzw. `mergeLinearLabels`.
//! Bei Teilfehlern der Quelle oder unvollständiger Liste wird nichts archiviert.

use super::{HResult, Session};
use crate::port::{PageQuery, SourceEntity};
use lineal_app::error::AppResult;
use lineal_app::ports::repos::import::ImportTable;
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{DocumentId, InitiativeId, IssueId, ProjectId, TeamId};
use std::collections::HashSet;
use uuid::Uuid;

/// Obergrenze gelesener Seiten je Entität; wird sie erreicht, gilt die Liste als unvollständig.
const MAX_PAGES: usize = 4000;

impl Session<'_, '_> {
    pub(crate) async fn step_removed(&mut self) -> HResult<()> {
        if self.dry {
            return Ok(());
        }
        if self.opts.merge_labels {
            self.merge_linear_labels().await;
        }
        if !self.opts.detect_removed {
            return Ok(());
        }
        for (entity, ty, table) in [
            (SourceEntity::Issues, "Issue", ImportTable::Issue),
            (SourceEntity::Projects, "Project", ImportTable::Project),
            (
                SourceEntity::Initiatives,
                "Initiative",
                ImportTable::Initiative,
            ),
            (SourceEntity::Documents, "Document", ImportTable::Document),
        ] {
            let Some(present) = self.source_ids(entity).await? else {
                self.note(
                    "warning",
                    ty,
                    None,
                    "the list of objects could not be read completely; removed objects were not checked this time",
                );
                continue;
            };
            self.archive_missing(ty, table, &present).await?;
        }
        Ok(())
    }

    /// Alle IDs einer Entität im Umfang des Laufs (ohne Delta-Filter). `None`, wenn die Liste
    /// unvollständig oder unsicher ist (Teilfehler der Quelle, zu viele Seiten).
    async fn source_ids(&mut self, entity: SourceEntity) -> HResult<Option<HashSet<String>>> {
        let _ = self.eng.source.take_warnings();
        let mut ids = HashSet::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let page = self
                .guarded(self.eng.source.page(&PageQuery {
                    entity,
                    cursor: cursor.clone(),
                    updated_after: None,
                    limit: self.eng.config.page_size.clamp(1, 100),
                    scope: self.scope(),
                }))
                .await?;
            ids.extend(
                page.items
                    .iter()
                    .filter_map(|v| v["id"].as_str().map(str::to_owned)),
            );
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => {
                    let warned = !self.eng.source.take_warnings().is_empty();
                    return Ok((!warned).then_some(ids));
                }
            }
        }
        Ok(None)
    }

    async fn archive_missing(
        &mut self,
        ty: &'static str,
        table: ImportTable,
        present: &HashSet<String>,
    ) -> HResult<()> {
        let source = self.source().to_owned();
        let selected_teams: HashSet<Uuid> = {
            self.begin_unit().await?;
            let ids = self.mapped_team_ids().await?;
            self.end_unit(None).await?;
            ids.into_iter().collect()
        };
        let mut after: Option<String> = None;
        loop {
            self.begin_unit().await?;
            let page = self
                .tx()?
                .imports()
                .list_mappings(&source, ty, after.as_deref(), 200)
                .await?;
            if page.is_empty() {
                self.end_unit(None).await?;
                return Ok(());
            }
            after = page.last().map(|m| m.source_id.clone());
            for m in page.iter().filter(|m| !present.contains(&m.source_id)) {
                let r = self.archive_one(ty, table, m, &selected_teams).await;
                self.item_result(ty, Some(&m.source_id), r)?;
            }
            self.end_unit(None).await?;
        }
    }

    async fn archive_one(
        &mut self,
        ty: &'static str,
        table: ImportTable,
        m: &lineal_app::ports::repos::import::MappingRow,
        selected_teams: &HashSet<Uuid>,
    ) -> AppResult<()> {
        let team = if table == ImportTable::Issue {
            let t = self.tx()?.imports().row_team_id(table, m.target_id).await?;
            // Teams, die nicht mehr zum Abgleich gehören, werden nie archiviert.
            match t {
                Some(t) if selected_teams.contains(&t) => Some(t),
                _ => return Ok(()),
            }
        } else {
            None
        };
        let current = self
            .tx()?
            .imports()
            .row_updated_at(table, m.target_id)
            .await?;
        let Some(current) = current else {
            return Ok(());
        };
        if current > m.imported_at {
            self.counters(ty).conflicts += 1;
            self.note(
                "warning",
                ty,
                Some(&m.source_id),
                "removed or archived in Linear, but changed in Lineal since the import; it was kept",
            );
            return Ok(());
        }
        let now = self.now;
        if !self
            .tx()?
            .imports()
            .archive_row(table, m.target_id, now)
            .await?
        {
            return Ok(());
        }
        self.counters(ty).updated += 1;
        self.note(
            "info",
            ty,
            Some(&m.source_id),
            "archived in Lineal: deleted, archived or moved out of the synced scope in Linear",
        );
        let id = m.target_id;
        let ev = match table {
            ImportTable::Issue => team.map(|t| DomainEvent::IssueUpdated {
                previous: None,
                issue_id: IssueId(id),
                team_id: TeamId(t),
                changed: vec!["archivedAt".into()],
                mentioned_user_ids: vec![],
            }),
            ImportTable::Project => Some(DomainEvent::ProjectUpdated {
                project_id: ProjectId(id),
                changed: vec!["archivedAt".into()],
                previous_lead_id: None,
            }),
            ImportTable::Initiative => Some(DomainEvent::InitiativeUpdated {
                initiative_id: InitiativeId(id),
                changed: vec!["archivedAt".into()],
                previous_owner_id: None,
            }),
            ImportTable::Document => Some(DomainEvent::DocumentUpdated {
                document_id: DocumentId(id),
            }),
            _ => None,
        };
        if let Some(ev) = ev {
            self.event(ev).await?;
        }
        Ok(())
    }

    /// Führt `Name (Linear)`-Duplikate zusammen (Rechte des Starters; Fehler landen im Bericht).
    async fn merge_linear_labels(&mut self) {
        let run = self.run.clone();
        let result = async {
            let ctx = self.eng.app.import_run_ctx(self.org, &run).await?;
            self.eng.app.import_merge_linear_labels(&ctx).await
        }
        .await;
        match result {
            Ok(r) => {
                self.note(
                    "info",
                    "IssueLabel",
                    None,
                    format!(
                        "{} duplicate `(Linear)` label(s) were merged into the existing label",
                        r.merged
                    ),
                );
                for s in r.skipped {
                    self.note("warning", "IssueLabel", None, format!("not merged: {s}"));
                }
            }
            Err(e) => self.note(
                "warning",
                "IssueLabel",
                None,
                format!("the label clean-up could not run: {}", e.message),
            ),
        }
    }
}
