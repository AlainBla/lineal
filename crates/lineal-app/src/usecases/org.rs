//! usecases – Workspace (docs/02 §1): anlegen mit Grundgerüst, lesen, Settings ändern.

use super::audit::{AUDIT_RETENTION_MIN_DAYS, AuditEvent};
use super::team::{CreateTeamInput, TeamView, emit};
use super::user::display_name_from_email;
use crate::app::App;
use crate::context::{Actor, RequestCtx, Scopes};
use crate::error::{AppError, AppResult};
use crate::policy::{
    Action, OrgPolicy, PolicySnapshot, default_limits_json, default_security_settings_json,
};
use crate::ports::repos::org::{NewOrg, NewProjectStatus, NewWorkspaceLabel, Org, OrgPatch};
use crate::ports::repos::user::{NewUser, User};
use lineal_domain::enums::{ProjectStatusType, UserRole};
use lineal_domain::events::DomainEvent;
use lineal_domain::ids::{OrgId, UserId};
use serde_json::{Value, json};

/// Eingabe für `create_workspace` (Konto-Session ohne Workspace-Kontext, docs/09 §1.6).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CreateWorkspaceInput {
    pub name: String,
    pub url_key: Option<String>,
    pub owner_name: String,
    pub owner_email: String,
    /// Klartext-Passwort des Owners (mindestens 8 Zeichen); `None` = kein Passwort-Login.
    pub owner_password: Option<String>,
    /// Herkunft für den Audit-Log: `Some("cli")` beim CLI, `Some("session")` bei einer Konto-Session,
    /// `None` ohne Zugangsdaten.
    pub credential_kind: Option<&'static str>,
}

/// Ergebnis von `create_workspace`.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceCreated {
    pub org: Org,
    pub owner: User,
    /// Das Default-Team (Name = Workspace-Name).
    pub team: TeamView,
}

/// Eingabe für `update_workspace` (`None` = unverändert).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdateWorkspaceInput {
    pub name: Option<String>,
    pub url_key: Option<String>,
    pub logo_url: Option<Option<String>>,
    pub fiscal_year_start_month: Option<i32>,
    pub initiatives_enabled: Option<bool>,
    /// Mindestrollen (flach zusammengeführt); nur Owner.
    pub security_settings: Option<Value>,
    /// Limits (flach zusammengeführt); nur Owner.
    pub limits: Option<Value>,
}

/// URL-Key: `^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$` (docs/02 §1.1).
pub fn validate_url_key(key: &str) -> AppResult<()> {
    let b = key.as_bytes();
    let alnum = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    let ok = (3..=32).contains(&b.len())
        && alnum(b[0])
        && alnum(b[b.len() - 1])
        && b.iter().all(|c| alnum(*c) || *c == b'-');
    if ok {
        Ok(())
    } else {
        Err(
            AppError::invalid_request("urlKey must match ^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$")
                .with_details(json!({ "field": "urlKey" })),
        )
    }
}

/// Erzeugt einen URL-Key aus dem Namen (Kleinbuchstaben, Umlaute transliteriert,
/// Nicht-Alphanumerisches zu `-`).
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        let part: String = match c {
            'ä' | 'Ä' => "ae".into(),
            'ö' | 'Ö' => "oe".into(),
            'ü' | 'Ü' => "ue".into(),
            'ß' => "ss".into(),
            c if c.is_ascii_alphanumeric() => c.to_ascii_lowercase().to_string(),
            _ => "-".into(),
        };
        out.push_str(&part);
    }
    let mut collapsed = String::new();
    for c in out.chars() {
        if c == '-' && collapsed.ends_with('-') {
            continue;
        }
        collapsed.push(c);
    }
    let trimmed = collapsed.trim_matches('-');
    let mut slug: String = trimmed.chars().take(32).collect();
    slug = slug.trim_end_matches('-').to_string();
    if slug.len() < 3 {
        slug = format!("{slug}-workspace").trim_matches('-').to_string();
    }
    slug
}

fn merge_object(base: &Value, patch: &Value) -> Value {
    let mut merged = base.as_object().cloned().unwrap_or_default();
    if let Some(p) = patch.as_object() {
        merged.extend(p.clone());
    }
    Value::Object(merged)
}

/// Standard-Projektstatus eines neuen Workspace (docs/04 §2.3).
const PROJECT_STATUSES: [(&str, ProjectStatusType, &str, bool); 6] = [
    ("Backlog", ProjectStatusType::Backlog, "#BEC2C8", true),
    ("Planned", ProjectStatusType::Planned, "#E2E2E2", true),
    ("In Progress", ProjectStatusType::Started, "#F2C94C", false),
    ("Paused", ProjectStatusType::Paused, "#95A2B3", true),
    ("Completed", ProjectStatusType::Completed, "#5E6AD2", true),
    ("Canceled", ProjectStatusType::Canceled, "#95A2B3", true),
];

/// Standard-Labels (docs/02 §2.4).
const WORKSPACE_LABELS: [(&str, &str); 3] = [
    ("Bug", "#EB5757"),
    ("Feature", "#BB87FC"),
    ("Improvement", "#4EA7FC"),
];

impl App {
    /// Legt einen Workspace samt Owner, Projektstatus, Standard-Labels und Default-Team an
    /// (docs/02 §1.1), in einer Transaktion.
    pub async fn create_workspace(
        &self,
        input: CreateWorkspaceInput,
    ) -> AppResult<WorkspaceCreated> {
        let name = input.name.trim().to_string();
        if name.is_empty() || name.chars().count() > 80 {
            return Err(AppError::invalid_request("name must be 1-80 characters")
                .with_details(json!({ "field": "name" })));
        }
        let owner_name = input.owner_name.trim().to_string();
        if owner_name.is_empty() || owner_name.chars().count() > 255 {
            return Err(
                AppError::invalid_request("owner name must be 1-255 characters")
                    .with_details(json!({ "field": "ownerName" })),
            );
        }
        let email = input.owner_email.trim().to_string();
        super::user::validate_email(&email)?;
        let password_hash = match &input.owner_password {
            Some(p) => {
                super::user::validate_password(p)?;
                Some(self.hasher.hash(p)?)
            }
            None => None,
        };
        let org_id = OrgId(self.ids.new_uuid());
        let mut tx = self.store.begin(org_id).await?;

        let url_key = match &input.url_key {
            Some(k) => {
                validate_url_key(k)?;
                if tx.orgs().url_key_taken(k).await? {
                    return Err(AppError::conflict("This urlKey is already in use")
                        .with_details(json!({ "field": "urlKey" })));
                }
                k.clone()
            }
            None => {
                let base = slugify(&name);
                let mut candidate = base.clone();
                let mut i = 2;
                while tx.orgs().url_key_taken(&candidate).await? {
                    let suffix = format!("-{i}");
                    let keep: String = base.chars().take(32 - suffix.len()).collect();
                    candidate = format!("{}{}", keep.trim_end_matches('-'), suffix);
                    i += 1;
                }
                candidate
            }
        };

        let now = self.clock.now();
        let url_key_for_audit = url_key.clone();
        tx.orgs()
            .insert(&NewOrg {
                id: org_id,
                name: name.clone(),
                url_key,
                security_settings: default_security_settings_json(),
                limits: default_limits_json(),
                now,
            })
            .await?;
        let owner_id = UserId(self.ids.new_uuid());
        let display_name = display_name_from_email(&mut *tx, &email).await?;
        tx.users()
            .insert(&NewUser {
                id: owner_id,
                name: owner_name,
                display_name,
                email,
                role: UserRole::Owner,
                password_hash,
                now,
            })
            .await?;
        for (i, (n, t, color, indefinite)) in PROJECT_STATUSES.iter().enumerate() {
            tx.orgs()
                .insert_project_status(&NewProjectStatus {
                    id: self.ids.new_uuid(),
                    name: (*n).into(),
                    status_type: *t,
                    color: (*color).into(),
                    position: (i as f64 + 1.0) * 1000.0,
                    indefinite: *indefinite,
                    now,
                })
                .await?;
        }
        for (n, color) in WORKSPACE_LABELS {
            tx.orgs()
                .insert_workspace_label(&NewWorkspaceLabel {
                    id: self.ids.new_uuid(),
                    name: n.into(),
                    color: color.into(),
                    creator_id: Some(owner_id.0),
                    now,
                })
                .await?;
        }
        let ctx = RequestCtx {
            org_id,
            actor: Actor::User {
                id: owner_id,
                role: UserRole::Owner,
            },
            request_id: "req_create_workspace".into(),
            scopes: Scopes::all(),
            team_scope: None,
            credential_kind: input.credential_kind,

            via_session: false,
            client_ip: None,
        };
        emit(
            &mut *tx,
            &ctx,
            DomainEvent::UserCreated { user_id: owner_id },
        )
        .await?;
        let team_id = self
            .create_team_in(
                &mut *tx,
                &ctx,
                CreateTeamInput {
                    name: name.clone(),
                    ..Default::default()
                },
            )
            .await?;
        let tree = crate::policy::TeamTree::new(tx.teams().list_all().await?);
        let team = tree
            .get(team_id)
            .cloned()
            .ok_or_else(|| AppError::internal("default team vanished"))?;
        let team = TeamView {
            display_name: tree.display_name(team_id),
            team,
        };
        self.record_audit(
            &mut *tx,
            &ctx,
            AuditEvent::new(
                "workspace.created",
                "workspace",
                Some(org_id.0),
                json!({ "url_key": url_key_for_audit, "owner_id": owner_id.0 }),
            ),
        )
        .await?;
        let org = tx
            .orgs()
            .get()
            .await?
            .ok_or_else(|| AppError::internal("workspace vanished"))?;
        let owner = tx
            .users()
            .get(owner_id)
            .await?
            .ok_or_else(|| AppError::internal("owner vanished"))?;
        tx.commit().await?;
        Ok(WorkspaceCreated { org, owner, team })
    }

    /// `get_workspace`: der Workspace des Aufrufers.
    pub async fn get_workspace(&self, ctx: &RequestCtx) -> AppResult<Org> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        tx.orgs()
            .get()
            .await?
            .ok_or_else(|| AppError::not_found("Could not find referenced Workspace."))
    }

    /// Absolute URL des Workspace (`<baseUrl>/<urlKey>`).
    pub fn workspace_url(&self, org: &Org) -> String {
        format!(
            "{}/{}",
            self.config.base_url.trim_end_matches('/'),
            org.url_key
        )
    }

    /// Ändert Workspace-Settings (docs/02 §1.2): Admin/Owner; Limits und Mindestrollen nur Owner.
    pub async fn update_workspace(
        &self,
        ctx: &RequestCtx,
        input: UpdateWorkspaceInput,
    ) -> AppResult<Org> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::UpdateWorkspaceSettings)?;
        if input.limits.is_some() || input.security_settings.is_some() {
            snap.require_workspace(ctx, Action::UpdateWorkspaceLimits)?;
        }
        let current = tx
            .orgs()
            .get()
            .await?
            .ok_or_else(|| AppError::not_found("Could not find referenced Workspace."))?;
        let mut patch = OrgPatch {
            now: Some(self.clock.now()),
            ..Default::default()
        };
        if let Some(n) = &input.name {
            let n = n.trim();
            if n.is_empty() || n.chars().count() > 80 {
                return Err(AppError::invalid_request("name must be 1-80 characters")
                    .with_details(json!({ "field": "name" })));
            }
            patch.name = Some(n.to_string());
        }
        if let Some(k) = &input.url_key
            && *k != current.url_key
        {
            validate_url_key(k)?;
            if tx.orgs().url_key_taken(k).await? {
                return Err(AppError::conflict("This urlKey is already in use")
                    .with_details(json!({ "field": "urlKey" })));
            }
            patch.url_key = Some(k.clone());
        }
        patch.logo_url = input.logo_url.clone();
        if let Some(m) = input.fiscal_year_start_month {
            if !(0..=11).contains(&m) {
                return Err(
                    AppError::invalid_request("fiscalYearStartMonth must be 0-11")
                        .with_details(json!({ "field": "fiscalYearStartMonth" })),
                );
            }
            patch.fiscal_year_start_month = Some(m);
        }
        patch.initiatives_enabled = input.initiatives_enabled;
        if let Some(sec) = &input.security_settings {
            let ok = sec.as_object().is_some_and(|o| {
                o.values()
                    .all(|v| v.as_str().is_some_and(|s| s.parse::<UserRole>().is_ok()))
            });
            if !ok {
                return Err(
                    AppError::invalid_request("securitySettings values must be roles")
                        .with_details(json!({ "field": "securitySettings" })),
                );
            }
            patch.security_settings = Some(merge_object(&current.security_settings, sec));
        }
        if let Some(l) = &input.limits {
            if !l.is_object() {
                return Err(AppError::invalid_request("limits must be an object")
                    .with_details(json!({ "field": "limits" })));
            }
            patch.limits = Some(merge_object(&current.limits, l));
        }
        let org = tx.orgs().update(&patch).await?;
        // Sanity: die neuen Einstellungen müssen sich parsen lassen.
        let _ = OrgPolicy::from_json(&org.security_settings, &org.limits);
        tx.commit().await?;
        Ok(org)
    }
}

/// Schlüssel der Rollen-Einstellungen in `securitySettings` (camelCase wie im JSON).
pub const SECURITY_ROLE_KEYS: [&str; 10] = [
    "teamCreationRole",
    "invitationsRole",
    "adminManagementRole",
    "labelManagementRole",
    "templateManagementRole",
    "personalApiKeysRole",
    "apiSettingsRole",
    "workspaceInitiativesRole",
    "pinnedViewManagementRole",
    "importRole",
];

/// Höchstzahl erlaubter Einladungs-Domains.
pub const MAX_INVITE_DOMAINS: usize = 50;
/// Schlüssel der Audit-Log-Aufbewahrung in `securitySettings`.
pub const AUDIT_RETENTION_KEY: &str = "auditLogRetentionDays";
/// Höchstwert der Aufbewahrung (zehn Jahre).
pub const AUDIT_RETENTION_MAX_DAYS: i64 = 3650;

/// Änderung der Security-Settings (`None` = unverändert).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SecuritySettingsPatch {
    /// `(Schlüssel aus SECURITY_ROLE_KEYS, Mindestrolle)`.
    pub roles: Vec<(String, UserRole)>,
    /// Erlaubte Domains für Einladungen; leere Liste = alle.
    pub allowed_invite_domains: Option<Vec<String>>,
    /// Aufbewahrung des Audit-Logs in Tagen (`AUDIT_RETENTION_MIN_DAYS`..=`AUDIT_RETENTION_MAX_DAYS`).
    pub audit_log_retention_days: Option<i64>,
}

/// Normalisiert und prüft eine Domain (`Acme.Example`, `@acme.example` -> `acme.example`).
pub fn normalize_invite_domain(raw: &str) -> AppResult<String> {
    let d = raw.trim().trim_start_matches('@').to_lowercase();
    let label_ok = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    let labels: Vec<&str> = d.split('.').collect();
    if d.len() > 253 || labels.len() < 2 || !labels.iter().all(|l| label_ok(l)) {
        return Err(
            AppError::invalid_request(format!("`{raw}` is not a valid domain"))
                .with_details(json!({ "field": "allowedInviteDomains" })),
        );
    }
    Ok(d)
}

impl App {
    /// Liest die Security-Settings (alle Mitglieder außer Gästen und Apps). Die Werte sind nicht
    /// geheim und werden vom Web gebraucht (Einladungs-Dialog, ausgeblendete Aktionen).
    pub async fn get_security_settings(&self, ctx: &RequestCtx) -> AppResult<OrgPolicy> {
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        if matches!(snap.actor.role, UserRole::Guest | UserRole::App) || !ctx.scopes.read {
            return Err(AppError::forbidden(
                "Security settings are not available to guests or apps.",
            ));
        }
        let org = tx
            .orgs()
            .get()
            .await?
            .ok_or_else(|| AppError::not_found("Could not find referenced Workspace."))?;
        Ok(OrgPolicy::from_json(&org.security_settings, &org.limits))
    }

    /// Ändert die Security-Settings (Admin/Owner; `adminManagementRole` nur Owner). Werte sind
    /// Mindestrollen `user|admin|owner` (`adminManagementRole`: `admin|owner`, sonst könnte jedes
    /// Mitglied Admins ernennen). Owner erfüllen jede Mindestrolle und können sich so nie aussperren.
    pub async fn update_security_settings(
        &self,
        ctx: &RequestCtx,
        patch: SecuritySettingsPatch,
    ) -> AppResult<OrgPolicy> {
        if matches!(ctx.actor, Actor::System) {
            return Err(AppError::forbidden(
                "Security settings cannot be changed by system contexts.",
            ));
        }
        if let Some(days) = patch.audit_log_retention_days
            && !(AUDIT_RETENTION_MIN_DAYS..=AUDIT_RETENTION_MAX_DAYS).contains(&days)
        {
            return Err(AppError::invalid_request(format!(
                "auditLogRetentionDays must be between {AUDIT_RETENTION_MIN_DAYS} and {AUDIT_RETENTION_MAX_DAYS}"
            ))
            .with_details(json!({ "field": "auditLogRetentionDays" })));
        }
        let mut tx = self.store.begin(ctx.org_id).await?;
        let snap = PolicySnapshot::load(&mut *tx, ctx).await?;
        snap.require_workspace(ctx, Action::UpdateWorkspaceSettings)?;
        let mut new_roles: Vec<(&str, UserRole)> = Vec::new();
        for (key, role) in &patch.roles {
            let Some(k) = SECURITY_ROLE_KEYS.iter().find(|k| **k == key) else {
                return Err(
                    AppError::invalid_request(format!("unknown setting `{key}`"))
                        .with_details(json!({ "field": key })),
                );
            };
            let ok = !matches!(
                (*k, role),
                (_, UserRole::Guest | UserRole::App) | ("adminManagementRole", UserRole::User)
            );
            if !ok {
                return Err(AppError::invalid_request(format!(
                    "`{key}` does not allow the role `{}`",
                    role.as_str()
                ))
                .with_details(json!({ "field": key })));
            }
            if *k == "adminManagementRole" {
                snap.require_workspace(ctx, Action::UpdateWorkspaceLimits)?;
            }
            new_roles.push((k, *role));
        }
        let domains = match &patch.allowed_invite_domains {
            Some(list) => {
                if list.len() > MAX_INVITE_DOMAINS {
                    return Err(AppError::invalid_request(format!(
                        "at most {MAX_INVITE_DOMAINS} domains"
                    ))
                    .with_details(json!({ "field": "allowedInviteDomains" })));
                }
                let mut out: Vec<String> = Vec::new();
                for d in list {
                    let d = normalize_invite_domain(d)?;
                    if !out.contains(&d) {
                        out.push(d);
                    }
                }
                Some(out)
            }
            None => None,
        };
        let current = tx
            .orgs()
            .get()
            .await?
            .ok_or_else(|| AppError::not_found("Could not find referenced Workspace."))?;
        let before = merge_object(
            &default_security_settings_json(),
            &current.security_settings,
        );
        let mut merged = before.clone();
        if let Some(obj) = merged.as_object_mut() {
            for (k, r) in &new_roles {
                obj.insert((*k).to_owned(), json!(r.as_str()));
            }
            if let Some(d) = &domains {
                obj.insert("allowedInviteDomains".into(), json!(d));
            }
            if let Some(days) = patch.audit_log_retention_days {
                obj.insert(AUDIT_RETENTION_KEY.into(), json!(days));
            }
        }
        // Audit-Log: vorher/nachher je geänderter Einstellung (nur Werte, keine Geheimnisse).
        let mut changes = serde_json::Map::new();
        let mut change = |key: &str, to: Value| {
            changes.insert(
                key.to_owned(),
                json!({ "from": before.get(key).cloned().unwrap_or(Value::Null), "to": to }),
            );
        };
        for (k, r) in &new_roles {
            change(k, json!(r.as_str()));
        }
        if let Some(d) = &domains {
            change("allowedInviteDomains", json!(d));
        }
        if let Some(days) = patch.audit_log_retention_days {
            change(AUDIT_RETENTION_KEY, json!(days));
        }
        let org = tx
            .orgs()
            .update(&OrgPatch {
                security_settings: Some(merged),
                now: Some(self.clock.now()),
                ..Default::default()
            })
            .await?;
        emit(&mut *tx, ctx, DomainEvent::OrganizationUpdated {}).await?;
        self.record_audit(
            &mut *tx,
            ctx,
            AuditEvent::new(
                "workspace.security_settings_updated",
                "workspace",
                Some(ctx.org_id.0),
                json!({ "changes": changes }),
            ),
        )
        .await?;
        tracing::info!(
            actor = ?ctx.actor,
            changed_roles = ?new_roles.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            domains_changed = domains.is_some(),
            "security settings updated"
        );
        tx.commit().await?;
        Ok(OrgPolicy::from_json(&org.security_settings, &org.limits))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_transliterates_and_collapses() {
        assert_eq!(slugify("Acme"), "acme");
        assert_eq!(slugify("Müller & Söhne GmbH"), "mueller-soehne-gmbh");
        assert_eq!(slugify("  --Hello,   World!!  "), "hello-world");
        assert!(slugify("A").len() >= 3);
        assert!(slugify(&"x".repeat(80)).len() <= 32);
        assert!(validate_url_key(&slugify("Straße 17")).is_ok());
    }

    #[test]
    fn url_key_validation() {
        for ok in ["acme", "a-b", "acme-2", "a1b"] {
            assert!(validate_url_key(ok).is_ok(), "{ok}");
        }
        for bad in ["", "ab", "-acme", "acme-", "Acme", "ac me", &"a".repeat(33)] {
            assert!(validate_url_key(bad).is_err(), "{bad}");
        }
    }
}
