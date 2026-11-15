//! `TotpRepo` für `PgTx` (LIN-70): TOTP-Secret, Recovery-Codes, Schonfrist.

use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use lineal_app::AppResult;
use lineal_app::ports::repos::TotpRepo;
use lineal_app::ports::repos::totp::{GraceRow, NewUserTotp, RecoveryCodeRow, UserTotp};
use lineal_domain::ids::{OrgId, UserId};
use uuid::Uuid;

#[async_trait]
impl TotpRepo for PgTx {
    async fn get(&mut self, user: UserId) -> AppResult<Option<UserTotp>> {
        let row = sqlx::query_as::<_, (Vec<u8>, Vec<u8>, String, i32, Option<DateTime<Utc>>, Option<i64>)>(
            "SELECT secret_enc, dek_wrapped, key_backend, key_version, confirmed_at, last_used_step \
             FROM user_totp WHERE user_id = $1",
        )
        .bind(user.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(
            |(secret_enc, dek_wrapped, key_backend, key_version, confirmed_at, last_used_step)| {
                UserTotp {
                    user_id: user,
                    secret_enc,
                    dek_wrapped,
                    key_backend,
                    key_version,
                    confirmed_at,
                    last_used_step,
                }
            },
        ))
    }

    async fn upsert_pending(&mut self, rec: &NewUserTotp) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO user_totp (user_id, organization_id, secret_enc, dek_wrapped, key_backend, key_version, confirmed_at, last_used_step, created_at, updated_at) \
             VALUES ($1,$2,$3,$4,$5,$6,NULL,NULL,$7,$7) \
             ON CONFLICT (user_id) DO UPDATE SET \
               secret_enc = EXCLUDED.secret_enc, dek_wrapped = EXCLUDED.dek_wrapped, \
               key_backend = EXCLUDED.key_backend, key_version = EXCLUDED.key_version, \
               confirmed_at = NULL, last_used_step = NULL, updated_at = EXCLUDED.updated_at",
        )
        .bind(rec.user_id.0)
        .bind(rec.org_id.0)
        .bind(&rec.secret_enc)
        .bind(&rec.dek_wrapped)
        .bind(&rec.key_backend)
        .bind(rec.key_version)
        .bind(rec.now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn confirm(&mut self, user: UserId, now: DateTime<Utc>) -> AppResult<()> {
        sqlx::query(
            "UPDATE user_totp SET confirmed_at = $2, updated_at = $2 WHERE user_id = $1 AND confirmed_at IS NULL",
        )
        .bind(user.0)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn set_last_used_step(&mut self, user: UserId, step: i64) -> AppResult<()> {
        sqlx::query("UPDATE user_totp SET last_used_step = $2 WHERE user_id = $1")
            .bind(user.0)
            .bind(step)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn delete_all(&mut self, user: UserId) -> AppResult<()> {
        // Recovery-Codes und Schonfrist hängen per FK am Nutzer, aber wir räumen sie
        // hier explizit mit, da ein Reset nur TOTP entfernen soll, nicht den Nutzer.
        sqlx::query("DELETE FROM user_recovery_code WHERE user_id = $1")
            .bind(user.0)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        sqlx::query("DELETE FROM user_totp_grace WHERE user_id = $1")
            .bind(user.0)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        sqlx::query("DELETE FROM user_totp WHERE user_id = $1")
            .bind(user.0)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn replace_recovery_codes(
        &mut self,
        user: UserId,
        org: OrgId,
        hashes: &[Vec<u8>],
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        sqlx::query("DELETE FROM user_recovery_code WHERE user_id = $1")
            .bind(user.0)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        for h in hashes {
            sqlx::query(
                "INSERT INTO user_recovery_code (user_id, organization_id, code_hash, created_at) VALUES ($1,$2,$3,$4)",
            )
            .bind(user.0)
            .bind(org.0)
            .bind(h)
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        }
        Ok(())
    }

    async fn unused_recovery_codes(&mut self, user: UserId) -> AppResult<Vec<RecoveryCodeRow>> {
        let rows = sqlx::query_as::<_, (Uuid, Vec<u8>)>(
            "SELECT id, code_hash FROM user_recovery_code WHERE user_id = $1 AND used_at IS NULL ORDER BY created_at",
        )
        .bind(user.0)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(id, code_hash)| RecoveryCodeRow { id, code_hash })
            .collect())
    }

    async fn consume_recovery_code(&mut self, id: Uuid, now: DateTime<Utc>) -> AppResult<()> {
        sqlx::query("UPDATE user_recovery_code SET used_at = $2 WHERE id = $1 AND used_at IS NULL")
            .bind(id)
            .bind(now)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn grace_get(&mut self, user: UserId) -> AppResult<Option<GraceRow>> {
        let row = sqlx::query_as::<_, (DateTime<Utc>, i32)>(
            "SELECT started_at, logins_since FROM user_totp_grace WHERE user_id = $1",
        )
        .bind(user.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(row.map(|(started_at, logins_since)| GraceRow {
            started_at,
            logins_since,
        }))
    }

    async fn grace_bump(
        &mut self,
        user: UserId,
        org: OrgId,
        now: DateTime<Utc>,
    ) -> AppResult<GraceRow> {
        let (started_at, logins_since) = sqlx::query_as::<_, (DateTime<Utc>, i32)>(
            "INSERT INTO user_totp_grace (user_id, organization_id, started_at, logins_since, created_at) \
             VALUES ($1,$2,$3,1,$3) \
             ON CONFLICT (user_id) DO UPDATE SET logins_since = user_totp_grace.logins_since + 1 \
             RETURNING started_at, logins_since",
        )
        .bind(user.0)
        .bind(org.0)
        .bind(now)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(GraceRow {
            started_at,
            logins_since,
        })
    }

    async fn grace_clear(&mut self, user: UserId) -> AppResult<()> {
        sqlx::query("DELETE FROM user_totp_grace WHERE user_id = $1")
            .bind(user.0)
            .execute(&mut *self.tx)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn owners_past_deadline(
        &mut self,
        now: DateTime<Utc>,
        max_age_days: i64,
        max_logins: i32,
    ) -> AppResult<Vec<UserId>> {
        let cutoff = now - Duration::days(max_age_days);
        let rows = sqlx::query_scalar::<_, Uuid>(
            "SELECT g.user_id FROM user_totp_grace g \
             JOIN \"user\" u ON u.id = g.user_id AND u.role = 'owner' AND u.active \
             LEFT JOIN user_totp t ON t.user_id = g.user_id AND t.confirmed_at IS NOT NULL \
             WHERE t.user_id IS NULL AND (g.started_at <= $1 OR g.logins_since > $2)",
        )
        .bind(cutoff)
        .bind(max_logins)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        Ok(rows.into_iter().map(UserId).collect())
    }
}
