//! Team-Lebenszyklus-Persistenz (WP16): stilllegen, löschen, endgültig entfernen.
//! Inhärente Methoden von `PgTx`, weitergeleitet von `webhook.rs`.

use crate::db_err;
use crate::store::PgTx;
use chrono::{DateTime, Utc};
use lineal_app::AppResult;
use lineal_domain::ids::{OrgId, TeamId};
use uuid::Uuid;

impl PgTx {
    pub(crate) async fn set_team_retired_impl(
        &mut self,
        team: TeamId,
        retired_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE team SET retired_at = $3, version = version + 1, updated_at = $4 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(retired_at)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    pub(crate) async fn set_team_trashed_impl(
        &mut self,
        team: TeamId,
        trashed: bool,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE team SET trashed = $3, trashed_at = CASE WHEN $3 THEN $4 ELSE NULL END, \
             version = version + 1, updated_at = $4 WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(trashed)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    pub(crate) async fn trash_sole_projects_impl(
        &mut self,
        team: TeamId,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<Uuid>> {
        sqlx::query_scalar(
            "UPDATE project p SET trashed = true, trashed_at = $3, version = version + 1, updated_at = $3 \
             WHERE p.organization_id = $1 AND NOT p.trashed \
               AND EXISTS (SELECT 1 FROM project_team pt WHERE pt.organization_id = p.organization_id \
                           AND pt.project_id = p.id AND pt.team_id = $2) \
               AND NOT EXISTS (SELECT 1 FROM project_team pt WHERE pt.organization_id = p.organization_id \
                           AND pt.project_id = p.id AND pt.team_id <> $2) \
             RETURNING p.id",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(now)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    pub(crate) async fn restore_sole_projects_impl(
        &mut self,
        team: TeamId,
        trashed_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<Uuid>> {
        sqlx::query_scalar(
            "UPDATE project p SET trashed = false, trashed_at = NULL, version = version + 1, updated_at = $4 \
             WHERE p.organization_id = $1 AND p.trashed AND p.trashed_at = $3 \
               AND EXISTS (SELECT 1 FROM project_team pt WHERE pt.organization_id = p.organization_id \
                           AND pt.project_id = p.id AND pt.team_id = $2) \
               AND NOT EXISTS (SELECT 1 FROM project_team pt WHERE pt.organization_id = p.organization_id \
                           AND pt.project_id = p.id AND pt.team_id <> $2) \
             RETURNING p.id",
        )
        .bind(self.org.0)
        .bind(team.0)
        .bind(trashed_at)
        .bind(now)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    pub(crate) async fn shared_team_projects_impl(
        &mut self,
        team: TeamId,
    ) -> AppResult<Vec<(Uuid, TeamId)>> {
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT p.id, p.lead_team_id FROM project p WHERE p.organization_id = $1 AND NOT p.trashed \
               AND EXISTS (SELECT 1 FROM project_team pt WHERE pt.organization_id = p.organization_id \
                           AND pt.project_id = p.id AND pt.team_id = $2) \
               AND EXISTS (SELECT 1 FROM project_team pt WHERE pt.organization_id = p.organization_id \
                           AND pt.project_id = p.id AND pt.team_id <> $2) \
             ORDER BY p.created_at, p.id",
        )
        .bind(self.org.0)
        .bind(team.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(|(p, l)| (p, TeamId(l))).collect())
    }

    pub(crate) async fn project_issue_counts_impl(
        &mut self,
        project: Uuid,
    ) -> AppResult<Vec<(TeamId, i64)>> {
        let rows: Vec<(Uuid, i64)> = sqlx::query_as(
            "SELECT team_id, count(*) FROM issue WHERE organization_id = $1 AND project_id = $2 AND NOT trashed \
             GROUP BY team_id",
        )
        .bind(self.org.0)
        .bind(project)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(|(t, n)| (TeamId(t), n)).collect())
    }

    pub(crate) async fn list_teams_due_for_purge_impl(
        &mut self,
        before: DateTime<Utc>,
    ) -> AppResult<Vec<(OrgId, TeamId)>> {
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT organization_id, id FROM team WHERE trashed AND trashed_at IS NOT NULL AND trashed_at <= $1 \
             ORDER BY trashed_at LIMIT 100",
        )
        .bind(before)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(o, t)| (OrgId(o), TeamId(t)))
            .collect())
    }

    /// Löscht das Team samt Inhalt. Reihenfolge nach den Fremdschlüsseln: Verweise lösen, Dokumente,
    /// Issues, Projekte, Cycles, Labels, Templates, Status, zuletzt das Team.
    pub(crate) async fn purge_team_impl(&mut self, team: TeamId) -> AppResult<()> {
        let (org, t) = (self.org.0, team.0);
        // `document_content` ist polymorph ohne Fremdschlüssel (Schema 0016): vor den Besitzern räumen.
        let owners = "(owner_type = 'issue' AND owner_id IN (SELECT id FROM issue WHERE organization_id = $1 AND team_id = $2)) \
             OR (owner_type = 'document' AND owner_id IN (SELECT id FROM document WHERE organization_id = $1 AND (team_id = $2 \
                 OR issue_id IN (SELECT id FROM issue WHERE organization_id = $1 AND team_id = $2) \
                 OR cycle_id IN (SELECT id FROM cycle WHERE organization_id = $1 AND team_id = $2)))) \
             OR (owner_type = 'project' AND owner_id IN (SELECT p.id FROM project p WHERE p.organization_id = $1 \
                 AND EXISTS (SELECT 1 FROM project_team pt WHERE pt.project_id = p.id AND pt.team_id = $2) \
                 AND NOT EXISTS (SELECT 1 FROM project_team pt WHERE pt.project_id = p.id AND pt.team_id <> $2)))";
        for sql in [
            format!(
                "DELETE FROM document_content_history WHERE organization_id = $1 AND document_content_id IN \
                 (SELECT id FROM document_content WHERE organization_id = $1 AND ({owners}))"
            ),
            format!("DELETE FROM document_content WHERE organization_id = $1 AND ({owners})"),
        ] {
            sqlx::query(&sql)
                .bind(org)
                .bind(t)
                .execute(&mut *self.tx)
                .await
                .map_err(db_err)?;
        }
        let steps: [&str; 21] = [
            // Verweise aus anderen Teams/Objekten lösen
            "UPDATE team SET parent_id = NULL WHERE organization_id = $1 AND parent_id = $2",
            "UPDATE team SET default_issue_state_id = NULL, triage_issue_state_id = NULL, auto_close_state_id = NULL \
             WHERE organization_id = $1 AND id = $2",
            "UPDATE workflow_state SET inherited_from_id = NULL WHERE organization_id = $1 AND inherited_from_id IN \
             (SELECT id FROM workflow_state WHERE organization_id = $1 AND team_id = $2)",
            "UPDATE issue_label SET inherited_from_id = NULL WHERE organization_id = $1 AND inherited_from_id IN \
             (SELECT id FROM issue_label WHERE organization_id = $1 AND team_id = $2)",
            "UPDATE cycle SET inherited_from_id = NULL WHERE organization_id = $1 AND inherited_from_id IN \
             (SELECT id FROM cycle WHERE organization_id = $1 AND team_id = $2)",
            "UPDATE initiative SET lead_team_id = NULL WHERE organization_id = $1 AND lead_team_id = $2",
            // Dokumente
            "DELETE FROM document WHERE organization_id = $1 AND (team_id = $2 \
               OR issue_id IN (SELECT id FROM issue WHERE organization_id = $1 AND team_id = $2) \
               OR cycle_id IN (SELECT id FROM cycle WHERE organization_id = $1 AND team_id = $2))",
            // Issues (Kinder, Kommentare, Anhänge, Verlauf … hängen per ON DELETE CASCADE daran)
            "UPDATE issue SET parent_id = NULL WHERE organization_id = $1 AND team_id <> $2 AND parent_id IN \
             (SELECT id FROM issue WHERE organization_id = $1 AND team_id = $2)",
            "DELETE FROM issue WHERE organization_id = $1 AND team_id = $2",
            // Projekte: nur dieses Team -> löschen; sonst Beteiligung entfernen
            "DELETE FROM project WHERE organization_id = $1 AND NOT EXISTS (SELECT 1 FROM project_team pt \
               WHERE pt.organization_id = project.organization_id AND pt.project_id = project.id AND pt.team_id <> $2) \
               AND EXISTS (SELECT 1 FROM project_team pt WHERE pt.organization_id = project.organization_id \
               AND pt.project_id = project.id AND pt.team_id = $2)",
            "DELETE FROM project_team WHERE organization_id = $1 AND team_id = $2",
            "DELETE FROM project_label WHERE organization_id = $1 AND team_id = $2",
            "DELETE FROM project_status WHERE organization_id = $1 AND team_id = $2",
            // Cycles, Labels, Templates
            "DELETE FROM cycle WHERE organization_id = $1 AND team_id = $2",
            "UPDATE issue_label SET parent_id = NULL WHERE organization_id = $1 AND parent_id IN \
             (SELECT id FROM issue_label WHERE organization_id = $1 AND team_id = $2)",
            "DELETE FROM issue_label WHERE organization_id = $1 AND team_id = $2",
            "DELETE FROM template WHERE organization_id = $1 AND team_id = $2",
            // Status (Team-FK würde sie ohnehin kaskadieren)
            "DELETE FROM workflow_state WHERE organization_id = $1 AND team_id = $2",
            "DELETE FROM team_membership WHERE organization_id = $1 AND team_id = $2",
            "DELETE FROM team_key WHERE organization_id = $1 AND team_id = $2",
            "DELETE FROM team WHERE organization_id = $1 AND id = $2",
        ];
        for sql in steps {
            sqlx::query(sql)
                .bind(org)
                .bind(t)
                .execute(&mut *self.tx)
                .await
                .map_err(db_err)?;
        }
        Ok(())
    }
}
