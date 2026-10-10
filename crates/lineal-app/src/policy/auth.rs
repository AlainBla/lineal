//! policy – Aktionen und `require` mit der Berechtigungsmatrix aus docs/02 §6.4/§6.5.
//!
//! `require` ist rein (keine I/O): Es bekommt die vorab geladene [`TeamPolicyView`] bzw.
//! [`OrgPolicy`]. Die Reihenfolge der Entscheidung:
//! 1. nicht zugängliches Team → `not_found` (nie `forbidden`; Gast ohne Mitgliedschaft ebenso),
//! 2. Token-Scope (`write`/`admin`) → `forbidden`,
//! 3. stillgelegtes Team + schreibende Aktion → `forbidden` mit `details.reason = "team_retired"`,
//! 4. Rollen-/Team-Regel der Aktion → `forbidden`.
//!
//! Bei mehreren Teams hat `not_found` Vorrang vor `forbidden`.

use super::org::{OrgPolicy, role_rank};
use super::team::{TeamPolicyView, team_not_found};
use crate::context::{Actor, RequestCtx};
use crate::error::{AppError, AppResult, ErrorKind};
use lineal_domain::enums::UserRole;
use serde_json::json;

/// Aktionen der Berechtigungsmatrix (docs/02 §6.4). Andere Arbeitspakete benutzen die
/// Issue-/Projekt-Aktionen für ihre Use-Cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    // --- Workspace ---
    UpdateWorkspaceSettings,
    UpdateWorkspaceLimits,
    DeleteWorkspace,
    CancelWorkspaceDeletion,
    /// Admin-Rolle vergeben/entziehen (`adminManagementRole`).
    GrantAdminRole,
    InviteUsers,
    SuspendUser,
    CreateTeam,
    CreatePrivateTeam,
    ManageWorkspaceLabels,
    ManageWorkspaceTemplates,
    ManageWorkspaceProjectStatus,
    /// Webhooks und OAuth-Apps des Workspace (`apiSettingsRole`).
    ManageWebhooks,
    CreatePersonalApiKey,
    ViewInitiatives,
    // --- Team ---
    /// Team und Inhalte sehen (Zugriff).
    ViewTeam,
    /// Team in der Verwaltung sehen (Admins sehen auch private Teams als Metadaten).
    ViewTeamMetadata,
    JoinTeam,
    LeaveTeam,
    AddMember,
    AddGuest,
    RemoveMember,
    SetTeamOwner,
    /// Team-Einstellungen (`teamManagement`).
    UpdateTeamSettings,
    ManageTeamLabels,
    ManageTeamTemplates,
    ManageTeamViews,
    ChangeTeamKey,
    ChangeTeamVisibility,
    SetTeamParent,
    RetireTeam,
    RestoreTeam,
    DeleteTeam,
    RestoreDeletedTeam,
    CreateTeamWebhook,
    // --- Inhalte eines Teams (für andere Pakete) ---
    CreateIssue,
    UpdateIssue,
    DeleteIssue,
    CommentIssue,
    ShareIssue,
    /// Projekt anlegen/bearbeiten (für jedes beteiligte Team auszuwerten).
    ManageProject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    AnyAccess,
    Owner,
    Admin,
    /// Team-Recht (`member`|`owner`); Gäste und Apps ausgeschlossen.
    MemberSettingNoGuest(&'static str),
    /// Team-Recht (`member`|`owner`); Gäste (als Mitglied) und Apps erlaubt.
    MemberSetting(&'static str),
    CanJoin,
    IssueSharing,
}

impl Action {
    /// Ob die Aktion Workspace-Ebene betrifft (sonst Team-Ebene).
    pub const fn is_workspace(&self) -> bool {
        use Action::*;
        matches!(
            self,
            UpdateWorkspaceSettings
                | UpdateWorkspaceLimits
                | DeleteWorkspace
                | CancelWorkspaceDeletion
                | GrantAdminRole
                | InviteUsers
                | SuspendUser
                | CreateTeam
                | CreatePrivateTeam
                | ManageWorkspaceLabels
                | ManageWorkspaceTemplates
                | ManageWorkspaceProjectStatus
                | ManageWebhooks
                | CreatePersonalApiKey
                | ViewInitiatives
        )
    }

    /// Schreibende Aktion (braucht Scope `write`).
    pub const fn is_write(&self) -> bool {
        !matches!(
            self,
            Action::ViewTeam | Action::ViewTeamMetadata | Action::ViewInitiatives
        )
    }

    /// Workspace-Administration (braucht zusätzlich Scope `admin`).
    pub const fn needs_admin_scope(&self) -> bool {
        use Action::*;
        matches!(
            self,
            UpdateWorkspaceSettings
                | UpdateWorkspaceLimits
                | DeleteWorkspace
                | CancelWorkspaceDeletion
                | GrantAdminRole
                | SuspendUser
                | ManageWorkspaceProjectStatus
                | ManageWebhooks
        )
    }

    /// Regel und „nur Metadaten“-Flag (Admin-Verwaltung privater Teams).
    fn team_rule(&self) -> (Rule, bool) {
        use Action::*;
        match self {
            ViewTeam => (Rule::AnyAccess, false),
            ViewTeamMetadata => (Rule::AnyAccess, true),
            JoinTeam => (Rule::CanJoin, true),
            LeaveTeam => (Rule::AnyAccess, false),
            AddMember => (Rule::MemberSettingNoGuest("memberManagement"), true),
            AddGuest | RemoveMember | SetTeamOwner => (Rule::Owner, true),
            UpdateTeamSettings => (Rule::MemberSetting("teamManagement"), true),
            ManageTeamLabels => (Rule::MemberSetting("labelManagement"), false),
            ManageTeamTemplates => (Rule::MemberSetting("templateManagement"), false),
            ManageTeamViews => (Rule::MemberSetting("pinnedViewManagement"), false),
            ChangeTeamKey | ChangeTeamVisibility | RetireTeam | RestoreTeam | DeleteTeam
            | CreateTeamWebhook => (Rule::Owner, true),
            SetTeamParent | RestoreDeletedTeam => (Rule::Admin, true),
            CreateIssue | UpdateIssue | DeleteIssue | CommentIssue | ManageProject => {
                (Rule::AnyAccess, false)
            }
            ShareIssue => (Rule::IssueSharing, false),
            // Workspace-Aktionen werden nie über Team-Regeln ausgewertet.
            _ => (Rule::Admin, true),
        }
    }
}

/// Worauf eine Aktion angewendet wird.
#[derive(Debug, Clone, Copy)]
pub enum Resource<'a> {
    Workspace(&'a OrgPolicy),
    Team(&'a TeamPolicyView),
    /// Mehrere Teams (Teamwechsel, Projekte); `not_found` hat Vorrang vor `forbidden`.
    Teams(&'a [TeamPolicyView]),
}

fn actor_role(ctx: &RequestCtx) -> Option<UserRole> {
    match ctx.actor {
        Actor::User { role, .. } => Some(role),
        Actor::App { .. } => Some(UserRole::App),
        Actor::System => None,
    }
}

fn forbidden(action: Action, reason: &str) -> AppError {
    AppError::forbidden(format!("Not allowed to perform {action:?}."))
        .with_details(json!({ "action": format!("{action:?}"), "reason": reason }))
}

fn check_scope(ctx: &RequestCtx, action: Action) -> AppResult<()> {
    if matches!(ctx.actor, Actor::System) {
        return Ok(());
    }
    if action.is_write() && !ctx.scopes.write {
        return Err(AppError::forbidden("Token scope does not allow write operations.")
            .with_details(json!({ "action": format!("{action:?}"), "reason": "insufficient_scope", "scope": "write" })));
    }
    if action.needs_admin_scope() && !ctx.scopes.admin {
        return Err(AppError::forbidden("Token scope does not allow admin operations.")
            .with_details(json!({ "action": format!("{action:?}"), "reason": "insufficient_scope", "scope": "admin" })));
    }
    if !ctx.scopes.read {
        return Err(AppError::forbidden("Token scope does not allow read operations.")
            .with_details(json!({ "action": format!("{action:?}"), "reason": "insufficient_scope", "scope": "read" })));
    }
    Ok(())
}

fn is_admin(role: UserRole) -> bool {
    matches!(role, UserRole::Owner | UserRole::Admin)
}

fn require_workspace(ctx: &RequestCtx, action: Action, org: &OrgPolicy) -> AppResult<()> {
    let Some(role) = actor_role(ctx) else {
        return Ok(()); // System
    };
    if !action.is_workspace() {
        return Err(AppError::internal(format!(
            "{action:?} is a team action and needs a team resource"
        )));
    }
    check_scope(ctx, action)?;
    let at_least = |min: UserRole| role_rank(role) >= role_rank(min);
    let guest_or_app = matches!(role, UserRole::Guest | UserRole::App);
    use Action::*;
    let allowed = match action {
        UpdateWorkspaceSettings
        | SuspendUser
        | ManageWorkspaceProjectStatus
        | CancelWorkspaceDeletion => is_admin(role),
        UpdateWorkspaceLimits | DeleteWorkspace => role == UserRole::Owner,
        GrantAdminRole => !guest_or_app && at_least(org.admin_management_role),
        InviteUsers => !guest_or_app && at_least(org.invitations_role),
        CreateTeam => role != UserRole::Guest && at_least(org.team_creation_role),
        CreatePrivateTeam => {
            let ok = !guest_or_app && at_least(org.team_creation_role);
            if ok && !org.limits.private_teams_enabled {
                return Err(AppError::limit_exceeded("Private teams are disabled.")
                    .with_details(json!({ "limit": "privateTeamsEnabled" })));
            }
            ok
        }
        ManageWorkspaceLabels => role != UserRole::Guest && at_least(org.label_management_role),
        ManageWorkspaceTemplates => !guest_or_app && at_least(org.template_management_role),
        ManageWebhooks => !guest_or_app && at_least(org.api_settings_role),
        CreatePersonalApiKey => !guest_or_app && at_least(org.personal_api_keys_role),
        ViewInitiatives => role != UserRole::Guest,
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(forbidden(action, "role"))
    }
}

fn setting_is_member(view: &TeamPolicyView, key: &str) -> bool {
    let default = if key == "issueSharing" {
        "owner"
    } else {
        "member"
    };
    view.team
        .security_settings
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or(default)
        == "member"
}

fn require_one_team(ctx: &RequestCtx, action: Action, view: &TeamPolicyView) -> AppResult<()> {
    let Some(role) = actor_role(ctx) else {
        return Ok(()); // System
    };
    if action.is_workspace() {
        return Err(AppError::internal(format!(
            "{action:?} is a workspace action and needs a workspace resource"
        )));
    }
    if action == Action::RestoreDeletedTeam {
        // Gelöschte Teams sind für Nicht-Admins unsichtbar.
        if !is_admin(role) {
            return Err(team_not_found());
        }
        check_scope(ctx, action)?;
        return Ok(());
    }
    let (rule, metadata_only) = action.team_rule();
    let visible = if metadata_only {
        view.can_see_metadata
    } else {
        view.can_access
    };
    if !visible || (role == UserRole::Guest && !view.is_member) {
        return Err(team_not_found());
    }
    check_scope(ctx, action)?;
    if view.team.retired_at.is_some()
        && action.is_write()
        && !matches!(action, Action::RestoreTeam | Action::DeleteTeam)
    {
        return Err(AppError::forbidden("The team is retired and read-only.")
            .with_details(json!({ "action": format!("{action:?}"), "reason": "team_retired" })));
    }
    let ok = match rule {
        Rule::AnyAccess => true,
        Rule::Owner => view.is_team_owner,
        Rule::Admin => is_admin(role),
        Rule::MemberSettingNoGuest(key) => {
            !matches!(role, UserRole::Guest | UserRole::App)
                && (view.is_team_owner || (view.is_member && setting_is_member(view, key)))
        }
        Rule::MemberSetting(key) => {
            view.is_team_owner || (view.is_member && setting_is_member(view, key))
        }
        Rule::CanJoin => view.can_join,
        Rule::IssueSharing => {
            if matches!(role, UserRole::Guest | UserRole::App) {
                false
            } else if !view.org.limits.issue_sharing_enabled {
                return Err(AppError::limit_exceeded("Issue sharing is disabled.")
                    .with_details(json!({ "limit": "issueSharingEnabled" })));
            } else {
                view.is_member && (view.is_team_owner || setting_is_member(view, "issueSharing"))
            }
        }
    };
    if ok {
        Ok(())
    } else {
        Err(forbidden(action, "role"))
    }
}

/// Prüft, ob `ctx` die Aktion auf der Ressource ausführen darf (docs/02 §6.5).
pub fn require(ctx: &RequestCtx, action: Action, resource: Resource<'_>) -> AppResult<()> {
    match resource {
        Resource::Workspace(org) => require_workspace(ctx, action, org),
        Resource::Team(view) => require_one_team(ctx, action, view),
        Resource::Teams(views) => {
            let mut first_forbidden: Option<AppError> = None;
            for view in views {
                match require_one_team(ctx, action, view) {
                    Ok(()) => {}
                    Err(e) if e.kind == ErrorKind::NotFound => return Err(e),
                    Err(e) => {
                        first_forbidden.get_or_insert(e);
                    }
                }
            }
            first_forbidden.map_or(Ok(()), Err)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::team::fixtures::*;
    use super::super::team::{ActorView, PolicySnapshot, TeamTree, compute_policy_view};
    use super::*;
    use crate::context::Scopes;
    use chrono::Utc;
    use lineal_domain::enums::TeamVisibility;

    fn snap(actor: ActorView) -> PolicySnapshot {
        PolicySnapshot {
            actor,
            tree: tree(),
            org: OrgPolicy::default(),
        }
    }

    fn kind<T>(r: AppResult<T>) -> Option<ErrorKind> {
        r.err().map(|e| e.kind)
    }

    #[test]
    fn private_team_is_not_found_for_non_members_even_for_reads() {
        let alice = alice_view();
        let ctx = ctx_of(&alice);
        let s = snap(alice);
        assert_eq!(
            kind(s.require_team(&ctx, Action::ViewTeam, des())),
            Some(ErrorKind::NotFound)
        );
        assert_eq!(
            kind(s.require_team(&ctx, Action::CreateIssue, des())),
            Some(ErrorKind::NotFound)
        );
        // Admin-Verwaltung (Metadaten) bleibt möglich.
        assert!(
            s.require_team(&ctx, Action::ViewTeamMetadata, des())
                .is_ok()
        );
        assert!(
            s.require_team(&ctx, Action::UpdateTeamSettings, des())
                .is_ok()
        );
        assert!(s.require_team(&ctx, Action::AddMember, des()).is_ok());
    }

    #[test]
    fn guest_only_sees_member_teams() {
        let carol = carol_view();
        let ctx = ctx_of(&carol);
        let s = snap(carol);
        assert!(s.require_team(&ctx, Action::CreateIssue, web()).is_ok());
        for t in [eng(), des()] {
            assert_eq!(
                kind(s.require_team(&ctx, Action::ViewTeam, t)),
                Some(ErrorKind::NotFound),
                "{t}"
            );
        }
        // Matrix: Gast darf Team-Einstellungen (T:teamManagement, Mitglied), aber keine Mitglieder hinzufügen.
        assert!(
            s.require_team(&ctx, Action::UpdateTeamSettings, web())
                .is_ok()
        );
        assert_eq!(
            kind(s.require_team(&ctx, Action::AddMember, web())),
            Some(ErrorKind::Forbidden)
        );
        assert_eq!(
            kind(s.require_team(&ctx, Action::JoinTeam, eng())),
            Some(ErrorKind::NotFound)
        );
    }

    #[test]
    fn team_settings_respect_team_security_and_owner_derivation() {
        let bob = bob_view();
        let ctx = ctx_of(&bob);
        let mut s = snap(bob);
        assert!(
            s.require_team(&ctx, Action::UpdateTeamSettings, eng())
                .is_ok()
        );
        // ENG verlangt Owner für teamManagement: Bob (nur Mitglied in ENG) darf nicht, Alice schon.
        let mut teams: Vec<_> = s.tree.all().cloned().collect();
        teams[0].security_settings = json!({ "teamManagement": "owner" });
        s.tree = TeamTree::new(teams);
        assert_eq!(
            kind(s.require_team(&ctx, Action::UpdateTeamSettings, eng())),
            Some(ErrorKind::Forbidden)
        );
        let alice_ctx = ctx_of(&alice_view());
        let mut sa = snap(alice_view());
        sa.tree = s.tree.clone();
        assert!(
            sa.require_team(&alice_ctx, Action::UpdateTeamSettings, eng())
                .is_ok()
        );
        // Bob ist Owner von WEB: Key ändern erlaubt; in ENG nicht.
        assert!(s.require_team(&ctx, Action::ChangeTeamKey, web()).is_ok());
        assert_eq!(
            kind(s.require_team(&ctx, Action::ChangeTeamKey, eng())),
            Some(ErrorKind::Forbidden)
        );
        // Alice (Owner von ENG) ist auch Owner des Sub-Teams WEB.
        assert!(
            sa.require_team(&alice_ctx, Action::ChangeTeamKey, web())
                .is_ok()
        );
    }

    #[test]
    fn nesting_is_admin_only_and_guests_are_added_by_owners_only() {
        let bob = bob_view();
        let ctx = ctx_of(&bob);
        let s = snap(bob);
        assert_eq!(
            kind(s.require_team(&ctx, Action::SetTeamParent, web())),
            Some(ErrorKind::Forbidden)
        );
        // Bob (Owner WEB) darf Gäste hinzufügen, in ENG (nur Mitglied) nicht.
        assert!(s.require_team(&ctx, Action::AddGuest, web()).is_ok());
        assert_eq!(
            kind(s.require_team(&ctx, Action::AddGuest, eng())),
            Some(ErrorKind::Forbidden)
        );
        let alice = alice_view();
        let actx = ctx_of(&alice);
        assert!(
            snap(alice)
                .require_team(&actx, Action::SetTeamParent, web())
                .is_ok()
        );
    }

    #[test]
    fn retired_teams_are_read_only() {
        let bob = bob_view();
        let ctx = ctx_of(&bob);
        let mut s = snap(bob);
        let mut teams: Vec<_> = s.tree.all().cloned().collect();
        teams[1].retired_at = Some(Utc::now());
        s.tree = TeamTree::new(teams);
        let err = s
            .require_team(&ctx, Action::CreateIssue, web())
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Forbidden);
        assert_eq!(err.details["reason"], "team_retired");
        assert!(s.require_team(&ctx, Action::ViewTeam, web()).is_ok());
        // Wiederherstellen und Löschen bleiben möglich (Bob ist Owner von WEB).
        assert!(s.require_team(&ctx, Action::RestoreTeam, web()).is_ok());
        assert!(s.require_team(&ctx, Action::DeleteTeam, web()).is_ok());
    }

    #[test]
    fn scopes_gate_writes_and_admin_actions() {
        let bob = bob_view();
        let mut ctx = ctx_of(&bob);
        ctx.scopes = Scopes::read_only();
        let s = snap(bob);
        assert_eq!(
            kind(s.require_team(&ctx, Action::CreateIssue, web())),
            Some(ErrorKind::Forbidden)
        );
        assert!(s.require_team(&ctx, Action::ViewTeam, web()).is_ok());
        // Nicht zugängliche Teams bleiben not_found, auch mit unzureichendem Scope.
        let carol = carol_view();
        let mut cctx = ctx_of(&carol);
        cctx.scopes = Scopes::read_only();
        assert_eq!(
            kind(snap(carol).require_team(&cctx, Action::CreateIssue, des())),
            Some(ErrorKind::NotFound)
        );
        // Workspace-Administration braucht den admin-Scope.
        let alice = alice_view();
        let mut actx = ctx_of(&alice);
        actx.scopes = Scopes::read_write();
        assert_eq!(
            kind(snap(alice).require_workspace(&actx, Action::UpdateWorkspaceSettings)),
            Some(ErrorKind::Forbidden)
        );
    }

    #[test]
    fn multiple_teams_not_found_beats_forbidden() {
        let bob = bob_view();
        let ctx = ctx_of(&bob);
        let s = snap(bob);
        let teams = [web(), eng()];
        let views: Vec<_> = teams
            .iter()
            .filter_map(|t| compute_policy_view(&s.actor, &s.tree, &s.org, *t))
            .collect();
        assert!(require(&ctx, Action::CreateIssue, Resource::Teams(&views)).is_ok());
        // Carol: WEB ok, DES unsichtbar -> not_found.
        let carol = carol_view();
        let cctx = ctx_of(&carol);
        let sc = snap(carol);
        let err = sc
            .require_teams(&cctx, Action::CreateIssue, &[web(), des()])
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::NotFound);
    }

    #[test]
    fn workspace_matrix() {
        let check = |view: ActorView, action: Action| {
            let ctx = ctx_of(&view);
            snap(view).require_workspace(&ctx, action)
        };
        assert!(check(alice_view(), Action::UpdateWorkspaceLimits).is_ok());
        assert!(check(alice_view(), Action::DeleteWorkspace).is_ok());
        assert!(check(bob_view(), Action::CreateTeam).is_ok());
        assert!(check(bob_view(), Action::CreatePrivateTeam).is_ok());
        assert!(check(bob_view(), Action::InviteUsers).is_ok());
        assert_eq!(
            kind(check(bob_view(), Action::UpdateWorkspaceSettings)),
            Some(ErrorKind::Forbidden)
        );
        assert_eq!(
            kind(check(bob_view(), Action::ManageWebhooks)),
            Some(ErrorKind::Forbidden)
        );
        assert_eq!(
            kind(check(bob_view(), Action::GrantAdminRole)),
            Some(ErrorKind::Forbidden)
        );
        for a in [
            Action::CreateTeam,
            Action::InviteUsers,
            Action::ViewInitiatives,
            Action::ManageWorkspaceLabels,
        ] {
            assert_eq!(
                kind(check(carol_view(), a)),
                Some(ErrorKind::Forbidden),
                "{a:?}"
            );
        }
        // Apps: Teams anlegen ja (public), privat nein, Webhooks nein.
        let app = ActorView::new(Actor::App { id: bob() }, &[]);
        assert!(check(app.clone(), Action::CreateTeam).is_ok());
        assert_eq!(
            kind(check(app.clone(), Action::CreatePrivateTeam)),
            Some(ErrorKind::Forbidden)
        );
        assert_eq!(
            kind(check(app, Action::ManageWebhooks)),
            Some(ErrorKind::Forbidden)
        );
    }

    #[test]
    fn private_teams_limit_is_a_limit_error() {
        let bob = bob_view();
        let ctx = ctx_of(&bob);
        let mut s = snap(bob);
        s.org.limits.private_teams_enabled = false;
        let err = s
            .require_workspace(&ctx, Action::CreatePrivateTeam)
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::LimitExceeded);
        assert_eq!(err.details["limit"], "privateTeamsEnabled");
    }

    #[test]
    fn issue_sharing_needs_membership_setting_and_limit() {
        let bob = bob_view();
        let ctx = ctx_of(&bob);
        let mut s = snap(bob);
        assert_eq!(
            kind(s.require_team(&ctx, Action::ShareIssue, web())),
            Some(ErrorKind::LimitExceeded)
        );
        s.org.limits.issue_sharing_enabled = true;
        // Bob ist Owner von WEB -> ok; in ENG nur Mitglied und Default `issueSharing = owner` -> forbidden.
        assert!(s.require_team(&ctx, Action::ShareIssue, web()).is_ok());
        assert_eq!(
            kind(s.require_team(&ctx, Action::ShareIssue, eng())),
            Some(ErrorKind::Forbidden)
        );
    }

    #[test]
    fn system_actor_bypasses_everything() {
        let sys = ActorView::new(Actor::System, &[]);
        let ctx = RequestCtx::system(org(), "req_sys");
        let s = snap(sys);
        assert!(s.require_team(&ctx, Action::ChangeTeamKey, des()).is_ok());
        assert!(s.require_workspace(&ctx, Action::DeleteWorkspace).is_ok());
        let _ = TeamVisibility::Public;
    }
}
