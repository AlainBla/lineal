//! Seed-Bereich `planning` (WP07): Cycles für ENG (Zeitzone Europe/Berlin, über die Zeitumstellung
//! 2026-10-25), das Projekt `P-ENG-1` „Checkout v2“ mit Meilenstein und die Initiative `I-1`.
//!
//! Alle IDs sind fest, damit Tests sie kennen. Die Cycle-Fenster passen zur Standard-Testuhr
//! (2026-10-10): #1 abgeschlossen, #2 aktiv, #3 und #4 künftig.

use super::SeedData;
use chrono::{DateTime, TimeZone, Utc};
use lineal_app::AppResult;
use lineal_app::ports::Tx;
use lineal_app::ports::repos::cycle::NewCycle;
use lineal_app::ports::repos::initiative::NewInitiative;
use lineal_app::ports::repos::project::{NewMilestone, NewProject};
use lineal_app::ports::repos::team::TeamPatch;
use lineal_domain::enums::InitiativeStatus;
use lineal_domain::ids::{CycleId, InitiativeId, MilestoneId, ProjectId};
use uuid::Uuid;

/// Feste ID des Seed-Cycles Nummer `n` (1–4) von ENG.
pub fn seed_cycle_id(n: u128) -> CycleId {
    CycleId(Uuid::from_u128(0xC7C1_E000_0000_7000_8000_0000_0000_0000 + n))
}

/// Das Seed-Projekt `P-ENG-1`.
pub fn seed_project_id() -> ProjectId {
    ProjectId(Uuid::from_u128(0x9201_EC70_0000_7000_8000_0000_0000_0001))
}

/// Der Seed-Meilenstein „Alpha“.
pub fn seed_milestone_id() -> MilestoneId {
    MilestoneId(Uuid::from_u128(0x3115_70E0_0000_7000_8000_0000_0000_0001))
}

/// Die Seed-Initiative `I-1`.
pub fn seed_initiative_id() -> InitiativeId {
    InitiativeId(Uuid::from_u128(0x1117_1A71_0000_7000_8000_0000_0000_0001))
}

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0)
        .single()
        .unwrap_or_default()
}

fn at(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, 0, 0).single().unwrap_or_default()
}

/// Legt die Beispieldaten dieses Bereichs an.
pub async fn seed(tx: &mut dyn Tx, data: &SeedData) -> AppResult<()> {
    let now = t0();
    // Cycles für ENG: 2 Wochen, Montag, Europe/Berlin.
    tx.teams()
        .update(
            data.eng,
            &TeamPatch {
                cycles_enabled: Some(true),
                cycle_duration: Some(2),
                cycle_cooldown_time: Some(0),
                cycle_start_day: Some(1),
                upcoming_cycle_count: Some(2),
                ..Default::default()
            },
            now,
        )
        .await?;
    let windows = [
        (at(2026, 9, 20, 22), at(2026, 10, 4, 22)),
        (at(2026, 10, 4, 22), at(2026, 10, 18, 22)),
        (at(2026, 10, 18, 22), at(2026, 11, 1, 23)),
        (at(2026, 11, 1, 23), at(2026, 11, 15, 23)),
    ];
    for (i, (start, end)) in windows.iter().enumerate() {
        let number = tx.teams().next_cycle_number(data.eng).await?;
        let id = seed_cycle_id(i as u128 + 1);
        tx.cycles()
            .insert(&NewCycle {
                id,
                team_id: data.eng,
                number,
                name: None,
                description: None,
                starts_at: *start,
                ends_at: *end,
                inherited_from_id: None,
                now,
            })
            .await?;
        if i == 0 {
            tx.cycles().mark_started(id, now).await?;
            tx.cycles().close(id, *end, *end, &[], 0.0, now).await?;
        } else if i == 1 {
            tx.cycles().mark_started(id, now).await?;
        }
    }

    // Projekt P-ENG-1 „Checkout v2“ (Status „In Progress“, Lead Alice).
    let number = tx.teams().next_project_number(data.eng).await?;
    let project = seed_project_id();
    tx.projects()
        .insert(&NewProject {
            id: project,
            name: "Checkout v2".into(),
            lead_team_id: data.eng,
            number,
            description: Some("Neuer Checkout mit Wallets".into()),
            content: None,
            color: Some("#F2994A".into()),
            icon: None,
            status_id: Uuid::from_u128(0x9A05_0000_0000_7000_8000_0000_0000_0002),
            priority: 2,
            sort_order: 1000.0,
            lead_id: Some(data.alice),
            creator_id: Some(data.alice),
            start_date: None,
            start_date_resolution: None,
            target_date: None,
            target_date_resolution: None,
            started_at: Some(now),
            completed_at: None,
            canceled_at: None,
            now,
        })
        .await?;
    tx.projects().add_team(project, data.eng, 0.0, now).await?;
    tx.projects().add_member(project, data.alice, now).await?;
    tx.projects()
        .insert_milestone(&NewMilestone {
            id: seed_milestone_id(),
            project_id: project,
            name: "Alpha".into(),
            description: None,
            target_date: None,
            sort_order: 1.0,
            now,
        })
        .await?;

    // Initiative I-1 „Q4 Platform“ (aktiv, Owner Alice) mit dem Projekt.
    let n = tx.initiatives().next_number().await?;
    let initiative = seed_initiative_id();
    tx.initiatives()
        .insert(&NewInitiative {
            id: initiative,
            number: n,
            name: "Q4 Platform".into(),
            description: Some("Plattform-Stabilität und Payment".into()),
            content: None,
            color: Some("#4CB782".into()),
            icon: None,
            status: InitiativeStatus::Active,
            owner_id: Some(data.alice),
            creator_id: Some(data.alice),
            lead_team_id: None,
            priority: 1,
            sort_order: 1000.0,
            target_date: None,
            target_date_resolution: None,
            started_at: Some(now),
            completed_at: None,
            now,
        })
        .await?;
    tx.initiatives().link_project(initiative, project, now).await?;
    Ok(())
}
