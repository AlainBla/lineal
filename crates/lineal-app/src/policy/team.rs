//! policy – Sichtbarkeit und Team-Hierarchie (docs/02 §3, §5, §6.2, §6.5).
//!
//! Reiner Kern (`TeamTree`, `ActorView`, `compute_*`) plus `PolicySnapshot`, der die Daten einer
//! Anfrage einmal aus der Transaktion lädt. Adapter und Use-Cases anderer Pakete benutzen
//! [`PolicySnapshot`] bzw. die freien Funktionen `visible_team_ids` / `require_team`.

use super::auth::{Action, Resource, require};
use super::org::OrgPolicy;
use crate::context::{Actor, RequestCtx};
use crate::error::{AppError, AppResult};
use crate::ports::Tx;
use crate::ports::repos::team::{Membership, Team};
use lineal_domain::enums::{TeamVisibility, UserRole};
use lineal_domain::ids::TeamId;
use serde_json::json;
use std::collections::{BTreeSet, HashMap};

/// Der Fehler für nicht sichtbare oder nicht vorhandene Teams (Existenz wird nie verraten).
pub fn team_not_found() -> AppError {
    AppError::not_found("Could not find referenced Team.").with_details(json!({ "type": "Team" }))
}

/// Alle Teams des Workspace als Baum (Hierarchie, Tiefe, Anzeigename).
#[derive(Debug, Clone, Default)]
pub struct TeamTree {
    teams: HashMap<TeamId, Team>,
    order: Vec<TeamId>,
}

impl TeamTree {
    pub fn new(teams: Vec<Team>) -> Self {
        let order = teams.iter().map(|t| t.id).collect();
        let teams = teams.into_iter().map(|t| (t.id, t)).collect();
        Self { teams, order }
    }

    pub fn get(&self, id: TeamId) -> Option<&Team> {
        self.teams.get(&id)
    }

    /// Alle Teams in stabiler Reihenfolge (Anlage).
    pub fn all(&self) -> impl Iterator<Item = &Team> {
        self.order.iter().filter_map(|id| self.teams.get(id))
    }

    /// Direkte Kinder.
    pub fn children(&self, id: TeamId) -> Vec<&Team> {
        self.all().filter(|t| t.parent_id == Some(id)).collect()
    }

    /// Vorfahren, nächster zuerst (bricht bei einem Zyklus ab).
    pub fn ancestors(&self, id: TeamId) -> Vec<&Team> {
        let mut out = Vec::new();
        let mut cur = self.get(id).and_then(|t| t.parent_id);
        while let Some(pid) = cur {
            if out.len() > self.teams.len() {
                break;
            }
            match self.get(pid) {
                Some(p) => {
                    out.push(p);
                    cur = p.parent_id;
                }
                None => break,
            }
        }
        out
    }

    /// Alle Nachfahren beliebiger Tiefe (ohne das Team selbst), Breitensuche.
    pub fn descendants(&self, id: TeamId) -> Vec<&Team> {
        let mut out: Vec<&Team> = Vec::new();
        let mut queue = vec![id];
        let mut seen = BTreeSet::new();
        seen.insert(id);
        while let Some(cur) = queue.pop() {
            for c in self.children(cur) {
                if seen.insert(c.id) {
                    out.push(c);
                    queue.push(c.id);
                }
            }
        }
        out
    }

    /// Tiefe: Root = 1.
    pub fn depth(&self, id: TeamId) -> i64 {
        1 + self.ancestors(id).len() as i64
    }

    /// Höhe des Teilbaums: Blatt = 1.
    pub fn height(&self, id: TeamId) -> i64 {
        let kids = self.children(id);
        if kids.is_empty() {
            1
        } else {
            1 + kids
                .iter()
                .map(|c| self.height_guarded(c.id, 0))
                .max()
                .unwrap_or(0)
        }
    }

    fn height_guarded(&self, id: TeamId, depth: usize) -> i64 {
        if depth > self.teams.len() {
            return 0;
        }
        let kids = self.children(id);
        1 + kids
            .iter()
            .map(|c| self.height_guarded(c.id, depth + 1))
            .max()
            .unwrap_or(0)
    }

    /// Ob `candidate` das Team `id` selbst oder einer seiner Nachfahren ist.
    pub fn is_self_or_descendant(&self, id: TeamId, candidate: TeamId) -> bool {
        candidate == id || self.descendants(id).iter().any(|t| t.id == candidate)
    }

    /// `displayName`: Namen aller Vorfahren und des Teams, getrennt durch ` › `.
    pub fn display_name(&self, id: TeamId) -> String {
        let Some(team) = self.get(id) else {
            return String::new();
        };
        let mut names: Vec<&str> = self
            .ancestors(id)
            .iter()
            .rev()
            .map(|t| t.name.as_str())
            .collect();
        names.push(team.name.as_str());
        names.join(" › ")
    }
}

/// Wer handelt, mit seinen Mitgliedschaften (`owner`-Flag je Team).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorView {
    pub actor: Actor,
    pub role: UserRole,
    pub memberships: HashMap<TeamId, bool>,
    /// Team-Beschränkung eines API-Keys (`RequestCtx::team_scope`); `None` = unbeschränkt.
    pub team_scope: Option<BTreeSet<TeamId>>,
}

impl ActorView {
    pub fn new(actor: Actor, memberships: &[Membership]) -> Self {
        let role = match actor {
            Actor::User { role, .. } => role,
            Actor::App { .. } => UserRole::App,
            Actor::System => UserRole::Owner,
        };
        Self {
            actor,
            role,
            memberships: memberships.iter().map(|m| (m.team_id, m.owner)).collect(),
            team_scope: None,
        }
    }

    /// Beschränkt die Sicht auf die Teams eines team-beschränkten API-Keys.
    pub fn with_team_scope(mut self, scope: Option<&[TeamId]>) -> Self {
        self.team_scope = scope.map(|s| s.iter().copied().collect());
        self
    }

    /// Liegt das Team im Geltungsbereich des Tokens (immer wahr ohne Team-Beschränkung)?
    pub fn in_scope(&self, team: TeamId) -> bool {
        self.team_scope.as_ref().is_none_or(|s| s.contains(&team))
    }

    pub fn is_system(&self) -> bool {
        matches!(self.actor, Actor::System)
    }

    pub fn is_member(&self, team: TeamId) -> bool {
        self.memberships.contains_key(&team)
    }

    pub fn is_admin(&self) -> bool {
        matches!(self.role, UserRole::Owner | UserRole::Admin)
    }
}

/// `accessibleTeamIds` (docs/02 §6.5), rein berechnet; bei einem team-beschränkten API-Key
/// auf dessen Teams geschnitten.
pub fn compute_visible_team_ids(actor: &ActorView, tree: &TeamTree) -> BTreeSet<TeamId> {
    let mut ids = compute_visible_unscoped(actor, tree);
    if let Some(scope) = &actor.team_scope {
        ids.retain(|t| scope.contains(t));
    }
    ids
}

fn compute_visible_unscoped(actor: &ActorView, tree: &TeamTree) -> BTreeSet<TeamId> {
    if actor.is_system() {
        return tree.all().filter(|t| !t.trashed).map(|t| t.id).collect();
    }
    let mut ids: BTreeSet<TeamId> = actor.memberships.keys().copied().collect();
    if actor.role != UserRole::Guest {
        for t in tree.all() {
            match t.visibility {
                TeamVisibility::Public => {
                    ids.insert(t.id);
                }
                TeamVisibility::Restricted => {
                    if t.parent_id.is_some_and(|p| actor.is_member(p)) {
                        ids.insert(t.id);
                    }
                }
                TeamVisibility::Private => {}
            }
        }
    }
    ids.retain(|id| tree.get(*id).is_some_and(|t| !t.trashed));
    ids
}

/// Ob der Akteur effektiver Team-Owner ist (docs/02 §6.2).
pub fn compute_is_team_owner(
    actor: &ActorView,
    tree: &TeamTree,
    org: &OrgPolicy,
    team: TeamId,
    can_see_metadata: bool,
) -> bool {
    if actor.is_system() {
        return true;
    }
    if matches!(actor.role, UserRole::Guest | UserRole::App) {
        return false;
    }
    if actor.is_admin() && can_see_metadata {
        return true;
    }
    if !org.limits.team_owners_enabled {
        return false;
    }
    std::iter::once(team)
        .chain(tree.ancestors(team).iter().map(|t| t.id))
        .any(|id| actor.memberships.get(&id).copied().unwrap_or(false))
}

/// Regel „Sub-Team nicht offener als Parent“ (docs/02 §5.2).
pub fn visibility_allowed_under_parent(parent: TeamVisibility, child: TeamVisibility) -> bool {
    use TeamVisibility::*;
    match (parent, child) {
        (Public, _) => true,
        (Restricted, Public) | (Private, Public) => false,
        (Restricted, _) | (Private, _) => true,
    }
}

/// Alles, was `policy::require` über ein Team und den Akteur wissen muss.
#[derive(Debug, Clone, PartialEq)]
pub struct TeamPolicyView {
    pub team: Team,
    pub actor_role: UserRole,
    pub is_system: bool,
    pub is_member: bool,
    pub can_access: bool,
    pub can_see_metadata: bool,
    pub is_team_owner: bool,
    pub can_join: bool,
    pub org: OrgPolicy,
}

/// `canJoin` (docs/02 §4.2; ohne scimManaged, das es nicht gibt).
pub fn compute_can_join(actor: &ActorView, org: &OrgPolicy, team: &Team) -> bool {
    if actor.is_system() || actor.is_member(team.id) {
        return false;
    }
    if team.trashed || team.retired_at.is_some() {
        return false;
    }
    if matches!(actor.role, UserRole::Guest | UserRole::App) {
        return false;
    }
    if let Some(max) = org.limits.max_team_memberships_per_user
        && actor.memberships.len() as i64 >= max
    {
        return false;
    }
    match team.visibility {
        TeamVisibility::Public => team.all_members_can_join || actor.is_admin(),
        TeamVisibility::Restricted => team.parent_id.is_some_and(|p| actor.is_member(p)),
        TeamVisibility::Private => actor.is_admin(),
    }
}

/// Berechnet die Sicht auf ein Team; `None`, wenn das Team nicht existiert.
pub fn compute_policy_view(
    actor: &ActorView,
    tree: &TeamTree,
    org: &OrgPolicy,
    team: TeamId,
) -> Option<TeamPolicyView> {
    let t = tree.get(team)?;
    let visible = compute_visible_team_ids(actor, tree);
    let can_access = visible.contains(&team);
    let can_see_metadata = if actor.is_system() || actor.is_admin() {
        actor.in_scope(team)
    } else {
        can_access
    };
    let is_team_owner = compute_is_team_owner(actor, tree, org, team, can_see_metadata);
    Some(TeamPolicyView {
        team: t.clone(),
        actor_role: actor.role,
        is_system: actor.is_system(),
        is_member: actor.is_member(team),
        can_access,
        can_see_metadata,
        is_team_owner,
        can_join: compute_can_join(actor, org, t),
        org: org.clone(),
    })
}

/// Pro Anfrage geladene Daten für alle Policy-Entscheidungen. Einmal bauen, mehrfach benutzen
/// (die „Pro-Request-Cache“-Rolle aus docs/02 §6.5).
#[derive(Debug, Clone)]
pub struct PolicySnapshot {
    pub actor: ActorView,
    pub tree: TeamTree,
    pub org: OrgPolicy,
}

impl PolicySnapshot {
    /// Lädt Teams, Mitgliedschaften des Akteurs und Workspace-Einstellungen.
    pub async fn load(tx: &mut dyn Tx, ctx: &RequestCtx) -> AppResult<Self> {
        let teams = tx.teams().list_all().await?;
        let memberships = match ctx.actor {
            Actor::User { id, .. } | Actor::App { id } => {
                tx.teams().list_memberships_of_user(id).await?
            }
            Actor::System => Vec::new(),
        };
        let org = match tx.orgs().get().await {
            Ok(Some(org)) => OrgPolicy::from_json(&org.security_settings, &org.limits),
            Ok(None) => OrgPolicy::default(),
            Err(e) if e.message == "not implemented" => OrgPolicy::default(),
            Err(e) => return Err(e),
        };
        Ok(Self {
            actor: ActorView::new(ctx.actor, &memberships)
                .with_team_scope(ctx.team_scope.as_deref()),
            tree: TeamTree::new(teams),
            org,
        })
    }

    /// `accessibleTeamIds`: für den Akteur sichtbare Teams (nicht gelöscht; stillgelegte bleiben lesbar).
    pub fn visible_team_ids(&self) -> BTreeSet<TeamId> {
        compute_visible_team_ids(&self.actor, &self.tree)
    }

    /// Sicht auf ein Team; `not_found`, wenn es nicht existiert. Existenz-Entscheidungen
    /// (privat → `not_found`) trifft [`require`].
    pub fn view(&self, team: TeamId) -> AppResult<TeamPolicyView> {
        compute_policy_view(&self.actor, &self.tree, &self.org, team).ok_or_else(team_not_found)
    }

    /// `require` für ein Team: liefert die Sicht zurück, wenn die Aktion erlaubt ist.
    pub fn require_team(
        &self,
        ctx: &RequestCtx,
        action: Action,
        team: TeamId,
    ) -> AppResult<TeamPolicyView> {
        let view = self.view(team)?;
        require(ctx, action, Resource::Team(&view))?;
        Ok(view)
    }

    /// `require` für mehrere Teams (Teamwechsel, Projekte): `not_found` hat Vorrang vor `forbidden`.
    pub fn require_teams(
        &self,
        ctx: &RequestCtx,
        action: Action,
        teams: &[TeamId],
    ) -> AppResult<Vec<TeamPolicyView>> {
        let views = teams
            .iter()
            .map(|t| self.view(*t))
            .collect::<AppResult<Vec<_>>>()?;
        require(ctx, action, Resource::Teams(&views))?;
        Ok(views)
    }

    /// `require` für Workspace-Aktionen.
    pub fn require_workspace(&self, ctx: &RequestCtx, action: Action) -> AppResult<()> {
        require(ctx, action, Resource::Workspace(&self.org))
    }
}

/// `policy::visible_team_ids`: die für den Akteur sichtbaren Teams. Läuft in der laufenden
/// Transaktion; der Aufrufer hält die Menge für die Dauer des Requests.
pub async fn visible_team_ids(tx: &mut dyn Tx, ctx: &RequestCtx) -> AppResult<BTreeSet<TeamId>> {
    Ok(PolicySnapshot::load(tx, ctx).await?.visible_team_ids())
}

/// Komfortfunktion: Snapshot laden und `require` für ein Team ausführen.
pub async fn require_team(
    tx: &mut dyn Tx,
    ctx: &RequestCtx,
    action: Action,
    team: TeamId,
) -> AppResult<TeamPolicyView> {
    PolicySnapshot::load(tx, ctx)
        .await?
        .require_team(ctx, action, team)
}

/// Komfortfunktion für Workspace-Aktionen.
pub async fn require_workspace(
    tx: &mut dyn Tx,
    ctx: &RequestCtx,
    action: Action,
) -> AppResult<OrgPolicy> {
    let snap = PolicySnapshot::load(tx, ctx).await?;
    snap.require_workspace(ctx, action)?;
    Ok(snap.org)
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Seed-Struktur aus docs/02: ENG (public), WEB (Sub-Team von ENG), DES (privat);
    //! Alice owner (Owner in ENG), Bob user (Mitglied ENG/WEB/DES, Owner in WEB/DES), Carol guest (WEB).

    use super::*;
    use crate::ports::repos::team::NewTeam;
    use chrono::{TimeZone, Utc};
    use lineal_domain::ids::{OrgId, UserId};
    use uuid::Uuid;

    pub fn org() -> OrgId {
        OrgId(Uuid::from_u128(1))
    }
    pub fn alice() -> UserId {
        UserId(Uuid::from_u128(11))
    }
    pub fn bob() -> UserId {
        UserId(Uuid::from_u128(12))
    }
    pub fn carol() -> UserId {
        UserId(Uuid::from_u128(13))
    }
    pub fn eng() -> TeamId {
        TeamId(Uuid::from_u128(21))
    }
    pub fn web() -> TeamId {
        TeamId(Uuid::from_u128(22))
    }
    pub fn des() -> TeamId {
        TeamId(Uuid::from_u128(23))
    }

    pub fn team(id: TeamId, key: &str, parent: Option<TeamId>, visibility: TeamVisibility) -> Team {
        let now = Utc
            .with_ymd_and_hms(2026, 1, 5, 9, 0, 0)
            .single()
            .unwrap_or_default();
        Team::from_new(
            org(),
            &NewTeam {
                id,
                name: key.to_string(),
                key: key.to_string(),
                description: None,
                icon: None,
                color: None,
                parent_id: parent,
                visibility,
                all_members_can_join: true,
                timezone: "America/Los_Angeles".into(),
                security_settings: json!({}),
                triage_enabled: false,
                inherit_workflow_statuses: false,
                inherit_issue_estimation: false,
                now,
            },
        )
    }

    pub fn tree() -> TeamTree {
        TeamTree::new(vec![
            team(eng(), "ENG", None, TeamVisibility::Public),
            team(web(), "WEB", Some(eng()), TeamVisibility::Public),
            team(des(), "DES", None, TeamVisibility::Private),
        ])
    }

    fn m(team: TeamId, user: UserId, owner: bool) -> Membership {
        let now = Utc
            .with_ymd_and_hms(2026, 1, 5, 9, 0, 0)
            .single()
            .unwrap_or_default();
        Membership {
            id: Uuid::new_v4(),
            team_id: team,
            user_id: user,
            owner,
            sort_order: 0.0,
            created_at: now,
        }
    }

    pub fn alice_view() -> ActorView {
        ActorView::new(
            Actor::User {
                id: alice(),
                role: UserRole::Owner,
            },
            &[m(eng(), alice(), true)],
        )
    }
    pub fn bob_view() -> ActorView {
        ActorView::new(
            Actor::User {
                id: bob(),
                role: UserRole::User,
            },
            &[
                m(eng(), bob(), false),
                m(web(), bob(), true),
                m(des(), bob(), true),
            ],
        )
    }
    pub fn carol_view() -> ActorView {
        ActorView::new(
            Actor::User {
                id: carol(),
                role: UserRole::Guest,
            },
            &[m(web(), carol(), false)],
        )
    }
    pub fn ctx_of(view: &ActorView) -> RequestCtx {
        RequestCtx {
            org_id: org(),
            actor: view.actor,
            request_id: "req_test".into(),
            scopes: crate::context::Scopes::all(),
            team_scope: None,
            via_session: false,
            client_ip: None,
            credential_kind: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn visible_ids_follow_the_example_table() {
        let t = tree();
        // Alice: Mitglied nur in ENG, sieht public ENG + WEB, nicht privates DES.
        assert_eq!(
            compute_visible_team_ids(&alice_view(), &t),
            BTreeSet::from([eng(), web()])
        );
        // Bob: Mitglied in allen drei.
        assert_eq!(
            compute_visible_team_ids(&bob_view(), &t),
            BTreeSet::from([eng(), web(), des()])
        );
        // Carol (Gast): nur ihre Mitgliedschaft, keine öffentlichen Teams.
        assert_eq!(
            compute_visible_team_ids(&carol_view(), &t),
            BTreeSet::from([web()])
        );
    }

    #[test]
    fn restricted_team_is_visible_to_parent_members_only() {
        let secret = TeamId(uuid::Uuid::from_u128(24));
        let mut teams: Vec<Team> = tree().all().cloned().collect();
        teams.push(team(secret, "SEC", Some(des()), TeamVisibility::Restricted));
        let t = TeamTree::new(teams);
        // Alice ist nicht Mitglied von DES -> sieht SEC nicht; Bob (Mitglied DES) sieht es.
        assert!(!compute_visible_team_ids(&alice_view(), &t).contains(&secret));
        assert!(compute_visible_team_ids(&bob_view(), &t).contains(&secret));
    }

    #[test]
    fn trashed_teams_are_never_visible() {
        let mut teams: Vec<Team> = tree().all().cloned().collect();
        teams[0].trashed = true;
        let t = TeamTree::new(teams);
        assert!(!compute_visible_team_ids(&bob_view(), &t).contains(&eng()));
    }

    #[test]
    fn system_actor_sees_everything_not_trashed() {
        let sys = ActorView::new(Actor::System, &[]);
        assert_eq!(
            compute_visible_team_ids(&sys, &tree()),
            BTreeSet::from([eng(), web(), des()])
        );
    }

    #[test]
    fn hierarchy_queries() {
        let t = tree();
        assert_eq!(t.depth(eng()), 1);
        assert_eq!(t.depth(web()), 2);
        assert_eq!(t.height(eng()), 2);
        assert_eq!(t.height(web()), 1);
        assert_eq!(t.display_name(web()), "ENG › WEB");
        assert_eq!(t.descendants(eng()).len(), 1);
        assert!(t.is_self_or_descendant(eng(), web()));
        assert!(!t.is_self_or_descendant(web(), eng()));
    }

    #[test]
    fn team_owner_is_derived_from_ancestors() {
        let t = tree();
        let org = OrgPolicy::default();
        // Alice ist Owner von ENG und damit Owner von WEB, obwohl nicht Mitglied.
        let alice = alice_view();
        let see = compute_visible_team_ids(&alice, &t).contains(&web());
        assert!(compute_is_team_owner(&alice, &t, &org, web(), see));
        // Bob ist nur Mitglied von ENG, aber Owner von WEB und DES.
        let bob = bob_view();
        assert!(compute_is_team_owner(&bob, &t, &org, web(), true));
        assert!(!compute_is_team_owner(&bob, &t, &org, eng(), true));
        // Gast nie.
        assert!(!compute_is_team_owner(&carol_view(), &t, &org, web(), true));
    }

    #[test]
    fn team_owners_can_be_disabled() {
        let mut org = OrgPolicy::default();
        org.limits.team_owners_enabled = false;
        // Bob (user) ist nicht mehr Owner; Admin/Owner (Alice) bleibt Verwaltungs-Owner.
        assert!(!compute_is_team_owner(
            &bob_view(),
            &tree(),
            &org,
            web(),
            true
        ));
        assert!(compute_is_team_owner(
            &alice_view(),
            &tree(),
            &org,
            eng(),
            true
        ));
    }

    #[test]
    fn admins_see_metadata_of_private_teams_but_not_content() {
        let t = tree();
        let org = OrgPolicy::default();
        let view = compute_policy_view(&alice_view(), &t, &org, des()).unwrap();
        assert!(!view.can_access);
        assert!(view.can_see_metadata);
        assert!(view.is_team_owner);
        let bob = compute_policy_view(&bob_view(), &t, &OrgPolicy::default(), des()).unwrap();
        assert!(bob.can_access && bob.is_member);
    }

    #[test]
    fn can_join_rules() {
        let t = tree();
        let org = OrgPolicy::default();
        // Alice darf WEB (public, allMembersCanJoin) beitreten, Mitglied von ENG nicht erneut.
        assert!(
            compute_policy_view(&alice_view(), &t, &org, web())
                .unwrap()
                .can_join
        );
        assert!(
            !compute_policy_view(&alice_view(), &t, &org, eng())
                .unwrap()
                .can_join
        );
        // Privates DES: Admin darf (mit Warnhinweis), Gäste nie.
        assert!(
            compute_policy_view(&alice_view(), &t, &org, des())
                .unwrap()
                .can_join
        );
        assert!(
            !compute_policy_view(&carol_view(), &t, &org, eng())
                .unwrap()
                .can_join
        );
    }

    #[test]
    fn sub_team_visibility_matrix() {
        use TeamVisibility::*;
        for child in [Public, Restricted, Private] {
            assert!(visibility_allowed_under_parent(Public, child));
        }
        assert!(!visibility_allowed_under_parent(Restricted, Public));
        assert!(visibility_allowed_under_parent(Restricted, Restricted));
        assert!(visibility_allowed_under_parent(Restricted, Private));
        assert!(!visibility_allowed_under_parent(Private, Public));
        assert!(visibility_allowed_under_parent(Private, Restricted));
        assert!(visibility_allowed_under_parent(Private, Private));
    }
}
