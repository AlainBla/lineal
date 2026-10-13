//! usecases – Benutzer (docs/02 §6, docs/07 §3.1): lesen, auflisten, anlegen, Rolle, Suspendierung,
//! Passwort.

use super::audit::AuditEvent;
use super::team::{TeamView, emit};
use crate::app::App;
use crate::context::RequestCtx;
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Tx;
use crate::ports::repos::team::NewMembership;
use crate::ports::repos::user::{NewUser, Page, PageRequest, User, UserFilter};
use crate::resolve;
use crate::resolve::team::Visibility;
use lineal_domain::enums::UserRole;
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::UserId;
use serde_json::json;

/// Mindestlänge von Passwörtern.
pub const MIN_PASSWORD_LEN: usize = 8;

pub(crate) fn validate_email(email: &str) -> AppResult<()> {
    if is_valid_email(email) {
        Ok(())
    } else {
        Err(AppError::invalid_request("email is not a valid address")
            .with_details(json!({ "field": "email" })))
    }
}

/// Strenge Adressprüfung nach RFC 5321/5322 (ASCII, ohne Quoted-Strings und Kommentare):
/// genau ein `@`, Local-Part höchstens 64 Zeichen aus dem Atom-Zeichensatz mit Punkten nur
/// zwischen Atomen, Domain aus mindestens zwei Labels (je 1-63 Zeichen `a-z0-9-`, nicht mit
/// `-` beginnend/endend), gesamt höchstens 254 Zeichen.
pub(crate) fn is_valid_email(email: &str) -> bool {
    if email.len() > 254 || !email.is_ascii() {
        return false;
    }
    let mut parts = email.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let local_ok = !local.is_empty()
        && local.len() <= 64
        && local.split('.').all(|atom| {
            !atom.is_empty()
                && atom
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+/=?^_`{|}~-".contains(c))
        });
    let labels: Vec<&str> = domain.split('.').collect();
    let domain_ok = labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    local_ok && domain_ok
}

/// Prüft die erlaubten Einladungs-Domains (`allowedInviteDomains`); `field` benennt das Eingabefeld.
pub(crate) fn check_invite_domain(
    org: &crate::policy::OrgPolicy,
    email: &str,
    field: &str,
) -> AppResult<()> {
    if org.invite_domain_allowed(email) {
        return Ok(());
    }
    Err(
        AppError::invalid_request("This email domain is not allowed for invitations")
            .with_details(json!({ "field": field, "reason": "domain_not_allowed" })),
    )
}

pub(crate) fn validate_password(password: &str) -> AppResult<()> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(AppError::invalid_request(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        ))
        .with_details(json!({ "field": "password" })));
    }
    Ok(())
}

/// Basis des Anzeigenamens aus der E-Mail: Teil vor dem `@`, `^[a-z0-9._-]{1,40}$`.
pub fn display_name_base(email: &str) -> String {
    let local = email.split('@').next().unwrap_or("user");
    let mut s: String = local
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(40)
        .collect();
    if s.is_empty() || matches!(s.as_str(), "me" | "none") {
        s = format!("{s}user");
    }
    s
}

/// Erzeugt einen freien Anzeigenamen (`alice`, `alice2`, …).
pub(crate) async fn display_name_from_email(tx: &mut dyn Tx, email: &str) -> AppResult<String> {
    let base = display_name_base(email);
    let mut candidate = base.clone();
    let mut i = 2;
    while tx.users().display_name_taken(&candidate).await? {
        let suffix = i.to_string();
        let keep: String = base.chars().take(40 - suffix.len()).collect();
        candidate = format!("{keep}{suffix}");
        i += 1;
    }
    Ok(candidate)
}

/// Ein User mit seinen sichtbaren Teams.
#[derive(Debug, Clone, PartialEq)]
pub struct UserView {
    pub user: User,
    /// Mitgliedschaften, soweit der Aufrufer die Teams sehen darf.
    pub teams: Vec<TeamView>,
}

/// Filter für `list_users`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListUsersInput {
    pub query: Option<String>,
    /// Team-Referenz (Name, Key oder UUID).
    pub team: Option<String>,
    pub page: PageRequest,
}

/// Eingabe für `create_user`.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateUserInput {
    pub name: String,
    pub email: String,
    /// `admin`, `user` oder `guest`.
    pub role: UserRole,
    pub password: Option<String>,
    /// Teams (Referenzen), denen der User beitritt; Gäste brauchen mindestens eines.
    pub teams: Vec<String>,
}

/// Schreibende User-Operationen gelten nie für den System-Akteur (kein implizites Admin-Recht).
fn forbid_system(ctx: &RequestCtx) -> AppResult<()> {
    if matches!(ctx.actor, crate::context::Actor::System) {
        return Err(AppError::forbidden(
            "User administration is not available to system contexts.",
        ));
    }
    Ok(())
}

impl App {
    /// `get_user`: UUID, `"me"`, E-Mail oder Name.
    pub async fn get_user(&self, ctx: &RequestCtx, reference: &str) -> AppResult<UserView> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let user = resolve::user::resolve_in(&mut *tx, &snap, ctx, reference).await?;
        let visible = snap.visible_team_ids();
        let mut teams = Vec::new();
        for m in tx.teams().list_memberships_of_user(user.id).await? {
            if visible.contains(&m.team_id)
                && let Some(t) = snap.tree.get(m.team_id)
                && !t.trashed
            {
                teams.push(TeamView {
                    display_name: snap.tree.display_name(t.id),
                    team: t.clone(),
                });
            }
        }
        Ok(UserView { user, teams })
    }

    /// `list_users`: aktive User; Gäste sehen nur User ihrer Teams.
    pub async fn list_users(
        &self,
        ctx: &RequestCtx,
        input: ListUsersInput,
    ) -> AppResult<Page<User>> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let mut filter = UserFilter {
            query: input.query.clone(),
            ..Default::default()
        };
        if let Some(t) = &input.team {
            let team = resolve::team::resolve_in(&mut *tx, &snap, t, Visibility::Content).await?;
            filter.team_id = Some(team.id);
        }
        if snap.actor.role == UserRole::Guest {
            filter.in_any_team = Some(snap.actor.memberships.keys().copied().collect());
        }
        tx.users().list(&filter, &input.page).await
    }

    /// Admin-Mitgliederliste (nur Owner/Admin): alle User mit E-Mail, Rolle, Status und sichtbaren
    /// Teams, optional inklusive suspendierter. Neueste zuerst, höchstens `limit` (1..=250).
    pub async fn list_members(
        &self,
        ctx: &RequestCtx,
        include_suspended: bool,
        limit: i64,
    ) -> AppResult<Vec<UserView>> {
        forbid_system(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let is_admin = matches!(
            ctx.actor,
            crate::context::Actor::User {
                role: UserRole::Owner | UserRole::Admin,
                ..
            }
        );
        if !is_admin || !ctx.scopes.read || !ctx.scopes.admin {
            return Err(AppError::forbidden(
                "Only workspace admins can list members with their email addresses.",
            ));
        }
        let page = tx
            .users()
            .list(
                &UserFilter {
                    include_suspended,
                    ..Default::default()
                },
                &PageRequest {
                    limit: limit.clamp(1, 250),
                    order_by: crate::ports::repos::user::OrderBy::CreatedAt,
                    ..Default::default()
                },
            )
            .await?;
        let visible = snap.visible_team_ids();
        let mut out = Vec::with_capacity(page.items.len());
        for user in page.items {
            let mut teams = Vec::new();
            for m in tx.teams().list_memberships_of_user(user.id).await? {
                if visible.contains(&m.team_id)
                    && let Some(t) = snap.tree.get(m.team_id)
                    && !t.trashed
                {
                    teams.push(TeamView {
                        display_name: snap.tree.display_name(t.id),
                        team: t.clone(),
                    });
                }
            }
            out.push(UserView { user, teams });
        }
        Ok(out)
    }

    /// Legt einen User an (Recht: `invitationsRole`; Admin-Rolle zusätzlich `adminManagementRole`).
    pub async fn create_user(&self, ctx: &RequestCtx, input: CreateUserInput) -> AppResult<User> {
        forbid_system(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::InviteUsers)?;
        match input.role {
            UserRole::Admin => snap.require_workspace(ctx, Action::GrantAdminRole)?,
            UserRole::User | UserRole::Guest => {}
            UserRole::Owner | UserRole::App => {
                return Err(
                    AppError::invalid_request("role must be admin, user or guest")
                        .with_details(json!({ "field": "role" })),
                );
            }
        }
        if input.role == UserRole::Guest && !snap.org.limits.guests_enabled {
            return Err(AppError::limit_exceeded("Guests are disabled.")
                .with_details(json!({ "limit": "guestsEnabled" })));
        }
        if input.role == UserRole::Guest && input.teams.is_empty() {
            return Err(AppError::invalid_request("a guest needs at least one team")
                .with_details(json!({ "field": "teams" })));
        }
        let name = lineal_domain::rules::text::validate_single_line_name("name", &input.name, 255)?;
        let email = input.email.trim().to_string();
        validate_email(&email)?;
        check_invite_domain(&snap.org, &email, "email")?;
        if tx.users().find_by_email(&email).await?.is_some() {
            return Err(AppError::conflict("A user with this email already exists")
                .with_details(json!({ "field": "email" })));
        }
        let password_hash = match &input.password {
            Some(p) => {
                validate_password(p)?;
                Some(self.hasher.hash(p)?)
            }
            None => None,
        };
        let mut teams = Vec::new();
        for r in &input.teams {
            let team =
                resolve::team::resolve_in(&mut *tx, &snap, r, Visibility::Management).await?;
            let action = if input.role == UserRole::Guest {
                Action::AddGuest
            } else {
                Action::AddMember
            };
            snap.require_team(ctx, action, team.id)?;
            teams.push(team);
        }
        let now = self.clock.now();
        let id = UserId(self.ids.new_uuid());
        let display_name = display_name_from_email(&mut *tx, &email).await?;
        tx.users()
            .insert(&NewUser {
                id,
                name,
                display_name,
                email,
                role: input.role,
                password_hash,
                now,
            })
            .await?;
        for team in &teams {
            let chain: Vec<_> = if input.role == UserRole::Guest {
                vec![team.id]
            } else {
                std::iter::once(team.id)
                    .chain(snap.tree.ancestors(team.id).iter().map(|t| t.id))
                    .collect()
            };
            for t in chain {
                if tx.teams().get_membership(t, id).await?.is_none() {
                    tx.teams()
                        .insert_membership(&NewMembership {
                            id: self.ids.new_uuid(),
                            team_id: t,
                            user_id: id,
                            owner: false,
                            now,
                        })
                        .await?;
                    // B4.41: Mitgliedschaft per Admin-Anlage meldet das Membership-Ereignis.
                    emit(
                        &mut *tx,
                        ctx,
                        DomainEvent::TeamMembershipCreated {
                            team_id: t,
                            user_id: id,
                        },
                    )
                    .await?;
                }
            }
        }
        emit(&mut *tx, ctx, DomainEvent::UserCreated { user_id: id }).await?;
        let user = tx
            .users()
            .get(id)
            .await?
            .ok_or_else(|| AppError::internal("user vanished after insert"))?;
        tx.commit().await?;
        Ok(user)
    }

    /// Ändert die Rolle eines Users (docs/02 §6.4): Owner-Rolle nur durch Owner, Admin-Rolle
    /// über `adminManagementRole`, user ↔ guest durch Admins. Der letzte Owner bleibt Owner.
    pub async fn set_user_role(
        &self,
        ctx: &RequestCtx,
        user: &str,
        role: UserRole,
    ) -> AppResult<User> {
        forbid_system(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let target = resolve::user::resolve_in(&mut *tx, &snap, ctx, user).await?;
        if resolve::user::actor_user_id(ctx) == Some(target.id) {
            return Err(AppError::forbidden("You cannot change your own role.")
                .with_details(json!({ "reason": "self_role_change" })));
        }
        if role == UserRole::App || target.role == UserRole::App {
            return Err(
                AppError::invalid_request("app users cannot change their role")
                    .with_details(json!({ "field": "role" })),
            );
        }
        if role == UserRole::Owner || target.role == UserRole::Owner {
            snap.require_workspace(ctx, Action::UpdateWorkspaceLimits)?;
        } else if role == UserRole::Admin || target.role == UserRole::Admin {
            snap.require_workspace(ctx, Action::GrantAdminRole)?;
        } else {
            snap.require_workspace(ctx, Action::UpdateWorkspaceSettings)?;
        }
        if target.role == UserRole::Owner && role != UserRole::Owner {
            let owners = tx
                .users()
                .list(
                    &UserFilter::default(),
                    &PageRequest {
                        limit: 250,
                        ..Default::default()
                    },
                )
                .await?
                .items
                .iter()
                .filter(|u| u.role == UserRole::Owner)
                .count();
            if owners <= 1 {
                return Err(
                    AppError::invalid_request("the last owner cannot be demoted")
                        .with_details(json!({ "reason": "last_owner" })),
                );
            }
        }
        if role == UserRole::Guest && !snap.org.limits.guests_enabled {
            return Err(AppError::limit_exceeded("Guests are disabled.")
                .with_details(json!({ "limit": "guestsEnabled" })));
        }
        if role == UserRole::Guest
            && tx
                .teams()
                .list_memberships_of_user(target.id)
                .await?
                .is_empty()
        {
            return Err(AppError::invalid_request("a guest needs at least one team")
                .with_details(json!({ "field": "role" })));
        }
        let old_role = target.role;
        tx.users()
            .set_role(target.id, role, self.clock.now())
            .await?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new(
                "user.role_changed",
                "user",
                Some(target.id.0),
                json!({ "from": old_role.as_str(), "to": role.as_str() }),
            ),
        )
        .await?;
        emit(
            &mut *tx,
            ctx,
            DomainEvent::UserUpdated { user_id: target.id },
        )
        .await?;
        let user = tx
            .users()
            .get(target.id)
            .await?
            .ok_or_else(|| AppError::internal("user vanished"))?;
        tx.commit().await?;
        Ok(user)
    }

    /// Suspendiert oder reaktiviert einen User (Admin/Owner). Man kann sich nicht selbst und
    /// nicht den letzten Owner suspendieren.
    pub async fn set_user_active(
        &self,
        ctx: &RequestCtx,
        user: &str,
        active: bool,
    ) -> AppResult<User> {
        forbid_system(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::SuspendUser)?;
        let target = resolve::user::resolve_in(&mut *tx, &snap, ctx, user).await?;
        if target.role == UserRole::Owner {
            snap.require_workspace(ctx, Action::UpdateWorkspaceLimits)?; // nur Owner
        }
        if !active {
            if resolve::user::actor_user_id(ctx) == Some(target.id) {
                return Err(AppError::invalid_request("you cannot suspend yourself"));
            }
            if target.role == UserRole::Owner {
                let owners = tx
                    .users()
                    .list(
                        &UserFilter::default(),
                        &PageRequest {
                            limit: 250,
                            ..Default::default()
                        },
                    )
                    .await?
                    .items
                    .iter()
                    .filter(|u| u.role == UserRole::Owner)
                    .count();
                if owners <= 1 {
                    return Err(
                        AppError::invalid_request("the last owner cannot be suspended")
                            .with_details(json!({ "reason": "last_owner" })),
                    );
                }
            }
        }
        tx.users()
            .set_active(target.id, active, self.clock.now())
            .await?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new(
                if active {
                    "user.reactivated"
                } else {
                    "user.suspended"
                },
                "user",
                Some(target.id.0),
                json!({}),
            ),
        )
        .await?;
        if !active {
            // Alte Sessions, Keys und Tokens bleiben auch nach einer Entsperrung ungültig.
            tx.auth()
                .revoke_all_credentials_of_user(target.id, self.clock.now())
                .await?;
        }
        emit(
            &mut *tx,
            ctx,
            DomainEvent::UserUpdated { user_id: target.id },
        )
        .await?;
        let user = tx
            .users()
            .get(target.id)
            .await?
            .ok_or_else(|| AppError::internal("user vanished"))?;
        tx.commit().await?;
        Ok(user)
    }

    /// Setzt das Passwort: jeder für sich selbst, Admins/Owner für andere.
    pub async fn set_user_password(
        &self,
        ctx: &RequestCtx,
        user: &str,
        password: &str,
    ) -> AppResult<()> {
        forbid_system(ctx)?;
        validate_password(password)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        let target = resolve::user::resolve_in(&mut *tx, &snap, ctx, user).await?;
        if !ctx.scopes.write {
            return Err(AppError::forbidden(
                "Token scope does not allow write operations.",
            ));
        }
        if resolve::user::actor_user_id(ctx) != Some(target.id) {
            snap.require_workspace(ctx, Action::UpdateWorkspaceSettings)?;
            // Admins dürfen keine Owner-Passwörter zurücksetzen (Übernahme des Workspace).
            if target.role == UserRole::Owner {
                snap.require_workspace(ctx, Action::UpdateWorkspaceLimits)?;
            }
        }
        let hash = self.hasher.hash(password)?;
        tx.users()
            .set_password_hash(target.id, &hash, self.clock.now())
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_validation_is_strict() {
        for ok in [
            "alice@acme.example",
            "a.b+tag@x.io",
            "o'neil@sub.example.org",
        ] {
            assert!(is_valid_email(ok), "{ok}");
        }
        let long_local = format!("{}@example.test", "a".repeat(65));
        for bad in [
            "a@b@example.test",
            "a@example.test,b@example.test",
            "<script>@x.y",
            "a b@x.y",
            "a@x",
            "a@-x.y",
            "a@x..y",
            ".a@x.y",
            "a.@x.y",
            "a@x.y\n",
            "ä@x.y",
            "",
            long_local.as_str(),
        ] {
            assert!(!is_valid_email(bad), "{bad:?}");
        }
    }

    #[test]
    fn display_names_come_from_the_email() {
        assert_eq!(display_name_base("Alice@acme.example"), "alice");
        assert_eq!(display_name_base("a.b+tag@x.io"), "a.b-tag");
        assert_eq!(display_name_base("me@x.io"), "meuser");
        assert!(display_name_base(&format!("{}@x.io", "a".repeat(80))).len() <= 40);
    }

    #[test]
    fn email_and_password_validation() {
        assert!(validate_email("alice@acme.example").is_ok());
        for bad in ["", "alice", "@x.io", "a@b", "a b@x.io", "a@.io"] {
            assert!(validate_email(bad).is_err(), "{bad}");
        }
        assert!(validate_password("lineal-dev").is_ok());
        assert!(validate_password("short").is_err());
    }
}
