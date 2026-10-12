//! policy – Workspace-Einstellungen, die Rechte steuern (docs/02 §1.3 Limits, §6.3 Mindestrollen).

use lineal_domain::enums::UserRole;
use serde_json::{Value, json};

/// Rangfolge der Rollen für Mindestrollen-Vergleiche (`app` zählt wie `user`).
pub const fn role_rank(role: UserRole) -> u8 {
    match role {
        UserRole::Owner => 4,
        UserRole::Admin => 3,
        UserRole::User | UserRole::App => 2,
        UserRole::Guest => 1,
    }
}

/// Konfigurierbare Workspace-Limits (docs/02 §1.3). `None` = unbegrenzt.
#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    pub max_teams: Option<i64>,
    pub sub_teams_enabled: bool,
    pub max_team_depth: i64,
    pub private_teams_enabled: bool,
    pub guests_enabled: bool,
    pub team_owners_enabled: bool,
    pub issue_sharing_enabled: bool,
    pub max_active_issues_per_team: Option<i64>,
    pub max_labels_per_group: Option<i64>,
    pub max_upcoming_cycles: i64,
    pub max_unarchived_notifications_per_user: Option<i64>,
    pub trash_retention_days: i64,
    pub workspace_deletion_delay_hours: i64,
    pub max_team_memberships_per_user: Option<i64>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_teams: None,
            sub_teams_enabled: true,
            max_team_depth: 5,
            private_teams_enabled: true,
            guests_enabled: true,
            team_owners_enabled: true,
            issue_sharing_enabled: false,
            max_active_issues_per_team: Some(60_000),
            max_labels_per_group: Some(250),
            max_upcoming_cycles: 15,
            max_unarchived_notifications_per_user: Some(2_000),
            trash_retention_days: 30,
            workspace_deletion_delay_hours: 48,
            max_team_memberships_per_user: None,
        }
    }
}

/// Default-JSON für `Organization.limits` (docs/02 §1.3), als Wert für neue Workspaces.
pub fn default_limits_json() -> Value {
    json!({
        "maxTeams": null,
        "subTeamsEnabled": true,
        "maxTeamDepth": 5,
        "privateTeamsEnabled": true,
        "guestsEnabled": true,
        "teamOwnersEnabled": true,
        "issueSharingEnabled": false,
        "maxActiveIssuesPerTeam": 60000,
        "maxLabelsPerGroup": 250,
        "maxUpcomingCycles": 15,
        "maxUnarchivedNotificationsPerUser": 2000,
        "trashRetentionDays": 30,
        "workspaceDeletionDelayHours": 48,
        "maxTeamMembershipsPerUser": null
    })
}

/// Default-JSON für `Organization.securitySettings` (docs/02 §6.3).
pub fn default_security_settings_json() -> Value {
    json!({
        "teamCreationRole": "user",
        "invitationsRole": "user",
        "adminManagementRole": "admin",
        "labelManagementRole": "user",
        "templateManagementRole": "user",
        "personalApiKeysRole": "user",
        "apiSettingsRole": "admin",
        "workspaceInitiativesRole": "user",
        "pinnedViewManagementRole": "admin",
        "importRole": "admin",
        "allowedInviteDomains": [],
        "auditLogRetentionDays": 365
    })
}

/// Aus den Workspace-Einstellungen abgeleitete Policy-Daten.
#[derive(Debug, Clone, PartialEq)]
pub struct OrgPolicy {
    pub team_creation_role: UserRole,
    pub invitations_role: UserRole,
    pub admin_management_role: UserRole,
    pub label_management_role: UserRole,
    pub template_management_role: UserRole,
    pub personal_api_keys_role: UserRole,
    pub api_settings_role: UserRole,
    pub workspace_initiatives_role: UserRole,
    pub pinned_view_management_role: UserRole,
    pub import_role: UserRole,
    /// Erlaubte E-Mail-Domains für Einladungen und `create_user` (klein geschrieben);
    /// leer = alle Domains.
    pub allowed_invite_domains: Vec<String>,
    /// Aufbewahrung des Audit-Logs in Tagen (Default 365, nie unter 30).
    pub audit_log_retention_days: i64,
    pub limits: Limits,
}

impl Default for OrgPolicy {
    fn default() -> Self {
        Self::from_json(&default_security_settings_json(), &default_limits_json())
    }
}

fn role_of(v: &Value, key: &str, default: UserRole) -> UserRole {
    v.get(key)
        .and_then(Value::as_str)
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn opt_int(v: &Value, keys: &[&str], default: Option<i64>) -> Option<i64> {
    for k in keys {
        match v.get(*k) {
            Some(Value::Null) => return None,
            Some(x) => {
                if let Some(n) = x.as_i64() {
                    return Some(n);
                }
            }
            None => {}
        }
    }
    default
}

fn flag(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(default)
}

impl OrgPolicy {
    /// Liest `securitySettings` und `limits`; fehlende oder ungültige Werte fallen auf die
    /// Defaults zurück (die DB-Defaults aus der Migration nutzen teils andere Schlüsselnamen,
    /// `maxSubTeamDepth`/`maxOpenIssuesPerTeam` werden als Alias akzeptiert).
    pub fn from_json(security: &Value, limits: &Value) -> Self {
        let d = Limits::default();
        Self {
            team_creation_role: role_of(security, "teamCreationRole", UserRole::User),
            invitations_role: role_of(security, "invitationsRole", UserRole::User),
            admin_management_role: role_of(security, "adminManagementRole", UserRole::Admin),
            label_management_role: role_of(security, "labelManagementRole", UserRole::User),
            template_management_role: role_of(security, "templateManagementRole", UserRole::User),
            personal_api_keys_role: role_of(security, "personalApiKeysRole", UserRole::User),
            api_settings_role: role_of(security, "apiSettingsRole", UserRole::Admin),
            workspace_initiatives_role: role_of(
                security,
                "workspaceInitiativesRole",
                UserRole::User,
            ),
            pinned_view_management_role: role_of(
                security,
                "pinnedViewManagementRole",
                UserRole::Admin,
            ),
            import_role: role_of(security, "importRole", UserRole::Admin),
            allowed_invite_domains: security
                .get("allowedInviteDomains")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_lowercase)
                        .collect()
                })
                .unwrap_or_default(),
            audit_log_retention_days: security
                .get("auditLogRetentionDays")
                .and_then(Value::as_i64)
                .filter(|d| *d >= 30)
                .unwrap_or(365),
            limits: Limits {
                max_teams: opt_int(limits, &["maxTeams"], d.max_teams),
                sub_teams_enabled: flag(limits, "subTeamsEnabled", d.sub_teams_enabled),
                max_team_depth: opt_int(
                    limits,
                    &["maxTeamDepth", "maxSubTeamDepth"],
                    Some(d.max_team_depth),
                )
                .unwrap_or(d.max_team_depth),
                private_teams_enabled: flag(limits, "privateTeamsEnabled", d.private_teams_enabled),
                guests_enabled: flag(limits, "guestsEnabled", d.guests_enabled),
                team_owners_enabled: flag(limits, "teamOwnersEnabled", d.team_owners_enabled),
                issue_sharing_enabled: flag(limits, "issueSharingEnabled", d.issue_sharing_enabled),
                max_active_issues_per_team: opt_int(
                    limits,
                    &["maxActiveIssuesPerTeam", "maxOpenIssuesPerTeam"],
                    d.max_active_issues_per_team,
                ),
                max_labels_per_group: opt_int(
                    limits,
                    &["maxLabelsPerGroup"],
                    d.max_labels_per_group,
                ),
                max_upcoming_cycles: opt_int(
                    limits,
                    &["maxUpcomingCycles"],
                    Some(d.max_upcoming_cycles),
                )
                .unwrap_or(d.max_upcoming_cycles),
                max_unarchived_notifications_per_user: opt_int(
                    limits,
                    &["maxUnarchivedNotificationsPerUser"],
                    d.max_unarchived_notifications_per_user,
                ),
                trash_retention_days: opt_int(
                    limits,
                    &["trashRetentionDays"],
                    Some(d.trash_retention_days),
                )
                .unwrap_or(d.trash_retention_days),
                workspace_deletion_delay_hours: opt_int(
                    limits,
                    &["workspaceDeletionDelayHours"],
                    Some(d.workspace_deletion_delay_hours),
                )
                .unwrap_or(d.workspace_deletion_delay_hours),
                max_team_memberships_per_user: opt_int(
                    limits,
                    &["maxTeamMembershipsPerUser"],
                    d.max_team_memberships_per_user,
                ),
            },
        }
    }

    /// Ob Einladungen an diese E-Mail-Adresse erlaubt sind (leere Liste = alle Domains).
    pub fn invite_domain_allowed(&self, email: &str) -> bool {
        if self.allowed_invite_domains.is_empty() {
            return true;
        }
        let domain = email
            .rsplit_once('@')
            .map(|(_, d)| d.trim().to_lowercase())
            .unwrap_or_default();
        self.allowed_invite_domains.contains(&domain)
    }

    /// Ob `role` mindestens die Rolle `min` hat.
    pub fn role_at_least(role: UserRole, min: UserRole) -> bool {
        role_rank(role) >= role_rank(min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_docs_02() {
        let p = OrgPolicy::default();
        assert_eq!(p.team_creation_role, UserRole::User);
        assert_eq!(p.api_settings_role, UserRole::Admin);
        assert_eq!(p.limits.max_team_depth, 5);
        assert_eq!(p.limits.max_active_issues_per_team, Some(60_000));
        assert!(p.limits.private_teams_enabled && !p.limits.issue_sharing_enabled);
    }

    #[test]
    fn accepts_migration_default_aliases_and_null_means_unlimited() {
        let limits = json!({ "maxSubTeamDepth": 3, "maxOpenIssuesPerTeam": null, "maxTeams": 2 });
        let p = OrgPolicy::from_json(&json!({}), &limits);
        assert_eq!(p.limits.max_team_depth, 3);
        assert_eq!(p.limits.max_active_issues_per_team, None);
        assert_eq!(p.limits.max_teams, Some(2));
    }

    #[test]
    fn invite_domains_empty_means_all() {
        let mut p = OrgPolicy::default();
        assert!(p.invite_domain_allowed("x@anything.example"));
        p.allowed_invite_domains = vec!["acme.example".into()];
        assert!(p.invite_domain_allowed("Bob@ACME.example"));
        assert!(!p.invite_domain_allowed("bob@evil.example"));
        assert!(!p.invite_domain_allowed("bob@sub.acme.example"));
        assert!(!p.invite_domain_allowed("no-at-sign"));
    }

    #[test]
    fn role_ranks_treat_app_like_user() {
        assert!(role_rank(UserRole::Owner) > role_rank(UserRole::Admin));
        assert_eq!(role_rank(UserRole::App), role_rank(UserRole::User));
        assert!(role_rank(UserRole::Guest) < role_rank(UserRole::User));
    }
}
