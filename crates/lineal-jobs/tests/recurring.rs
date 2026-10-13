#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP30: SLA-Regeln und wiederkehrende Issues (CRUD, Rechte, Job) gegen echte Datenbank.

use chrono::{DateTime, Duration, Utc};
use lineal_app::ports::Store;
use lineal_app::ports::repos::triage::RecurringIssue;
use lineal_app::usecases::team::UpdateTeamInput;
use lineal_app::usecases::team_extra::{
    CreateRecurringInput, CreateSlaRuleInput, SetTeamSlaInput, UpdateRecurringInput,
    UpdateSlaRuleInput,
};
use lineal_app::usecases::template::SaveTemplateInput;
use lineal_app::{App, AppError, ErrorKind, RequestCtx};
use lineal_db::PgStore;
use lineal_domain::enums::{SlaType, TemplateType};
use lineal_jobs::PeriodicJob;
use lineal_jobs::periodic::recurring_issues::RecurringIssuesJob;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

struct Fx {
    app: Arc<App>,
    pool: PgPool,
}

async fn setup(pool: PgPool) -> Fx {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    Fx {
        app: test_app(Arc::new(store)),
        pool,
    }
}

fn alice() -> RequestCtx {
    ctx_for(SeedUser::Alice)
}

fn kind<T>(r: Result<T, AppError>) -> ErrorKind {
    r.err().expect("expected an error").kind
}

async fn template(fx: &Fx, name: &str, data: serde_json::Value) -> String {
    fx.app
        .save_template(
            &alice(),
            SaveTemplateInput {
                template_type: Some(TemplateType::Issue),
                name: Some(name.into()),
                team: Some("ENG".into()),
                template_data: Some(data),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .template
        .id
        .0
        .to_string()
}

fn daily(tpl: &str) -> CreateRecurringInput {
    CreateRecurringInput {
        team: "ENG".into(),
        template: tpl.into(),
        frequency: "daily".into(),
        interval: None,
        weekday: None,
        day_of_month: None,
        minute_of_day: 8 * 60,
        start_date: None,
    }
}

async fn count_issues(fx: &Fx, title: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM issue WHERE title = $1")
        .bind(title)
        .fetch_one(&fx.pool)
        .await
        .unwrap()
}

async fn by_id(fx: &Fx, id: Uuid) -> RecurringIssue {
    fx.app
        .list_recurring_issues(&alice(), "ENG")
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.id == id)
        .unwrap()
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn sla_settings_and_rules_crud_with_validation(pool: PgPool) {
    let fx = setup(pool).await;
    let sla = fx.app.get_team_sla(&alice(), "ENG").await.unwrap();
    assert!(!sla.enabled && sla.rules.is_empty());

    // Aktivieren legt die drei Standardregeln an (einmalig).
    let sla = fx
        .app
        .set_team_sla(
            &alice(),
            SetTeamSlaInput {
                team: "ENG".into(),
                enabled: Some(true),
                work_week: Some("sunToThu".into()),
            },
        )
        .await
        .unwrap();
    assert!(sla.enabled && sla.sun_thu);
    assert_eq!(sla.rules.len(), 3);
    assert!(sla.rules[2].removes_sla && sla.rules[2].duration.is_none());
    let again = fx
        .app
        .set_team_sla(
            &alice(),
            SetTeamSlaInput {
                team: "ENG".into(),
                enabled: Some(true),
                work_week: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(again.rules.len(), 3);
    assert!(again.sun_thu, "work week stays unchanged without input");
    assert_eq!(
        kind(
            fx.app
                .set_team_sla(
                    &alice(),
                    SetTeamSlaInput {
                        team: "ENG".into(),
                        enabled: None,
                        work_week: Some("weekend".into()),
                    },
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );

    let new = |name: &str, filter: serde_json::Value, duration: Option<&str>| CreateSlaRuleInput {
        team: "ENG".into(),
        name: name.into(),
        filter,
        removes_sla: false,
        duration: duration.map(Into::into),
        sla_type: SlaType::All,
        start_mode: None,
    };
    let rule = fx
        .app
        .create_sla_rule(
            &alice(),
            new(
                "Bugs",
                json!({ "labels": { "some": { "id": { "in": ["x"] } } } }),
                Some("P2D"),
            ),
        )
        .await
        .unwrap();
    assert!(rule.position > again.rules[2].position);
    assert_eq!(rule.start_mode, "issueCreation");

    // Ungültig: unbekanntes Feld, fehlende/ungültige Dauer, Name leer, Startmodus.
    for bad in [
        new("x", json!({ "title": { "contains": "a" } }), Some("P1D")),
        new("x", json!({ "priority": { "eq": 1 } }), None),
        new("x", json!({ "priority": { "eq": 1 } }), Some("soon")),
        new("x", json!({ "priority": { "eq": 1 } }), Some("-P1D")),
        new("x", json!({ "priority": { "eq": 1 } }), Some("P99999D")),
        new(" ", json!({ "priority": { "eq": 1 } }), Some("P1D")),
        CreateSlaRuleInput {
            start_mode: Some("never".into()),
            ..new("x", json!({ "priority": { "eq": 1 } }), Some("P1D"))
        },
    ] {
        assert_eq!(
            kind(fx.app.create_sla_rule(&alice(), bad).await),
            ErrorKind::InvalidRequest
        );
    }

    let upd = fx
        .app
        .update_sla_rule(
            &alice(),
            UpdateSlaRuleInput {
                team: "ENG".into(),
                id: rule.id,
                name: Some("Bugs v2".into()),
                sla_type: Some(SlaType::OnlyBusinessDays),
                duration: Some("P3D".into()),
                start_mode: Some("ruleMatch".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(upd.name, "Bugs v2");
    assert_eq!(upd.sla_type, SlaType::OnlyBusinessDays);

    // Fremde/unbekannte ID → not_found; Löschen entfernt die Regel.
    assert_eq!(
        kind(
            fx.app
                .delete_sla_rule(&alice(), "ENG", Uuid::new_v4())
                .await
        ),
        ErrorKind::NotFound
    );
    fx.app
        .delete_sla_rule(&alice(), "ENG", rule.id)
        .await
        .unwrap();
    assert_eq!(
        fx.app
            .get_team_sla(&alice(), "ENG")
            .await
            .unwrap()
            .rules
            .len(),
        3
    );

    // Rechte: Carol darf die SLA-Einstellungen nicht ändern.
    let carol = ctx_for(SeedUser::Carol);
    assert!(
        fx.app
            .set_team_sla(
                &carol,
                SetTeamSlaInput {
                    team: "ENG".into(),
                    enabled: Some(false),
                    work_week: None,
                },
            )
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn recurring_issue_job_creates_issue_once_and_advances(pool: PgPool) {
    let fx = setup(pool).await;
    let tpl = template(
        &fx,
        "Weekly report",
        json!({ "title": "Weekly report", "priority": 3, "dueIn": "P3D",
                "subIssues": [{ "title": "Collect numbers" }] }),
    )
    .await;
    let rec = fx
        .app
        .create_recurring_issue(&alice(), daily(&tpl))
        .await
        .unwrap();
    assert_eq!(rec.template_name, "Weekly report");
    assert!(!rec.paused && rec.last_run_at.is_none());
    let due: DateTime<Utc> = rec.next_run_at;

    // Vor dem Termin passiert nichts.
    RecurringIssuesJob
        .run(&fx.app, due - Duration::minutes(1))
        .await
        .unwrap();
    assert_eq!(count_issues(&fx, "Weekly report").await, 0);

    // Zum Termin: ein Issue mit Sub-Issue, ohne Ersteller, Template-Werte übernommen.
    RecurringIssuesJob.run(&fx.app, due).await.unwrap();
    assert_eq!(count_issues(&fx, "Weekly report").await, 1);
    assert_eq!(count_issues(&fx, "Collect numbers").await, 1);
    let (creator, priority, due_date): (Option<Uuid>, i32, Option<chrono::NaiveDate>) =
        sqlx::query_as(
            "SELECT creator_id, priority, due_date FROM issue WHERE title = 'Weekly report'",
        )
        .fetch_one(&fx.pool)
        .await
        .unwrap();
    assert!(creator.is_none());
    assert_eq!(priority, 3);
    assert!(due_date.is_some());
    let after = by_id(&fx, rec.id).await;
    assert_eq!(after.last_run_at, Some(due));
    assert_eq!(after.next_run_at, due + Duration::days(1));
    assert!(after.last_error.is_none());

    // Idempotent: derselbe Zeitpunkt erzeugt nichts mehr.
    RecurringIssuesJob.run(&fx.app, due).await.unwrap();
    assert_eq!(count_issues(&fx, "Weekly report").await, 1);

    // Nach drei verpassten Tagen: genau ein Issue (kein Nachholen), nächster Lauf in der Zukunft.
    let late = due + Duration::days(4) + Duration::minutes(5);
    RecurringIssuesJob.run(&fx.app, late).await.unwrap();
    assert_eq!(count_issues(&fx, "Weekly report").await, 2);
    assert!(by_id(&fx, rec.id).await.next_run_at > late);

    // Pausiert: kein Lauf; Fortsetzen berechnet den nächsten Lauf ab jetzt.
    let paused = fx
        .app
        .update_recurring_issue(
            &alice(),
            UpdateRecurringInput {
                id: rec.id,
                paused: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(paused.paused);
    RecurringIssuesJob
        .run(&fx.app, late + Duration::days(10))
        .await
        .unwrap();
    assert_eq!(count_issues(&fx, "Weekly report").await, 2);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn recurring_issue_validation_rights_and_missing_template(pool: PgPool) {
    let fx = setup(pool).await;
    let tpl = template(&fx, "Standup notes", json!({ "title": "Standup notes" })).await;

    // Validierung des Rhythmus.
    for bad in [
        CreateRecurringInput {
            frequency: "yearly".into(),
            ..daily(&tpl)
        },
        CreateRecurringInput {
            frequency: "weekly".into(),
            ..daily(&tpl)
        },
        CreateRecurringInput {
            frequency: "weekly".into(),
            weekday: Some(8),
            ..daily(&tpl)
        },
        CreateRecurringInput {
            frequency: "monthly".into(),
            ..daily(&tpl)
        },
        CreateRecurringInput {
            interval: Some(0),
            ..daily(&tpl)
        },
        CreateRecurringInput {
            minute_of_day: 1440,
            ..daily(&tpl)
        },
        daily("00000000-0000-0000-0000-000000000000"),
    ] {
        assert!(fx.app.create_recurring_issue(&alice(), bad).await.is_err());
    }
    // Rechte: Carol darf keine Wiederholung anlegen.
    assert!(
        fx.app
            .create_recurring_issue(&ctx_for(SeedUser::Carol), daily(&tpl))
            .await
            .is_err()
    );

    let weekly = fx
        .app
        .create_recurring_issue(
            &alice(),
            CreateRecurringInput {
                frequency: "weekly".into(),
                weekday: Some(1),
                ..daily(&tpl)
            },
        )
        .await
        .unwrap();
    let monthly = fx
        .app
        .update_recurring_issue(
            &alice(),
            UpdateRecurringInput {
                id: weekly.id,
                frequency: Some("monthly".into()),
                day_of_month: Some(31),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!((monthly.weekday, monthly.day_of_month), (None, Some(31)));

    // Ein Template mit aktiven Regeln lässt sich nicht löschen (B7.09) ...
    assert_eq!(
        kind(fx.app.delete_template(&alice(), &tpl).await),
        ErrorKind::Conflict
    );
    // ... ist es trotzdem weg (hier direkt in der Datenbank archiviert), pausiert der Job die
    // Regel mit Fehlermeldung, statt bei jedem Lauf still zu scheitern.
    sqlx::query("UPDATE template SET archived_at = now() WHERE id = $1::uuid")
        .bind(&tpl)
        .execute(&fx.pool)
        .await
        .unwrap();
    let created = fx.app.run_recurring_issues(monthly.next_run_at).await.unwrap();
    assert_eq!(created, 0, "a skipped rule does not count as created");
    assert_eq!(count_issues(&fx, "Standup notes").await, 0);
    let after = by_id(&fx, weekly.id).await;
    assert!(after.last_error.is_some());
    assert!(after.paused, "a rule without template is paused");

    // Löschen; fremde ID → not_found.
    fx.app
        .delete_recurring_issue(&alice(), weekly.id)
        .await
        .unwrap();
    assert_eq!(
        kind(fx.app.delete_recurring_issue(&alice(), weekly.id).await),
        ErrorKind::NotFound
    );
    assert!(
        fx.app
            .list_recurring_issues(&alice(), "ENG")
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn auto_close_state_can_be_reset_to_default(pool: PgPool) {
    let fx = setup(pool).await;
    let canceled: Uuid = sqlx::query_scalar(
        "SELECT s.id FROM workflow_state s JOIN team t ON t.id = s.team_id \
         WHERE t.key = 'ENG' AND s.type = 'canceled' ORDER BY s.position LIMIT 1",
    )
    .fetch_one(&fx.pool)
    .await
    .unwrap();
    let set = |state: String| UpdateTeamInput {
        auto_close_state: Some(state),
        ..Default::default()
    };
    let t = fx
        .app
        .update_team(&alice(), "ENG", set(canceled.to_string()))
        .await
        .unwrap();
    assert_eq!(t.team.auto_close_state_id.map(|s| s.0), Some(canceled));
    let t = fx
        .app
        .update_team(&alice(), "ENG", set(String::new()))
        .await
        .unwrap();
    assert!(t.team.auto_close_state_id.is_none());
}
