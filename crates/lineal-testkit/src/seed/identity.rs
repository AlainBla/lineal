//! Seed-Bereich `identity` (WP02): Workspace `acme`, Teams ENG, WEB (Sub-Team von ENG), DES (privat),
//! Alice (owner), Bob (user), Carol (guest) gemäß docs/02, Beispieltabelle.
//!
//! Wichtig: die Transaktion muss mit `Store::begin(SeedData::acme().org)` geöffnet sein
//! (die Repos filtern auf `tx.org_id()`).
//!
//! Dev-Passwort aller drei User: `lineal-dev` (Argon2id-PHC-Hash, mit jedem Argon2-Verifier prüfbar).

use super::SeedData;
use chrono::{DateTime, TimeZone, Utc};
use lineal_app::AppResult;
use lineal_app::policy::{default_limits_json, default_security_settings_json};
use lineal_app::ports::Tx;
use lineal_app::ports::repos::org::{NewOrg, NewProjectStatus, NewWorkspaceLabel};
use lineal_app::ports::repos::state::NewState;
use lineal_app::ports::repos::team::{NewMembership, NewTeam, TeamPatch};
use lineal_app::ports::repos::user::NewUser;
use lineal_domain::enums::{ProjectStatusType, StateType, TeamVisibility, UserRole};
use lineal_domain::ids::{StateId, TeamId, UserId};
use serde_json::json;
use uuid::Uuid;

/// Klartext-Passwort der Seed-User (nur Entwicklung).
pub const SEED_PASSWORD: &str = "lineal-dev";

/// Argon2id-Hash von [`SEED_PASSWORD`].
pub const SEED_PASSWORD_HASH: &str =
    "$argon2id$v=19$m=19456,t=2,p=1$bGluZWFsLWRldi1zZWVkLXNhbHQ$fYAmjfmRwFiLh1hBYr6L73LQaxuE6yZwll5rmfPYxa8";

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).single().unwrap_or_default()
}

const WORKFLOW: [(&str, StateType, &str); 6] = [
    ("Backlog", StateType::Backlog, "#BEC2C8"),
    ("Todo", StateType::Unstarted, "#E2E2E2"),
    ("In Progress", StateType::Started, "#F2C94C"),
    ("Done", StateType::Completed, "#5E6AD2"),
    ("Canceled", StateType::Canceled, "#95A2B3"),
    ("Duplicate", StateType::Duplicate, "#95A2B3"),
];

/// Feste Status-ID: `team_slot` unterscheidet die Teams, `index` den Status.
pub fn seed_state_id(team_slot: u128, index: usize) -> StateId {
    StateId(Uuid::from_u128(0x5747_E000_0000_7000_8000_0000_0000_0000 + (team_slot << 16) + index as u128))
}

/// Die Status eines Seed-Teams (`slot` 1 = ENG, 2 = WEB, 3 = DES), in `WORKFLOW`-Reihenfolge.
pub fn seed_workflow_state_ids(slot: u128) -> [StateId; 6] {
    std::array::from_fn(|i| seed_state_id(slot, i))
}

async fn workflow(tx: &mut dyn Tx, team: TeamId, slot: u128, inherit_from: Option<u128>) -> AppResult<()> {
    for (i, (name, kind, color)) in WORKFLOW.iter().enumerate() {
        tx.states()
            .insert(&NewState {
                id: seed_state_id(slot, i),
                team_id: team,
                name: (*name).into(),
                state_type: *kind,
                color: (*color).into(),
                description: None,
                position: 1000.0,
                inherited_from_id: inherit_from.map(|p| seed_state_id(p, i)),
                now: t0(),
            })
            .await?;
    }
    tx.teams()
        .update(
            team,
            &TeamPatch {
                default_issue_state_id: Some(Some(seed_state_id(slot, 0))),
                auto_close_state_id: Some(Some(seed_state_id(slot, 4))),
                ..Default::default()
            },
            t0(),
        )
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn team(
    tx: &mut dyn Tx,
    id: TeamId,
    name: &str,
    key: &str,
    parent: Option<TeamId>,
    visibility: TeamVisibility,
    icon: &str,
    color: &str,
    inherit_workflow: bool,
) -> AppResult<()> {
    tx.teams()
        .insert(&NewTeam {
            id,
            name: name.into(),
            key: key.into(),
            description: None,
            icon: Some(icon.into()),
            color: Some(color.into()),
            parent_id: parent,
            visibility,
            all_members_can_join: true,
            timezone: "Europe/Berlin".into(),
            security_settings: json!({
                "teamManagement": "member", "labelManagement": "member", "templateManagement": "member",
                "memberManagement": "member", "issueSharing": "owner", "pinnedViewManagement": "member"
            }),
            triage_enabled: false,
            inherit_workflow_statuses: inherit_workflow,
            inherit_issue_estimation: false,
            now: t0(),
        })
        .await
}

async fn member(tx: &mut dyn Tx, n: u128, team: TeamId, user: UserId, owner: bool) -> AppResult<()> {
    tx.teams()
        .insert_membership(&NewMembership {
            id: Uuid::from_u128(0x3E3B_0000_0000_7000_8000_0000_0000_0000 + n),
            team_id: team,
            user_id: user,
            owner,
            now: t0(),
        })
        .await
}

/// Legt die Beispieldaten dieses Bereichs an.
pub async fn seed(tx: &mut dyn Tx, data: &SeedData) -> AppResult<()> {
    let now = t0();
    tx.orgs()
        .insert(&NewOrg {
            id: data.org,
            name: "Acme".into(),
            url_key: "acme".into(),
            security_settings: default_security_settings_json(),
            limits: default_limits_json(),
            now,
        })
        .await?;
    for (i, (name, kind, color, indefinite)) in [
        ("Backlog", ProjectStatusType::Backlog, "#BEC2C8", true),
        ("Planned", ProjectStatusType::Planned, "#E2E2E2", true),
        ("In Progress", ProjectStatusType::Started, "#F2C94C", false),
        ("Paused", ProjectStatusType::Paused, "#95A2B3", true),
        ("Completed", ProjectStatusType::Completed, "#5E6AD2", true),
        ("Canceled", ProjectStatusType::Canceled, "#95A2B3", true),
    ]
    .into_iter()
    .enumerate()
    {
        tx.orgs()
            .insert_project_status(&NewProjectStatus {
                id: Uuid::from_u128(0x9A05_0000_0000_7000_8000_0000_0000_0000 + i as u128),
                name: name.into(),
                status_type: kind,
                color: color.into(),
                position: (i as f64 + 1.0) * 1000.0,
                indefinite,
                now,
            })
            .await?;
    }

    for (id, name, email, display, role) in [
        (data.alice, "Alice Example", "alice@acme.example", "alice", UserRole::Owner),
        (data.bob, "Bob Example", "bob@acme.example", "bob", UserRole::User),
        (data.carol, "Carol Example", "carol@acme.example", "carol", UserRole::Guest),
    ] {
        tx.users()
            .insert(&NewUser {
                id,
                name: name.into(),
                display_name: display.into(),
                email: email.into(),
                role,
                password_hash: Some(SEED_PASSWORD_HASH.into()),
                now,
            })
            .await?;
    }
    for (i, (name, color)) in [("Bug", "#EB5757"), ("Feature", "#BB87FC"), ("Improvement", "#4EA7FC")]
        .into_iter()
        .enumerate()
    {
        tx.orgs()
            .insert_workspace_label(&NewWorkspaceLabel {
                id: Uuid::from_u128(0x1ABE_0000_0000_7000_8000_0000_0000_0000 + i as u128),
                name: name.into(),
                color: color.into(),
                creator_id: Some(data.alice.0),
                now,
            })
            .await?;
    }

    team(tx, data.eng, "Engineering", "ENG", None, TeamVisibility::Public, "Code", "#5E6AD2", false).await?;
    workflow(tx, data.eng, 1, None).await?;
    team(tx, data.web, "Web", "WEB", Some(data.eng), TeamVisibility::Public, "Globe", "#26B5CE", true).await?;
    workflow(tx, data.web, 2, Some(1)).await?;
    team(tx, data.des, "Design", "DES", None, TeamVisibility::Private, "Palette", "#F2994A", false).await?;
    workflow(tx, data.des, 3, None).await?;

    // Mitgliedschaften (docs/02, Beispieltabelle)
    member(tx, 1, data.eng, data.alice, true).await?;
    member(tx, 2, data.eng, data.bob, false).await?;
    member(tx, 3, data.web, data.bob, true).await?;
    member(tx, 4, data.des, data.bob, true).await?;
    member(tx, 5, data.web, data.carol, false).await?;
    Ok(())
}
