//! Einladungs-Persistenz (WP16). Inhärente Methoden von `PgTx`, weitergeleitet von `webhook.rs`.

use super::user::parse_role;
use crate::db_err;
use crate::store::PgTx;
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::webhook::{InvitationRecord, NewInvitation};
use lineal_app::{AppError, AppResult};
use lineal_domain::enums::UserRole;
use lineal_domain::ids::{OrgId, TeamId, UserId};
use uuid::Uuid;

type Row = (
    Uuid,
    Uuid,
    String,
    String,
    Vec<Uuid>,
    Uuid,
    String,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<Uuid>,
    DateTime<Utc>,
);

const COLS: &str = "id, organization_id, email, role, team_ids, inviter_id, status, expires_at, last_sent_at, \
    accepted_at, accepted_user_id, created_at";

fn record(r: Row) -> AppResult<InvitationRecord> {
    let (
        id,
        org,
        email,
        role,
        teams,
        inviter,
        status,
        expires_at,
        last_sent_at,
        accepted_at,
        accepted_user,
        created_at,
    ) = r;
    Ok(InvitationRecord {
        id,
        org_id: OrgId(org),
        email,
        role: parse_role(&role)?,
        team_ids: teams.into_iter().map(TeamId).collect(),
        inviter_id: UserId(inviter),
        status,
        expires_at,
        last_sent_at,
        accepted_at,
        accepted_user_id: accepted_user.map(UserId),
        created_at,
    })
}

impl PgTx {
    pub(crate) async fn insert_invitation_impl(&mut self, i: &NewInvitation) -> AppResult<()> {
        let teams: Vec<Uuid> = i.team_ids.iter().map(|t| t.0).collect();
        sqlx::query(
            "INSERT INTO invitation (id, organization_id, email, role, team_ids, inviter_id, token_hash, \
             status, expires_at, last_sent_at, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', $8, $9, $9, $9)",
        )
        .bind(i.id)
        .bind(self.org.0)
        .bind(&i.email)
        .bind(i.role.as_str())
        .bind(teams)
        .bind(i.inviter_id.0)
        .bind(&i.token_hash)
        .bind(i.expires_at)
        .bind(i.now)
        .execute(&mut *self.tx)
        .await
        .map_err(|e| {
            let unique = e
                .as_database_error()
                .and_then(|d| d.code())
                .is_some_and(|c| c == "23505");
            if unique {
                AppError::conflict("There is already a pending invitation for this email")
                    .with_details(serde_json::json!({ "field": "email" }))
            } else {
                db_err(e)
            }
        })?;
        Ok(())
    }

    pub(crate) async fn get_invitation_impl(
        &mut self,
        id: Uuid,
    ) -> AppResult<Option<InvitationRecord>> {
        let row: Option<Row> = sqlx::query_as(&format!(
            "SELECT {COLS} FROM invitation WHERE organization_id = $1 AND id = $2 AND archived_at IS NULL"
        ))
        .bind(self.org.0)
        .bind(id)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(record).transpose()
    }

    pub(crate) async fn list_invitations_impl(
        &mut self,
        pending_only: bool,
    ) -> AppResult<Vec<InvitationRecord>> {
        let rows: Vec<Row> = sqlx::query_as(&format!(
            "SELECT {COLS} FROM invitation WHERE organization_id = $1 AND archived_at IS NULL \
             AND (NOT $2 OR status = 'pending') ORDER BY created_at DESC, id DESC LIMIT 500"
        ))
        .bind(self.org.0)
        .bind(pending_only)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(record).collect()
    }

    pub(crate) async fn find_pending_invitation_impl(
        &mut self,
        email: &str,
        _now: DateTime<Utc>,
    ) -> AppResult<Option<InvitationRecord>> {
        // Die Eindeutigkeit gilt für `status = 'pending'` (auch abgelaufene, bis sie ersetzt werden).
        let row: Option<Row> = sqlx::query_as(&format!(
            "SELECT {COLS} FROM invitation WHERE organization_id = $1 AND lower(email) = lower($2) \
             AND status = 'pending' AND archived_at IS NULL"
        ))
        .bind(self.org.0)
        .bind(email)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(record).transpose()
    }

    pub(crate) async fn find_invitation_by_token_hash_impl(
        &mut self,
        hash: &str,
    ) -> AppResult<Option<InvitationRecord>> {
        let row: Option<Row> = sqlx::query_as(&format!(
            "SELECT {COLS} FROM invitation WHERE token_hash = $1 AND archived_at IS NULL"
        ))
        .bind(hash)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(record).transpose()
    }

    pub(crate) async fn renew_invitation_impl(
        &mut self,
        id: Uuid,
        token_hash: &str,
        expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE invitation SET token_hash = $3, expires_at = $4, last_sent_at = $5, status = 'pending', \
             version = version + 1, updated_at = $5 WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id)
        .bind(token_hash)
        .bind(expires_at)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    pub(crate) async fn set_invitation_status_impl(
        &mut self,
        id: Uuid,
        status: &str,
        accepted_user: Option<UserId>,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE invitation SET status = $3, accepted_user_id = COALESCE($4, accepted_user_id), \
             accepted_at = CASE WHEN $3 = 'accepted' THEN $5 ELSE accepted_at END, \
             version = version + 1, updated_at = $5 WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id)
        .bind(status)
        .bind(accepted_user.map(|u| u.0))
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    pub(crate) async fn activate_imported_user_impl(
        &mut self,
        user: UserId,
        name: &str,
        password_hash: Option<&str>,
        role: UserRole,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE \"user\" SET status = 'active', active = true, name = $3, \
             password_hash = COALESCE($4, password_hash), role = $5, version = version + 1, updated_at = $6 \
             WHERE organization_id = $1 AND id = $2 AND status = 'imported'",
        )
        .bind(self.org.0)
        .bind(user.0)
        .bind(name)
        .bind(password_hash)
        .bind(role.as_str())
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    pub(crate) async fn password_hash_of_account_impl(
        &mut self,
        user: UserId,
    ) -> AppResult<Option<String>> {
        let h: Option<Option<String>> = sqlx::query_scalar(
            "SELECT password_hash FROM \"user\" WHERE id = $1 AND status = 'active' AND active",
        )
        .bind(user.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(h.flatten())
    }
}
