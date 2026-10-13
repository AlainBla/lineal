#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP11: firmenweite Sicht (docs/14) gegen echte Datenbank – Abnahmekriterien 1–3 aus §6 sowie
//! Rollups, Auslastung, aktive Cycles und die Risikoliste.

mod wp11_common;

use lineal_app::ErrorKind;
use lineal_app::usecases::overview::{OverviewInput, OverviewPeriod};
use lineal_testkit::seed::planning::seed_cycle_id;
use sqlx::PgPool;
use uuid::Uuid;
use wp11_common::*;

/// ENG: 3 offen (davon 1 in Arbeit, 1 überfällig), 1 erledigt (2026-10-08), Cycle 2.
/// WEB: 2 offen (davon 1 in Triage).
/// DES (privat): 4 offen, 1 überfällig, 1 SLA verletzt, 1 Projekt.
async fn fixture(fx: &Fx) {
    let d = fx.data;
    let a = fx.issue(d.eng, 101, "Eng todo", 1).await;
    fx.set(
        a,
        &format!(
            "assignee_id = '{}', cycle_id = '{}', due_date = '2026-10-01', estimate = 2, added_to_cycle_at = '2026-10-06T00:00:00Z'",
            d.bob.0,
            seed_cycle_id(2).0
        ),
    )
    .await;
    let b = fx.issue(d.eng, 102, "Eng started", 2).await;
    fx.set(
        b,
        &format!(
            "assignee_id = '{}', cycle_id = '{}', estimate = 3",
            d.bob.0,
            seed_cycle_id(2).0
        ),
    )
    .await;
    fx.issue(d.eng, 103, "Eng backlog", 0).await;
    let done = fx.issue(d.eng, 104, "Eng done", 3).await;
    fx.set(done, "completed_at = '2026-10-08T10:00:00Z'").await;
    fx.issue(d.web, 101, "Web todo", 1).await;
    let triage: Uuid = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO workflow_state (id, organization_id, team_id, name, type, color, position) \
         VALUES ($1,$2,$3,'Triage','triage','#CCCCCC',0)",
    )
    .bind(triage)
    .bind(d.org.0)
    .bind(d.web.0)
    .execute(&fx.pool)
    .await
    .unwrap();
    let t = fx.issue(d.web, 102, "Web triage", 0).await;
    fx.set(
        t,
        &format!("state_id = '{triage}', created_at = '2026-10-02T00:00:00Z'"),
    )
    .await;
    for n in 0..4 {
        let i = fx.issue(d.des, 101 + n, &format!("Secret {n}"), 1).await;
        if n == 0 {
            fx.set(i, "due_date = '2026-09-01'").await;
        }
        if n == 1 {
            fx.set(i, "sla_breaches_at = '2026-10-01T00:00:00Z'").await;
        }
    }
    // Privates Projekt in DES.
    let status: Uuid = sqlx::query_scalar(
        "SELECT id FROM project_status WHERE organization_id = $1 AND type = 'started'",
    )
    .bind(d.org.0)
    .fetch_one(&fx.pool)
    .await
    .unwrap();
    let p = Uuid::now_v7();
    sqlx::query("INSERT INTO project (id, organization_id, name, lead_team_id, number, status_id, health) VALUES ($1,$2,'Secret',$3,1,$4,'offTrack')")
        .bind(p)
        .bind(d.org.0)
        .bind(d.des.0)
        .bind(status)
        .execute(&fx.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO project_team (organization_id, project_id, team_id) VALUES ($1,$2,$3)",
    )
    .bind(d.org.0)
    .bind(p)
    .bind(d.des.0)
    .execute(&fx.pool)
    .await
    .unwrap();
}

fn row<'a>(
    o: &'a lineal_app::usecases::overview::WorkspaceOverview,
    key: &str,
) -> Option<&'a lineal_app::usecases::overview::TeamMetrics> {
    o.teams.iter().find(|t| t.key == key)
}

#[sqlx::test(migrations = "./migrations")]
async fn private_teams_are_absent_from_rows_and_sums(pool: PgPool) {
    let fx = setup(pool).await;
    fixture(&fx).await;
    // Kriterium 1: Alice (nicht in DES) sieht DES weder als Zeile noch in Summen.
    let o = fx
        .app
        .workspace_overview(&alice(), OverviewInput::default())
        .await
        .unwrap();
    assert!(row(&o, "DES").is_none());
    assert_eq!(
        o.teams.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(),
        ["ENG", "WEB"]
    );
    // 3 (ENG) + 2 (WEB) offen; erledigtes Issue zählt nicht als offen.
    assert_eq!(o.totals.open_issues, 5);
    assert_eq!(o.totals.overdue, 1);
    assert_eq!(o.totals.sla_breached, 0);
    assert_eq!(
        o.projects.total(),
        1,
        "only the seed project P-ENG-1; the private one is not counted"
    );
    assert_eq!(o.projects.off_track, 0);
    assert!(
        o.at_risk.iter().all(|i| !i.identifier.starts_with("DES-")),
        "{:?}",
        o.at_risk
    );
    // Bob (Mitglied von DES) sieht alles.
    let b = fx
        .app
        .workspace_overview(&bob(), OverviewInput::default())
        .await
        .unwrap();
    assert!(row(&b, "DES").is_some());
    assert_eq!(b.totals.open_issues, 9);
    assert_eq!(b.totals.sla_breached, 1);
    assert_eq!((b.projects.total(), b.projects.off_track), (2, 1));
    assert!(b.at_risk.iter().any(|i| i.identifier == "DES-102"));
    // Ein explizit genanntes privates Team ist für Alice nicht auffindbar (kein Unterschied zu
    // einem nicht vorhandenen Team).
    for t in ["DES", &fx.data.des.to_string()] {
        let e = fx
            .app
            .workspace_overview(
                &alice(),
                OverviewInput {
                    teams: vec![t.to_owned()],
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::NotFound);
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn rollup_over_sub_teams_with_and_without(pool: PgPool) {
    let fx = setup(pool).await;
    fixture(&fx).await;
    // Kriterium 2: mit „inkl. Sub-Teams“ enthält die ENG-Kachel die Zahlen von WEB, ohne nicht.
    let with = fx
        .app
        .workspace_overview(
            &alice(),
            OverviewInput {
                include_sub_teams: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let without = fx
        .app
        .workspace_overview(
            &alice(),
            OverviewInput {
                include_sub_teams: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(row(&with, "ENG").unwrap().counts.open_issues, 5);
    assert_eq!(row(&without, "ENG").unwrap().counts.open_issues, 3);
    assert_eq!(
        row(&with, "WEB").unwrap().counts.open_issues,
        2,
        "children keep their own rows"
    );
    assert_eq!(row(&without, "WEB").unwrap().counts.open_issues, 2);
    assert_eq!(row(&with, "ENG").unwrap().counts.triage, 1);
    assert_eq!(row(&without, "ENG").unwrap().counts.triage, 0);
    assert_eq!(row(&with, "ENG").unwrap().counts.in_progress, 1);
    assert_eq!(row(&with, "WEB").unwrap().depth, 1);
    assert_eq!(row(&with, "ENG").unwrap().depth, 0);
    assert!(row(&with, "ENG").unwrap().counts.triage_oldest_at.is_some());
    // Erledigt im Zeitraum (7 Tage ab 2026-10-10): „Eng done“ vom 10-08; 30d identisch hier.
    assert_eq!(row(&without, "ENG").unwrap().counts.completed, 1);
    let d30 = fx
        .app
        .workspace_overview(
            &alice(),
            OverviewInput {
                period: OverviewPeriod::Days30,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(d30.totals.completed, 1);
    // Durchsatz: Woche 0.
    assert_eq!(row(&without, "ENG").unwrap().throughput[0].issues, 1);
    // Totals zählen jedes Issue genau einmal (kein Doppelzählen durch Rollup).
    assert_eq!(with.totals.open_issues, 5);
    // Cycle-Fortschritt des aktiven ENG-Cycles (notUsed: 1 Punkt je Issue): (0 + 0.25*1)/2.
    let p = row(&with, "ENG").unwrap().cycle_progress.unwrap();
    assert!((p - 0.125).abs() < 1e-9, "{p}");
    // Teamauswahl.
    let only = fx
        .app
        .workspace_overview(
            &alice(),
            OverviewInput {
                teams: vec!["WEB".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(only.teams.len(), 1);
    assert_eq!(only.totals.open_issues, 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn guests_are_forbidden_everywhere(pool: PgPool) {
    let fx = setup(pool).await;
    fixture(&fx).await;
    // Kriterium 3.
    assert_eq!(
        kind(
            fx.app
                .workspace_overview(&carol(), OverviewInput::default())
                .await
        ),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(fx.app.workload(&carol(), Vec::new(), false).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(fx.app.active_cycles(&carol(), Vec::new()).await),
        ErrorKind::Forbidden
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn workload_and_active_cycles(pool: PgPool) {
    let fx = setup(pool).await;
    fixture(&fx).await;
    let w = fx.app.workload(&alice(), Vec::new(), false).await.unwrap();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].display_name, "bob");
    assert_eq!(w[0].per_team.len(), 1);
    assert_eq!(
        (w[0].per_team[0].open_issues, w[0].per_team[0].points),
        (2, 2.0)
    );
    // Estimation notUsed (Seed-Team) -> jeder Punkt = 1.
    // (Das Seed-Team ENG hat `notUsed`; Punkte = Anzahl.)
    assert!(w[0].total_open_issues == 2);
    // Nächster Cycle: leer.
    assert!(
        fx.app
            .workload(&alice(), Vec::new(), true)
            .await
            .unwrap()
            .is_empty()
    );
    // Bobs private Team-Issues erscheinen nicht in Alices Matrix, aber auch nicht ohne Cycle.
    let cycles = fx.app.active_cycles(&alice(), Vec::new()).await.unwrap();
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0].team_key, "ENG");
    assert_eq!(cycles[0].number, 2);
    assert_eq!(
        cycles[0].scope_change, 1,
        "one issue was added after the cycle started"
    );
    assert!(cycles[0].remaining_days >= 8 && cycles[0].remaining_days <= 9);
    assert_eq!(
        kind(fx.app.active_cycles(&alice(), vec!["DES".into()]).await),
        ErrorKind::NotFound
    );
}

/// B5.25: veraltete Health-Werte laufender Projekte zählen als „no update“.
#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn stale_health_counts_as_no_update(pool: PgPool) {
    let fx = setup(pool).await;
    let d = fx.data;
    let status: Uuid = sqlx::query_scalar(
        "SELECT id FROM project_status WHERE organization_id = $1 AND type = 'started' AND NOT indefinite LIMIT 1",
    )
    .bind(d.org.0)
    .fetch_one(&fx.pool)
    .await
    .unwrap();
    let before = fx
        .app
        .workspace_overview(&alice(), OverviewInput::default())
        .await
        .unwrap()
        .projects;
    for (n, updated) in [(901, "2026-10-09T00:00:00Z"), (902, "2020-01-01T00:00:00Z")] {
        let p = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO project (id, organization_id, name, lead_team_id, number, status_id, health, \
             health_updated_at, update_reminder_mode, update_reminder_frequency_in_weeks) \
             VALUES ($1,$2,$3,$4,$5,$6,'onTrack',$7::timestamptz,'custom',1)",
        )
        .bind(p)
        .bind(d.org.0)
        .bind(format!("P{n}"))
        .bind(d.eng.0)
        .bind(n)
        .bind(status)
        .bind(updated)
        .execute(&fx.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO project_team (organization_id, project_id, team_id) VALUES ($1,$2,$3)",
        )
        .bind(d.org.0)
        .bind(p)
        .bind(d.eng.0)
        .execute(&fx.pool)
        .await
        .unwrap();
    }
    let after = fx
        .app
        .workspace_overview(&alice(), OverviewInput::default())
        .await
        .unwrap()
        .projects;
    assert_eq!(after.on_track, before.on_track + 1, "frisches Update bleibt onTrack");
    assert_eq!(after.no_update, before.no_update + 1, "veraltetes zählt als no update");
}
