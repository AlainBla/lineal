//! usecases – Einladungen (WP16, docs/01 §2.11, docs/08 F1/F16): einladen, erneut senden,
//! widerrufen, annehmen.
//!
//! Das Token (96 Hex-Zeichen) steht nur im Link der E-Mail (`{base}/invite/{token}`); gespeichert
//! wird `hash_token` = SHA-256. Eine Einladung gilt 7 Tage. Die Annahme verknüpft einen importierten
//! Platzhalter-User gleicher E-Mail, statt einen zweiten User anzulegen.

use super::auth::{LoginInput, hash_token};
use super::team::emit;
use super::user::{display_name_from_email, validate_email, validate_password};
use crate::app::App;
use crate::context::{Actor, RequestCtx};
use crate::error::{AppError, AppResult};
use crate::policy::{Action, PolicySnapshot};
use crate::ports::Mail;
use crate::ports::repos::team::NewMembership;
use crate::ports::repos::user::NewUser;
use crate::ports::repos::webhook::{InvitationRecord, NewInvitation};
use crate::resolve;
use crate::resolve::team::Visibility;
use chrono::Duration;
use lineal_domain::enums::UserRole;
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{TeamId, UserId};
use serde_json::json;
use uuid::Uuid;

/// Gültigkeit einer Einladung in Tagen.
pub const INVITATION_TTL_DAYS: i64 = 7;
/// Höchstzahl E-Mail-Adressen je Aufruf.
pub const MAX_INVITES_PER_CALL: usize = 50;

/// Eingabe für `invite_users`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InviteUsersInput {
    pub emails: Vec<String>,
    /// `admin`, `user` oder `guest`.
    pub role: Option<UserRole>,
    /// Teams (Referenzen), denen der Eingeladene beitritt; Gäste brauchen mindestens eines.
    pub teams: Vec<String>,
}

/// Neu angelegte Einladung. `token` ist der Klartext für den Link (nur hier, nie gespeichert);
/// API-Adapter geben ihn **nicht** an den Aufrufer weiter.
#[derive(Debug, Clone, PartialEq)]
pub struct CreatedInvitation {
    pub invitation: InvitationRecord,
    pub token: String,
}

/// Eingabe für `accept_invitation`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AcceptInvitationInput {
    pub token: String,
    /// Anzeigename; Default: Teil der E-Mail vor dem `@` bzw. Name des Platzhalter-Users.
    pub name: Option<String>,
    /// Passwort des neuen Zugangs (mind. 8 Zeichen); Pflicht ohne Konto-Session.
    pub password: Option<String>,
}

/// Ergebnis der Annahme.
#[derive(Debug, Clone, PartialEq)]
pub struct AcceptedInvitation {
    pub org_id: lineal_domain::ids::OrgId,
    pub url_key: String,
    pub user_id: UserId,
    /// Neue Session, wenn ein Passwort gesetzt wurde.
    pub login: Option<super::auth::Login>,
}

fn invitation_not_found() -> AppError {
    AppError::not_found("Could not find referenced Invitation.")
        .with_details(json!({ "type": "Invitation" }))
}

fn random_token() -> String {
    format!(
        "{}{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

fn forbid_system(ctx: &RequestCtx) -> AppResult<()> {
    if matches!(ctx.actor, Actor::System) {
        return Err(AppError::forbidden(
            "Invitations are not available to system contexts.",
        ));
    }
    Ok(())
}

impl App {
    fn invitation_mail(&self, to: &str, org_name: &str, token: &str) -> Mail {
        let link = format!("{}/invite/{token}", self.config.base_url.trim_end_matches('/'));
        Mail {
            to: to.to_string(),
            subject: format!("You have been invited to {org_name} on Lineal"),
            body: format!(
                "You have been invited to join {org_name}.\n\nAccept the invitation: {link}\n\nThe link is valid for {INVITATION_TTL_DAYS} days."
            ),
        }
    }

    /// Lädt E-Mail-Adressen ein (Recht `invitationsRole`, Scope `write`). Gäste brauchen mindestens
    /// ein Team und `guestsEnabled`. Je Adresse gibt es höchstens eine offene Einladung; ist die
    /// Adresse bereits ein aktiver User, schlägt der Aufruf mit `conflict` fehl (nichts wird angelegt).
    pub async fn invite_users(
        &self,
        ctx: &RequestCtx,
        input: InviteUsersInput,
    ) -> AppResult<Vec<CreatedInvitation>> {
        forbid_system(ctx)?;
        let role = input.role.unwrap_or(UserRole::User);
        let mut emails: Vec<String> = Vec::new();
        for e in &input.emails {
            let e = e.trim().to_string();
            validate_email(&e)?;
            if !emails.iter().any(|x| x.eq_ignore_ascii_case(&e)) {
                emails.push(e);
            }
        }
        if emails.is_empty() || emails.len() > MAX_INVITES_PER_CALL {
            return Err(AppError::invalid_request(format!(
                "emails must contain 1-{MAX_INVITES_PER_CALL} addresses"
            ))
            .with_details(json!({ "field": "emails" })));
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::InviteUsers)?;
        match role {
            UserRole::Admin => snap.require_workspace(ctx, Action::GrantAdminRole)?,
            UserRole::User | UserRole::Guest => {}
            UserRole::Owner | UserRole::App => {
                return Err(
                    AppError::invalid_request("role must be admin, user or guest")
                        .with_details(json!({ "field": "role" })),
                );
            }
        }
        if role == UserRole::Guest && !snap.org.limits.guests_enabled {
            return Err(AppError::limit_exceeded("Guests are disabled.")
                .with_details(json!({ "limit": "guestsEnabled" })));
        }
        if role == UserRole::Guest && input.teams.is_empty() {
            return Err(AppError::invalid_request("a guest needs at least one team")
                .with_details(json!({ "field": "teams" })));
        }
        let mut team_ids: Vec<TeamId> = Vec::new();
        for r in &input.teams {
            let team = resolve::team::resolve_in(&mut *tx, &snap, r, Visibility::Management).await?;
            let action = if role == UserRole::Guest {
                Action::AddGuest
            } else {
                Action::AddMember
            };
            snap.require_team(ctx, action, team.id)?;
            if !team_ids.contains(&team.id) {
                team_ids.push(team.id);
            }
        }
        let org = tx
            .orgs()
            .get()
            .await?
            .ok_or_else(|| AppError::internal("workspace vanished"))?;
        let inviter = resolve::user::actor_user_id(ctx)
            .ok_or_else(|| AppError::forbidden("Only users can invite."))?;
        let now = self.clock.now();
        let mut created = Vec::new();
        for email in &emails {
            if let Some(u) = tx.users().find_by_email(email).await?
                && u.status == "active"
            {
                return Err(AppError::conflict("A user with this email already exists")
                    .with_details(json!({ "field": "emails", "email": email })));
            }
            if let Some(old) = tx.webhooks().find_pending_invitation(email, now).await? {
                if old.expires_at > now {
                    return Err(AppError::conflict(
                        "There is already a pending invitation for this email; resend it instead",
                    )
                    .with_details(json!({ "field": "emails", "email": email })));
                }
                tx.webhooks()
                    .set_invitation_status(old.id, "expired", None, now)
                    .await?;
            }
            let token = random_token();
            let id = self.ids.new_uuid();
            tx.webhooks()
                .insert_invitation(&NewInvitation {
                    id,
                    email: email.clone(),
                    role,
                    team_ids: team_ids.clone(),
                    inviter_id: inviter,
                    token_hash: hash_token(&token),
                    expires_at: now + Duration::days(INVITATION_TTL_DAYS),
                    now,
                })
                .await?;
            let invitation = tx
                .webhooks()
                .get_invitation(id)
                .await?
                .ok_or_else(|| AppError::internal("invitation vanished after insert"))?;
            created.push(CreatedInvitation { invitation, token });
        }
        tx.commit().await?;
        for c in &created {
            // Mailfehler dürfen die Einladung nicht rückgängig machen; „Resend“ wiederholt den Versand.
            if let Err(e) = self
                .mailer
                .send(self.invitation_mail(&c.invitation.email, &org.name, &c.token))
                .await
            {
                tracing::warn!(error = %e, "invitation mail failed");
            }
        }
        Ok(created)
    }

    /// Offene und frühere Einladungen (Admin-Sicht „Members → Invited“).
    pub async fn list_invitations(
        &self,
        ctx: &RequestCtx,
        pending_only: bool,
    ) -> AppResult<Vec<InvitationRecord>> {
        forbid_system(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::InviteUsers)?;
        tx.webhooks().list_invitations(pending_only).await
    }

    /// Sendet die Einladung mit neuem Token und neuer Frist erneut (alter Link wird ungültig).
    pub async fn resend_invitation(
        &self,
        ctx: &RequestCtx,
        id: Uuid,
    ) -> AppResult<CreatedInvitation> {
        forbid_system(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::InviteUsers)?;
        let inv = tx
            .webhooks()
            .get_invitation(id)
            .await?
            .ok_or_else(invitation_not_found)?;
        if !matches!(inv.status.as_str(), "pending" | "expired") {
            return Err(AppError::conflict("The invitation can no longer be resent")
                .with_details(json!({ "status": inv.status })));
        }
        let now = self.clock.now();
        let token = random_token();
        tx.webhooks()
            .renew_invitation(
                id,
                &hash_token(&token),
                now + Duration::days(INVITATION_TTL_DAYS),
                now,
            )
            .await?;
        let invitation = tx
            .webhooks()
            .get_invitation(id)
            .await?
            .ok_or_else(invitation_not_found)?;
        let org = tx
            .orgs()
            .get()
            .await?
            .ok_or_else(|| AppError::internal("workspace vanished"))?;
        tx.commit().await?;
        if let Err(e) = self
            .mailer
            .send(self.invitation_mail(&invitation.email, &org.name, &token))
            .await
        {
            tracing::warn!(error = %e, "invitation mail failed");
        }
        Ok(CreatedInvitation { invitation, token })
    }

    /// Widerruft eine offene Einladung; der Link wird ungültig.
    pub async fn revoke_invitation(&self, ctx: &RequestCtx, id: Uuid) -> AppResult<InvitationRecord> {
        forbid_system(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::InviteUsers)?;
        let inv = tx
            .webhooks()
            .get_invitation(id)
            .await?
            .ok_or_else(invitation_not_found)?;
        if inv.status != "pending" {
            return Err(AppError::conflict("Only pending invitations can be revoked")
                .with_details(json!({ "status": inv.status })));
        }
        tx.webhooks()
            .set_invitation_status(id, "revoked", None, self.clock.now())
            .await?;
        let updated = tx
            .webhooks()
            .get_invitation(id)
            .await?
            .ok_or_else(invitation_not_found)?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Nimmt eine Einladung mit Token an (Einladungslink, kein Login nötig: das Token belegt die
    /// Kontrolle über die E-Mail-Adresse). Legt den User an bzw. verknüpft den importierten
    /// Platzhalter gleicher E-Mail, trägt die Team-Mitgliedschaften ein und liefert bei gesetztem
    /// Passwort eine Session. Unbekannte, abgelaufene, widerrufene oder angenommene Tokens → `not_found`.
    pub async fn accept_invitation(
        &self,
        input: AcceptInvitationInput,
    ) -> AppResult<AcceptedInvitation> {
        if let Some(p) = &input.password {
            validate_password(p)?;
        }
        if input.password.is_none() {
            return Err(AppError::invalid_request("password is required")
                .with_details(json!({ "field": "password" })));
        }
        let password_hash = match &input.password {
            Some(p) => Some(self.hasher.hash(p)?),
            None => None,
        };
        let (accepted, email) = self
            .accept_invitation_core(&input.token, input.name.as_deref(), password_hash, None)
            .await?;
        // Session für den neuen Zugang.
        let login = match &input.password {
            Some(p) => Some(
                self.login(LoginInput {
                    email,
                    password: p.clone(),
                    workspace: Some(accepted.url_key.clone()),
                    user_agent: None,
                    ip_address: None,
                })
                .await?,
            ),
            None => None,
        };
        Ok(AcceptedInvitation { login, ..accepted })
    }

    /// Nimmt eine Einladung mit der Konto-Session eines (anderen) Workspace an: die E-Mail des
    /// Session-Users muss zur Einladung passen (sonst `not_found`); das Passwort wird übernommen.
    pub async fn accept_invitation_as_account(
        &self,
        ctx: &RequestCtx,
        token: &str,
    ) -> AppResult<AcceptedInvitation> {
        let me = resolve::user::actor_user_id(ctx)
            .filter(|_| matches!(ctx.actor, Actor::User { .. }))
            .ok_or_else(invitation_not_found)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let account = tx
            .users()
            .get(me)
            .await?
            .filter(|u| u.active && u.status == "active")
            .ok_or_else(invitation_not_found)?;
        drop(tx);
        let mut sys = self.store.begin_system().await?;
        let hash = sys.webhooks().password_hash_of_account(me).await?;
        drop(sys);
        let accepted = self
            .accept_invitation_core(token, Some(&account.name), hash, Some(&account.email))
            .await?;
        Ok(accepted.0)
    }

    /// Gemeinsamer Kern; `required_email` bindet die Annahme an eine E-Mail (Konto-Session).
    /// Liefert zusätzlich die E-Mail der Einladung.
    async fn accept_invitation_core(
        &self,
        token: &str,
        name: Option<&str>,
        password_hash: Option<String>,
        required_email: Option<&str>,
    ) -> AppResult<(AcceptedInvitation, String)> {
        if token.is_empty() || token.len() > 512 {
            return Err(invitation_not_found());
        }
        let hash = hash_token(token);
        let mut sys = self.store.begin_system().await?;
        let inv = sys
            .webhooks()
            .find_invitation_by_token_hash(&hash)
            .await?
            .ok_or_else(invitation_not_found)?;
        drop(sys);
        let now = self.clock.now();
        if inv.status != "pending" || inv.expires_at <= now {
            return Err(invitation_not_found());
        }
        if let Some(req) = required_email
            && !req.eq_ignore_ascii_case(&inv.email)
        {
            return Err(invitation_not_found());
        }
        if let Some(n) = name {
            let n = n.trim();
            if n.is_empty() || n.chars().count() > 255 {
                return Err(AppError::invalid_request("name must be 1-255 characters")
                    .with_details(json!({ "field": "name" })));
            }
        }
        let mut tx = self.store.begin(inv.org_id).await?;
        // Erneut im Workspace prüfen (Wettlauf mit widerrufen/angenommen).
        let inv = tx
            .webhooks()
            .get_invitation(inv.id)
            .await?
            .filter(|i| i.status == "pending" && i.expires_at > now)
            .ok_or_else(invitation_not_found)?;
        let org = tx
            .orgs()
            .get()
            .await?
            .ok_or_else(invitation_not_found)?;
        let teams = tx.teams().list_all().await?;
        let tree = crate::policy::TeamTree::new(teams);
        let existing = tx.users().find_by_email(&inv.email).await?;
        let user_id = match existing {
            Some(u) if u.status == "imported" => {
                tx.webhooks()
                    .activate_imported_user(
                        u.id,
                        name.map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&u.name),
                        password_hash.as_deref(),
                        inv.role,
                        now,
                    )
                    .await?;
                u.id
            }
            Some(_) => {
                return Err(AppError::conflict("A user with this email already exists"));
            }
            None => {
                let id = UserId(self.ids.new_uuid());
                let display_name = display_name_from_email(&mut *tx, &inv.email).await?;
                let default_name = inv.email.split('@').next().unwrap_or("user").to_string();
                tx.users()
                    .insert(&NewUser {
                        id,
                        name: name
                            .map(str::trim)
                            .filter(|n| !n.is_empty())
                            .map_or(default_name, str::to_string),
                        display_name,
                        email: inv.email.clone(),
                        role: inv.role,
                        password_hash,
                        now,
                    })
                    .await?;
                id
            }
        };
        let ev_ctx = RequestCtx {
            org_id: inv.org_id,
            actor: Actor::User {
                id: user_id,
                role: inv.role,
            },
            request_id: "req_accept_invitation".into(),
            scopes: crate::context::Scopes::read_write(),
        };
        emit(&mut *tx, &ev_ctx, DomainEvent::UserCreated { user_id }).await?;
        for t in &inv.team_ids {
            let Some(team) = tree.get(*t).filter(|x| !x.trashed && x.retired_at.is_none()) else {
                continue;
            };
            let chain: Vec<TeamId> = if inv.role == UserRole::Guest {
                vec![team.id]
            } else {
                std::iter::once(team.id)
                    .chain(tree.ancestors(team.id).iter().map(|a| a.id))
                    .collect()
            };
            for tid in chain {
                if tx.teams().get_membership(tid, user_id).await?.is_none() {
                    tx.teams()
                        .insert_membership(&NewMembership {
                            id: self.ids.new_uuid(),
                            team_id: tid,
                            user_id,
                            owner: false,
                            now,
                        })
                        .await?;
                    emit(
                        &mut *tx,
                        &ev_ctx,
                        DomainEvent::TeamMembershipCreated {
                            team_id: tid,
                            user_id,
                        },
                    )
                    .await?;
                }
            }
        }
        tx.webhooks()
            .set_invitation_status(inv.id, "accepted", Some(user_id), now)
            .await?;
        tx.commit().await?;
        Ok((
            AcceptedInvitation {
                org_id: inv.org_id,
                url_key: org.url_key,
                user_id,
                login: None,
            },
            inv.email,
        ))
    }
}
