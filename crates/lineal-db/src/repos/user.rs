//! `UserRepo` für `PgTx`.

use super::org::conflict_or_internal;
use crate::db_err;
use crate::store::PgTx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use lineal_app::ports::repos::UserRepo;
use lineal_app::ports::repos::user::{
    NewUser, OrderBy, Page, PageCursor, PageRequest, ProfilePatch, User, UserFilter,
};
use lineal_app::{AppError, AppResult};
use lineal_domain::enums::UserRole;
use lineal_domain::ids::{OrgId, UserId};
use sqlx::{Postgres, QueryBuilder};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub(crate) struct UserRow {
    id: Uuid,
    organization_id: Uuid,
    name: String,
    display_name: String,
    email: String,
    role: String,
    active: bool,
    status: String,
    avatar_url: Option<String>,
    timezone: Option<String>,
    last_seen: Option<DateTime<Utc>>,
    version: i32,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

pub(crate) const USER_COLS: &str = "u.id, u.organization_id, u.name, u.display_name, u.email, u.role, \
    u.active, u.status, u.avatar_url, u.timezone, u.last_seen, u.version, u.created_at, u.updated_at";

pub(crate) fn parse_role(s: &str) -> AppResult<UserRole> {
    s.parse()
        .map_err(|_| AppError::internal("invalid user role in database"))
}

impl UserRow {
    fn into_user(self) -> AppResult<User> {
        Ok(User {
            id: UserId(self.id),
            org_id: OrgId(self.organization_id),
            name: self.name,
            display_name: self.display_name,
            email: self.email,
            role: parse_role(&self.role)?,
            active: self.active,
            status: self.status,
            avatar_url: self.avatar_url,
            timezone: self.timezone,
            last_seen: self.last_seen,
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[async_trait]
impl UserRepo for PgTx {
    async fn insert(&mut self, u: &NewUser) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO \"user\" (id, organization_id, name, display_name, email, role, password_hash, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)",
        )
        .bind(u.id.0)
        .bind(self.org.0)
        .bind(&u.name)
        .bind(&u.display_name)
        .bind(&u.email)
        .bind(u.role.as_str())
        .bind(&u.password_hash)
        .bind(u.now)
        .execute(&mut *self.tx)
        .await
        .map_err(|e| conflict_or_internal(e, "email or display name already in use"))?;
        Ok(())
    }

    async fn get(&mut self, id: UserId) -> AppResult<Option<User>> {
        let row = sqlx::query_as::<_, UserRow>(&format!(
            "SELECT {USER_COLS} FROM \"user\" u WHERE u.organization_id = $1 AND u.id = $2 AND u.archived_at IS NULL"
        ))
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(UserRow::into_user).transpose()
    }

    async fn find_by_email(&mut self, email: &str) -> AppResult<Option<User>> {
        let row = sqlx::query_as::<_, UserRow>(&format!(
            "SELECT {USER_COLS} FROM \"user\" u WHERE u.organization_id = $1 AND lower(u.email) = lower($2) AND u.archived_at IS NULL"
        ))
        .bind(self.org.0)
        .bind(email)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(UserRow::into_user).transpose()
    }

    async fn find_by_name(&mut self, name: &str) -> AppResult<Vec<User>> {
        let rows = sqlx::query_as::<_, UserRow>(&format!(
            "SELECT {USER_COLS} FROM \"user\" u WHERE u.organization_id = $1 AND u.active AND u.archived_at IS NULL \
             AND (lower(u.name) = lower($2) OR lower(u.display_name) = lower($2)) ORDER BY u.created_at, u.id"
        ))
        .bind(self.org.0)
        .bind(name)
        .fetch_all(&mut *self.tx)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(UserRow::into_user).collect()
    }

    async fn display_name_taken(&mut self, display_name: &str) -> AppResult<bool> {
        sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM \"user\" WHERE organization_id = $1 AND lower(display_name) = lower($2))",
        )
        .bind(self.org.0)
        .bind(display_name)
        .fetch_one(&mut *self.tx)
        .await
        .map_err(db_err)
    }

    async fn list(&mut self, filter: &UserFilter, page: &PageRequest) -> AppResult<Page<User>> {
        let order_col = match page.order_by {
            OrderBy::CreatedAt => "u.created_at",
            OrderBy::UpdatedAt => "u.updated_at",
        };
        let mut qb: QueryBuilder<'_, Postgres> = QueryBuilder::new(format!(
            "SELECT {USER_COLS}, {order_col} AS sort_key FROM \"user\" u WHERE u.organization_id = "
        ));
        qb.push_bind(self.org.0);
        qb.push(" AND u.archived_at IS NULL");
        match filter.query.as_deref().filter(|q| !q.is_empty()) {
            Some(q) => {
                // Deaktivierte User nur bei exakter E-Mail.
                if filter.include_suspended {
                    qb.push(" AND TRUE");
                } else {
                    qb.push(" AND (u.active OR lower(u.email) = lower(");
                    qb.push_bind(q.to_owned());
                    qb.push("))");
                }
                let pattern = format!("%{}%", escape_like(&q.to_lowercase()));
                qb.push(" AND (lower(u.name) LIKE ");
                qb.push_bind(pattern.clone());
                qb.push(" ESCAPE '\\' OR lower(u.display_name) LIKE ");
                qb.push_bind(pattern.clone());
                qb.push(" ESCAPE '\\' OR lower(u.email) LIKE ");
                qb.push_bind(pattern);
                qb.push(" ESCAPE '\\')");
            }
            None => {
                if !filter.include_suspended {
                    qb.push(" AND u.active");
                }
            }
        }
        if let Some(team) = filter.team_id {
            qb.push(
                " AND EXISTS (SELECT 1 FROM team_membership m WHERE m.organization_id = u.organization_id \
                 AND m.user_id = u.id AND m.archived_at IS NULL AND m.team_id = ",
            );
            qb.push_bind(team.0);
            qb.push(")");
        }
        if let Some(teams) = &filter.in_any_team {
            let ids: Vec<Uuid> = teams.iter().map(|t| t.0).collect();
            qb.push(
                " AND EXISTS (SELECT 1 FROM team_membership m WHERE m.organization_id = u.organization_id \
                 AND m.user_id = u.id AND m.archived_at IS NULL AND m.team_id = ANY(",
            );
            qb.push_bind(ids);
            qb.push("))");
        }
        if let Some(c) = &page.cursor {
            qb.push(format!(" AND ({order_col}, u.id) < ("));
            qb.push_bind(c.key);
            qb.push(", ");
            qb.push_bind(c.id);
            qb.push(")");
        }
        qb.push(format!(" ORDER BY {order_col} DESC, u.id DESC LIMIT "));
        qb.push_bind(page.limit + 1);

        #[derive(sqlx::FromRow)]
        struct Row {
            #[sqlx(flatten)]
            user: UserRow,
            sort_key: DateTime<Utc>,
        }
        let mut rows = qb
            .build_query_as::<Row>()
            .fetch_all(&mut *self.tx)
            .await
            .map_err(db_err)?;
        let has_next = rows.len() as i64 > page.limit;
        if has_next {
            rows.truncate(page.limit as usize);
        }
        let next = if has_next {
            rows.last().map(|r| PageCursor {
                key: r.sort_key,
                id: r.user.id,
            })
        } else {
            None
        };
        let items = rows
            .into_iter()
            .map(|r| r.user.into_user())
            .collect::<AppResult<Vec<_>>>()?;
        Ok(Page { items, next })
    }

    async fn get_password_hash(&mut self, id: UserId) -> AppResult<Option<String>> {
        let row: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT password_hash FROM \"user\" WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .fetch_optional(&mut *self.tx)
        .await
        .map_err(db_err)?;
        row.map(|r| r.0)
            .ok_or_else(|| AppError::not_found("Could not find referenced User."))
    }

    async fn update_profile(
        &mut self,
        id: UserId,
        p: &ProfilePatch,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut qb: QueryBuilder<Postgres> =
            QueryBuilder::new("UPDATE \"user\" SET version = version + 1, updated_at = ");
        qb.push_bind(now);
        if let Some(v) = &p.name {
            qb.push(", name = ").push_bind(v.clone());
        }
        if let Some(v) = &p.display_name {
            qb.push(", display_name = ").push_bind(v.clone());
        }
        if let Some(v) = &p.avatar_url {
            qb.push(", avatar_url = ").push_bind(v.clone());
        }
        if let Some(v) = &p.timezone {
            qb.push(", timezone = ").push_bind(v.clone());
        }
        qb.push(" WHERE organization_id = ").push_bind(self.org.0);
        qb.push(" AND id = ").push_bind(id.0);
        let res = qb
            .build()
            .execute(&mut *self.tx)
            .await
            .map_err(|e| conflict_or_internal(e, "This display name is already taken"))?;
        if res.rows_affected() == 0 {
            return Err(AppError::not_found("Could not find referenced User."));
        }
        Ok(())
    }

    async fn set_password_hash(
        &mut self,
        id: UserId,
        hash: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let res = sqlx::query(
            "UPDATE \"user\" SET password_hash = $3, version = version + 1, updated_at = $4 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(hash)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if res.rows_affected() == 0 {
            return Err(AppError::not_found("Could not find referenced User."));
        }
        Ok(())
    }

    async fn set_role(&mut self, id: UserId, role: UserRole, now: DateTime<Utc>) -> AppResult<()> {
        let res = sqlx::query(
            "UPDATE \"user\" SET role = $3, version = version + 1, updated_at = $4 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(role.as_str())
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if res.rows_affected() == 0 {
            return Err(AppError::not_found("Could not find referenced User."));
        }
        Ok(())
    }

    async fn set_active(&mut self, id: UserId, active: bool, now: DateTime<Utc>) -> AppResult<()> {
        let res = sqlx::query(
            "UPDATE \"user\" SET active = $3, version = version + 1, updated_at = $4 \
             WHERE organization_id = $1 AND id = $2",
        )
        .bind(self.org.0)
        .bind(id.0)
        .bind(active)
        .bind(now)
        .execute(&mut *self.tx)
        .await
        .map_err(db_err)?;
        if res.rows_affected() == 0 {
            return Err(AppError::not_found("Could not find referenced User."));
        }
        Ok(())
    }
}
