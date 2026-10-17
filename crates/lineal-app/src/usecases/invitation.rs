//! usecases – Einladungen (WP16, docs/01 §2.11, docs/08 F1/F16): einladen, erneut senden,
//! widerrufen, annehmen.
//!
//! Das Token (96 Hex-Zeichen) steht nur im Link der E-Mail (`{base}/invite/{token}`); gespeichert
//! wird `hash_token` = SHA-256. Eine Einladung gilt 7 Tage. Die Annahme verknüpft einen importierten
//! Platzhalter-User gleicher E-Mail, statt einen zweiten User anzulegen.

use super::audit::AuditEvent;
use super::audit::user_ctx;
use super::auth::{LoginInput, hash_token};
use super::team::emit;
use super::user::{display_name_from_email, validate_email, validate_password};
use crate::app::{App, WorkspaceCreation};
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
    /// E-Mail-Adresse, an die die Einladung ging (Pflicht, case-insensitiv geprüft).
    pub email: String,
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

fn require_creator_or_admin(ctx: &RequestCtx, inv: &InvitationRecord) -> AppResult<()> {
    // Die Admin-Abkürzung gilt nur mit Scope `admin` (Sessions tragen ihn).
    let admin = ctx.scopes.admin
        && matches!(
            ctx.actor,
            Actor::User { role, .. } if matches!(role, UserRole::Owner | UserRole::Admin)
        );
    if admin || resolve::user::actor_user_id(ctx) == Some(inv.inviter_id) {
        Ok(())
    } else {
        Err(invitation_not_found())
    }
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

/// Prüft zur Ausführungszeit (Annahme/Resend), ob der ursprüngliche Einladende die Einladung heute
/// noch so aussprechen dürfte: aktiv, Recht `inviteUsers`, bei `admin` zusätzlich `grantAdminRole`
/// und für jedes (noch vorhandene) Team `addMember`/`addGuest`. Liefert den Ablehnungsgrund.
///
/// Die Prüfung am Annahme-Pfad ist robuster als das Widerrufen bei Herabstufung/Teamentfernung:
/// sie deckt jeden Weg des Rechteverlusts ab (Rolle, Team-Owner/-Mitgliedschaft, Teamsicht,
/// Workspace-Einstellungen), ohne dass jeder Schreibpfad daran denken muss.
async fn inviter_rejection(
    tx: &mut dyn crate::ports::Tx,
    inv: &InvitationRecord,
) -> AppResult<Option<&'static str>> {
    let Some(user) = tx
        .users()
        .get(inv.inviter_id)
        .await?
        .filter(|u| u.active && u.status == "active")
    else {
        return Ok(Some("inviter_inactive"));
    };
    let ctx = RequestCtx {
        org_id: inv.org_id,
        actor: Actor::User {
            id: user.id,
            role: user.role,
        },
        request_id: "req_invitation_recheck".into(),
        scopes: crate::context::Scopes::all(),
        team_scope: None,
        via_session: false,
        client_ip: None,
        credential_kind: None,
    };
    let snap = PolicySnapshot::load(tx, &ctx).await?;
    if snap.require_workspace(&ctx, Action::InviteUsers).is_err() {
        return Ok(Some("inviter_lost_invite_right"));
    }
    if inv.role == UserRole::Admin
        && snap
            .require_workspace(&ctx, Action::GrantAdminRole)
            .is_err()
    {
        return Ok(Some("inviter_lost_admin_right"));
    }
    if inv.role == UserRole::Guest && !snap.org.limits.guests_enabled {
        return Ok(Some("guests_disabled"));
    }
    let action = if inv.role == UserRole::Guest {
        Action::AddGuest
    } else {
        Action::AddMember
    };
    for t in &inv.team_ids {
        // Gelöschte/stillgelegte Teams überspringt die Annahme ohnehin.
        if snap
            .tree
            .get(*t)
            .is_none_or(|x| x.trashed || x.retired_at.is_some())
        {
            continue;
        }
        if snap.require_team(&ctx, action, *t).is_err() {
            return Ok(Some("inviter_lost_team_right"));
        }
    }
    Ok(None)
}

impl App {
    /// Widerruft eine Einladung, deren Einladender die Rechte verloren hat (Systemkontext, Audit).
    async fn revoke_invitation_for_reason(
        &self,
        mut tx: Box<dyn crate::ports::Tx>,
        inv: &InvitationRecord,
        reason: &str,
    ) -> AppResult<()> {
        let now = self.clock.now();
        tx.webhooks()
            .set_invitation_status(inv.id, "revoked", None, now)
            .await?;
        let sys = RequestCtx::system(inv.org_id, "req_invitation_recheck");
        self.record_audit(
            &mut *tx,
            &sys,
            AuditEvent::new(
                "invitation.revoked",
                "invitation",
                Some(inv.id),
                json!({ "email": inv.email, "reason": reason, "automatic": true }),
            ),
        )
        .await?;
        tx.commit().await
    }

    fn invitation_mail(&self, to: &str, org_name: &str, token: &str) -> Mail {
        let link = format!(
            "{}/invite/{token}",
            self.config.base_url.trim_end_matches('/')
        );
        Mail {
            to: to.to_string(),
            subject: lineal_domain::rules::text::sanitize_header_value(&format!(
                "You have been invited to {org_name} on Lineal"
            )),
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
        for e in &emails {
            super::user::check_invite_domain(&snap.org, e, "emails")?;
        }
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
            let team =
                resolve::team::resolve_in(&mut *tx, &snap, r, Visibility::Management).await?;
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
            self.record_audit(
                &mut *tx,
                ctx,
                AuditEvent::new(
                    "invitation.created",
                    "invitation",
                    Some(id),
                    json!({ "email": invitation.email, "role": role.as_str() }),
                ),
            )
            .await?;
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
        let mut all = tx.webhooks().list_invitations(pending_only).await?;
        // Nur Admins/Owner sehen die Einladungen anderer (konsistent mit Resend/Revoke; B8.07).
        if !snap.actor.is_admin() {
            let me = resolve::user::actor_user_id(ctx);
            all.retain(|i| Some(i.inviter_id) == me);
        }
        Ok(all)
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
        require_creator_or_admin(ctx, &inv)?;
        if !matches!(inv.status.as_str(), "pending" | "expired") {
            return Err(AppError::conflict("The invitation can no longer be resent")
                .with_details(json!({ "status": inv.status })));
        }
        super::user::check_invite_domain(&snap.org, &inv.email, "id")?;
        if let Some(reason) = inviter_rejection(&mut *tx, &inv).await? {
            self.revoke_invitation_for_reason(tx, &inv, reason).await?;
            return Err(invitation_not_found());
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
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new(
                "invitation.resent",
                "invitation",
                Some(id),
                json!({ "email": invitation.email }),
            ),
        )
        .await?;
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
    pub async fn revoke_invitation(
        &self,
        ctx: &RequestCtx,
        id: Uuid,
    ) -> AppResult<InvitationRecord> {
        forbid_system(ctx)?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::InviteUsers)?;
        let inv = tx
            .webhooks()
            .get_invitation(id)
            .await?
            .ok_or_else(invitation_not_found)?;
        require_creator_or_admin(ctx, &inv)?;
        if inv.status != "pending" {
            return Err(
                AppError::conflict("Only pending invitations can be revoked")
                    .with_details(json!({ "status": inv.status })),
            );
        }
        tx.webhooks()
            .set_invitation_status(id, "revoked", None, self.clock.now())
            .await?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new(
                "invitation.revoked",
                "invitation",
                Some(id),
                json!({ "email": inv.email }),
            ),
        )
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
            .accept_invitation_core(
                &input.token,
                input.name.as_deref(),
                password_hash,
                Some(&input.email),
            )
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
                    request_id: None,
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
        let name_norm = name
            .map(|n| lineal_domain::rules::text::validate_single_line_name("name", n, 255))
            .transpose()?;
        let name = name_norm.as_deref();
        let mut tx = self.store.begin(inv.org_id).await?;
        // Erneut im Workspace prüfen (Wettlauf mit widerrufen/angenommen).
        let inv = tx
            .webhooks()
            .get_invitation(inv.id)
            .await?
            .filter(|i| i.status == "pending" && i.expires_at > now)
            .ok_or_else(invitation_not_found)?;
        let org = tx.orgs().get().await?.ok_or_else(invitation_not_found)?;
        // Aktuelle Domain-Beschränkung gilt auch für früher erstellte Einladungen und für das Konto,
        // das sie annimmt (neutraler Fehler wie bei ungültigem Token).
        let policy = crate::policy::OrgPolicy::from_json(&org.security_settings, &org.limits);
        if !policy.invite_domain_allowed(&inv.email)
            || required_email.is_some_and(|e| !policy.invite_domain_allowed(e))
        {
            return Err(invitation_not_found());
        }
        // Rechte werden zur Annahmezeit erneut geprüft (F-06): hat der Einladende die Rolle bzw.
        // Team-Rechte verloren oder ist er nicht mehr aktiv, wird die Einladung widerrufen
        // (ganz ablehnen statt herabstufen – ein stilles Teilergebnis wäre nicht nachvollziehbar).
        if let Some(reason) = inviter_rejection(&mut *tx, &inv).await? {
            self.revoke_invitation_for_reason(tx, &inv, reason).await?;
            return Err(invitation_not_found());
        }
        let teams = tx.teams().list_all().await?;
        let tree = crate::policy::TeamTree::new(teams);
        let existing = tx.users().find_by_email(&inv.email).await?;
        let user_id = match existing {
            Some(u) if u.status == "imported" => {
                tx.webhooks()
                    .activate_imported_user(
                        u.id,
                        name.map(str::trim)
                            .filter(|n| !n.is_empty())
                            .unwrap_or(&u.name),
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
            team_scope: None,

            via_session: false,
            client_ip: None,
            credential_kind: None,
        };
        emit(&mut *tx, &ev_ctx, DomainEvent::UserCreated { user_id }).await?;
        for t in &inv.team_ids {
            let Some(team) = tree
                .get(*t)
                .filter(|x| !x.trashed && x.retired_at.is_none())
            else {
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
        // System-Benachrichtigung "Einladung angenommen" an den Einladenden (F.17, docs/05 §6.2).
        super::notification::fanout::insert_direct(
            &mut *tx,
            self.ids.as_ref(),
            now,
            inv.inviter_id,
            "invitationAccepted",
            Some(user_id),
            None,
            format!("invitation-accepted:{}", inv.id),
        )
        .await?;
        let accept_ctx = user_ctx(
            inv.org_id,
            user_id,
            UserRole::User,
            None,
            String::new(),
            None,
        );
        self.record_audit(
            &mut *tx,
            &accept_ctx,
            AuditEvent::new(
                "invitation.accepted",
                "invitation",
                Some(inv.id),
                json!({ "email": inv.email }),
            ),
        )
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

impl App {
    /// `createWorkspace` mit einer Konto-Session (docs/09 §1.6): ein angemeldeter User legt einen
    /// weiteren Workspace an und wird dort Owner; Name, E-Mail und Passwort-Hash werden aus dem
    /// Konto übernommen. Höchstens 10 Workspaces je E-Mail-Adresse.
    pub async fn create_workspace_for_account(
        &self,
        ctx: &RequestCtx,
        name: &str,
        url_key: Option<&str>,
    ) -> AppResult<super::org::WorkspaceCreated> {
        forbid_system(ctx)?;
        if !ctx.scopes.write {
            return Err(AppError::forbidden(
                "Token scope does not allow write operations.",
            ));
        }
        let me = resolve::user::actor_user_id(ctx)
            .filter(|_| matches!(ctx.actor, Actor::User { .. }))
            .ok_or_else(|| AppError::forbidden("Only users can create workspaces."))?;
        let mut tx = self.store.begin(ctx.org_id).await?;
        let account = tx
            .users()
            .get(me)
            .await?
            .filter(|u| u.active && u.status == "active")
            .ok_or_else(|| AppError::forbidden("Only active users can create workspaces."))?;
        drop(tx);
        let allowed = match self.config.workspace_creation {
            WorkspaceCreation::All => true,
            WorkspaceCreation::Disabled => false,
            WorkspaceCreation::Admins => {
                // Nur unveränderliche User-IDs zählen (E-Mail und url_key lassen sich ändern
                // bzw. in fremden Workspaces anlegen): aktives Konto mit Rolle Owner/Admin.
                matches!(account.role, UserRole::Owner | UserRole::Admin)
                    && self.config.instance_admins.contains(&me.0)
            }
        };
        if !allowed {
            return Err(AppError::forbidden(
                "Workspace creation is disabled on this instance.",
            ));
        }
        let mut sys = self.store.begin_system().await?;
        let owned = sys
            .auth()
            .find_login_candidates(&account.email, None)
            .await?
            .len();
        let hash = sys.webhooks().password_hash_of_account(me).await?;
        drop(sys);
        if owned >= 10 {
            return Err(
                AppError::limit_exceeded("Too many workspaces for this account.")
                    .with_details(json!({ "limit": "maxWorkspacesPerAccount" })),
            );
        }
        let created = self
            .create_workspace(super::org::CreateWorkspaceInput {
                name: name.to_string(),
                url_key: url_key.map(str::to_string),
                owner_name: account.name.clone(),
                owner_email: account.email.clone(),
                owner_password: None,
                credential_kind: ctx.credential_kind,
            })
            .await?;
        if let Some(h) = hash {
            let mut tx = self.store.begin(created.org.id).await?;
            tx.users()
                .set_password_hash(created.owner.id, &h, self.clock.now())
                .await?;
            tx.commit().await?;
        }
        Ok(created)
    }
}
