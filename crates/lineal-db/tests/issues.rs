#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP06: Issues-Kern gegen echte Datenbank (Seed `acme`): Anlage mit Zähler, Statuswechsel,
//! Patch, Labels, Zuordnung, Sub-Issues, Relationen, History, Teamwechsel, Papierkorb, Sharing.

use chrono::{DateTime, Duration, TimeZone, Utc};
use lineal_app::ports::repos::issue::HistorySource;
use lineal_app::ports::repos::label::{LabelKind, NewLabel};
use lineal_app::ports::repos::team::TeamPatch;
use lineal_app::ports::{Clock, Store};
use lineal_app::usecases::issue::{
    CreateIssueInput, DescriptionEdit, ListIssuesInput, RelationChanges, UpdateIssueInput,
};
use lineal_app::{App, ErrorKind, RequestCtx};
use lineal_db::PgStore;
use lineal_domain::enums::{EstimationType, LabelGroupType, StateType};
use lineal_domain::ids::LabelId;
use lineal_domain::rules::patch::PatchOp;
use lineal_testkit::seed::identity::seed_workflow_state_ids;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Steuerbare Uhr.
struct TestClock(Mutex<DateTime<Utc>>);

impl TestClock {
    fn advance(&self, secs: i64) {
        let mut t = self.0.lock().unwrap();
        *t += Duration::seconds(secs);
    }
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

struct Fx {
    app: Arc<App>,
    data: SeedData,
    pool: PgPool,
    clock: Arc<TestClock>,
}

async fn setup(pool: PgPool) -> Fx {
    let store = PgStore::new(pool.clone());
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let clock = Arc::new(TestClock(Mutex::new(
        Utc.with_ymd_and_hms(2026, 10, 10, 7, 15, 0).unwrap(),
    )));
    let mut app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    app.clock = clock.clone();
    Fx {
        app: Arc::new(app),
        data,
        pool,
        clock,
    }
}

fn kind<T>(r: Result<T, lineal_app::AppError>) -> ErrorKind {
    r.err().expect("expected an error").kind
}

fn create(team: &str, title: &str) -> CreateIssueInput {
    CreateIssueInput {
        team: team.into(),
        title: Some(title.into()),
        ..Default::default()
    }
}

fn update(id: &str) -> UpdateIssueInput {
    UpdateIssueInput {
        id: id.into(),
        ..Default::default()
    }
}

fn alice() -> RequestCtx {
    ctx_for(SeedUser::Alice)
}
fn bob() -> RequestCtx {
    ctx_for(SeedUser::Bob)
}

async fn label(
    fx: &Fx,
    name: &str,
    team: Option<lineal_domain::ids::TeamId>,
    parent: Option<LabelId>,
    group: Option<LabelGroupType>,
) -> LabelId {
    let mut tx = fx.app.store.begin(fx.data.org).await.unwrap();
    let id = LabelId(Uuid::new_v4());
    tx.labels()
        .insert(
            LabelKind::Issue,
            &NewLabel {
                id,
                name: name.into(),
                color: "#112233".into(),
                description: None,
                team_id: team,
                parent_id: parent,
                is_group: group.is_some(),
                group_type: group,
                creator_id: Some(fx.data.alice),
                now: Utc::now(),
            },
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    id
}

#[sqlx::test(migrations = "./migrations")]
async fn create_assigns_numbers_default_state_and_sort_order(pool: PgPool) {
    let fx = setup(pool).await;
    let states = seed_workflow_state_ids(1);
    let a = fx
        .app
        .create_issue(&alice(), create("ENG", "Erstes"))
        .await
        .unwrap();
    assert_eq!(a.issue.number, 1);
    assert_eq!(a.issue.identifier, "ENG-1");
    assert_eq!(a.issue.state_id, states[0], "default state = Backlog");
    assert_eq!(a.issue.state_type, StateType::Backlog);
    assert_eq!(a.issue.creator_id, Some(fx.data.alice));
    assert_eq!(a.extras.state_name, "Backlog");
    let b = fx
        .app
        .create_issue(&alice(), create("eng", "Zweites"))
        .await
        .unwrap();
    assert_eq!(b.issue.identifier, "ENG-2");
    assert!(
        b.issue.sort_order < a.issue.sort_order,
        "neue Issues stehen oben"
    );
    // Titel ist Pflicht und wird geprüft
    assert_eq!(
        kind(
            fx.app
                .create_issue(
                    &alice(),
                    CreateIssueInput {
                        team: "ENG".into(),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            fx.app
                .create_issue(&alice(), create("ENG", &"x".repeat(256)))
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // fehlgeschlagene Anlage verbraucht keine Nummer
    let c = fx
        .app
        .create_issue(&alice(), create("ENG", "Drittes"))
        .await
        .unwrap();
    assert_eq!(c.issue.number, 3);
}

#[sqlx::test(migrations = "./migrations")]
async fn twenty_concurrent_creates_get_gapless_unique_numbers(pool: PgPool) {
    let fx = setup(pool).await;
    let mut tasks = Vec::new();
    for i in 0..20 {
        let app = fx.app.clone();
        tasks.push(tokio::spawn(async move {
            app.create_issue(&alice(), create("ENG", &format!("Parallel {i}")))
                .await
                .unwrap()
                .issue
                .number
        }));
    }
    let mut numbers = Vec::new();
    for t in tasks {
        numbers.push(t.await.unwrap());
    }
    numbers.sort_unstable();
    assert_eq!(numbers, (1..=20).collect::<Vec<_>>());
    let distinct: i64 = sqlx::query_scalar("SELECT count(DISTINCT identifier) FROM issue")
        .fetch_one(&fx.pool)
        .await
        .unwrap();
    assert_eq!(distinct, 20);
    let counter: i32 = sqlx::query_scalar("SELECT last_issue_number FROM team WHERE id = $1")
        .bind(fx.data.eng.0)
        .fetch_one(&fx.pool)
        .await
        .unwrap();
    assert_eq!(counter, 20);
}

#[sqlx::test(migrations = "./migrations")]
async fn issues_of_private_teams_are_not_found(pool: PgPool) {
    let fx = setup(pool).await;
    let des = fx
        .app
        .create_issue(&bob(), create("DES", "Logo"))
        .await
        .unwrap();
    assert_eq!(des.issue.identifier, "DES-1");
    // Alice ist Owner, aber nicht Mitglied von DES.
    assert_eq!(
        kind(fx.app.get_issue(&alice(), "DES-1").await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(fx.app.get_issue(&alice(), &des.issue.id.to_string()).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(fx.app.get_issue(&alice(), "des-1").await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        title: Some("x".into()),
                        ..update("DES-1")
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(fx.app.create_issue(&alice(), create("DES", "x")).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(fx.app.delete_issue(&alice(), "DES-1", false).await),
        ErrorKind::NotFound
    );
    let page = fx
        .app
        .list_issues(
            &alice(),
            ListIssuesInput {
                limit: 50,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(page.items.is_empty());
    // Team-Filter auf ein privates Team: ebenfalls not_found
    assert_eq!(
        kind(
            fx.app
                .list_issues(
                    &alice(),
                    ListIssuesInput {
                        team: Some("DES".into()),
                        limit: 50,
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
    // Bob sieht es.
    assert_eq!(
        fx.app
            .get_issue(&bob(), "DES-1")
            .await
            .unwrap()
            .view
            .issue
            .title,
        "Logo"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn sub_team_uses_its_own_copy_of_the_default_state(pool: PgPool) {
    let fx = setup(pool).await;
    let web = fx
        .app
        .create_issue(&bob(), create("WEB", "Navigation"))
        .await
        .unwrap();
    let web_states = seed_workflow_state_ids(2);
    assert_eq!(web.issue.state_id, web_states[0]);
    let inherited: Option<Uuid> =
        sqlx::query_scalar("SELECT inherited_from_id FROM workflow_state WHERE id = $1")
            .bind(web.issue.state_id.0)
            .fetch_one(&fx.pool)
            .await
            .unwrap();
    assert!(inherited.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn state_changes_maintain_timestamps_and_forbid_direct_duplicate(pool: PgPool) {
    let fx = setup(pool).await;
    let s = seed_workflow_state_ids(1);
    let i = fx
        .app
        .create_issue(&alice(), create("ENG", "Zeit"))
        .await
        .unwrap();
    assert!(i.issue.started_at.is_none());
    fx.clock.advance(10);
    let started = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                state: Some("In Progress".into()),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    assert_eq!(started.issue.state_id, s[2]);
    assert!(started.issue.started_at.is_some() && started.issue.completed_at.is_none());
    fx.clock.advance(10);
    let done = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                state: Some("completed".into()),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    assert_eq!(done.issue.state_id, s[3]);
    assert!(done.issue.completed_at.is_some());
    assert_eq!(
        done.issue.started_at, started.issue.started_at,
        "startedAt bleibt"
    );
    fx.clock.advance(10);
    let canceled = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                state: Some("Canceled".into()),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    assert!(canceled.issue.completed_at.is_none() && canceled.issue.canceled_at.is_some());
    let reopened = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                state: Some("Todo".into()),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    assert!(reopened.issue.canceled_at.is_none() && reopened.issue.started_at.is_none());
    // duplicate nur über Markieren
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        state: Some("Duplicate".into()),
                        ..update("ENG-1")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            fx.app
                .create_issue(
                    &alice(),
                    CreateIssueInput {
                        state: Some("Duplicate".into()),
                        ..create("ENG", "x")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // unbekannter Status → not_found
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        state: Some("Nope".into()),
                        ..update("ENG-1")
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn priority_estimate_assignee_and_tri_state(pool: PgPool) {
    let fx = setup(pool).await;
    {
        let mut tx = fx.app.store.begin(fx.data.org).await.unwrap();
        tx.teams()
            .update(
                fx.data.eng,
                &TeamPatch {
                    issue_estimation_type: Some(EstimationType::Fibonacci),
                    ..Default::default()
                },
                Utc::now(),
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    let i = fx
        .app
        .create_issue(
            &alice(),
            CreateIssueInput {
                priority: Some(2),
                estimate: Some(5.0),
                assignee: Some("bob@acme.example".into()),
                due_date: Some(chrono::NaiveDate::from_ymd_opt(2026, 10, 17).unwrap()),
                ..create("ENG", "Felder")
            },
        )
        .await
        .unwrap();
    assert_eq!(i.issue.priority.value(), 2);
    assert_eq!(i.issue.estimate, Some(5));
    assert_eq!(i.issue.assignee_id, Some(fx.data.bob));
    assert_eq!(i.extras.assignee_name.as_deref(), Some("Bob Example"));
    // ungültige Schätzung: details.allowed
    let err = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                estimate: Some(Some(4.0)),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert!(err.details["allowed"].as_array().unwrap().len() >= 5);
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        priority: Some(7),
                        ..update("ENG-1")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // Tri-State: null leert, weggelassen bleibt
    let cleared = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                estimate: Some(None),
                assignee: Some(None),
                due_date: Some(None),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    assert_eq!(cleared.issue.estimate, None);
    assert_eq!(cleared.issue.assignee_id, None);
    assert_eq!(cleared.issue.due_date, None);
    assert_eq!(
        cleared.issue.priority.value(),
        2,
        "weggelassen = unverändert"
    );
    // Carol (Gast, nur WEB) ist in ENG nicht zuweisbar
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        assignee: Some(Some("carol".into())),
                        ..update("ENG-1")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // Update ohne Änderung: keine Versionserhöhung
    let before = fx
        .app
        .get_issue(&alice(), "ENG-1")
        .await
        .unwrap()
        .view
        .issue;
    let same = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                priority: Some(2),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    assert_eq!(same.issue.version, before.version);
}

#[sqlx::test(migrations = "./migrations")]
async fn patch_with_ambiguous_anchor_conflicts_and_leaves_text_unchanged(pool: PgPool) {
    let fx = setup(pool).await;
    fx.app
        .create_issue(
            &alice(),
            CreateIssueInput {
                description: Some("alpha beta alpha".into()),
                ..create("ENG", "Patch")
            },
        )
        .await
        .unwrap();
    let ops = vec![
        PatchOp::Append {
            text: " gamma".into(),
        },
        PatchOp::Replace {
            old_string: "alpha".into(),
            new_string: "omega".into(),
            replace_all: None,
        },
    ];
    let err = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                title: Some("Neuer Titel".into()),
                description: Some(DescriptionEdit::Patch(ops)),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Conflict);
    assert_eq!(err.details["opIndex"], 1);
    assert_eq!(err.details["matches"], 2);
    let issue = fx
        .app
        .get_issue(&alice(), "ENG-1")
        .await
        .unwrap()
        .view
        .issue;
    assert_eq!(issue.description.as_deref(), Some("alpha beta alpha"));
    assert_eq!(issue.title, "Patch", "atomar: auch der Titel bleibt");
    // eindeutiger Anker funktioniert
    let ok = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                description: Some(DescriptionEdit::Patch(vec![PatchOp::Replace {
                    old_string: "beta".into(),
                    new_string: "BETA".into(),
                    replace_all: None,
                }])),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    assert_eq!(ok.issue.description.as_deref(), Some("alpha BETA alpha"));
    // Größenlimit → limit_exceeded
    let big = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                description: Some(DescriptionEdit::Set("x".repeat(500_001))),
                ..update("ENG-1")
            },
        )
        .await;
    assert_eq!(kind(big), ErrorKind::LimitExceeded);
}

#[sqlx::test(migrations = "./migrations")]
async fn single_select_label_groups(pool: PgPool) {
    let fx = setup(pool).await;
    let group = label(
        &fx,
        "Platform",
        None,
        None,
        Some(LabelGroupType::SingleSelect),
    )
    .await;
    let ios = label(&fx, "iOS", None, Some(group), None).await;
    let android = label(&fx, "Android", None, Some(group), None).await;
    let multi = label(&fx, "Area", None, None, Some(LabelGroupType::MultiSelect)).await;
    let a1 = label(&fx, "A1", None, Some(multi), None).await;
    let a2 = label(&fx, "A2", None, Some(multi), None).await;
    let _ = (ios, android, a1, a2);
    let i = fx
        .app
        .create_issue(
            &alice(),
            CreateIssueInput {
                labels: Some(vec!["Bug".into(), "Platform/iOS".into()]),
                ..create("ENG", "Labels")
            },
        )
        .await
        .unwrap();
    assert_eq!(i.extras.label_names, vec!["Bug", "iOS"]);
    fx.clock.advance(600);
    // addLabels ersetzt das Geschwister der singleSelect-Gruppe
    let swapped = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                add_labels: vec!["Android".into()],
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    assert_eq!(swapped.extras.label_names, vec!["Android", "Bug"]);
    let history = fx.app.issue_history(&alice(), "ENG-1").await.unwrap();
    assert!(history.iter().any(|h| {
        h.removed_label_ids
            .as_ref()
            .is_some_and(|r| r.contains(&ios))
    }));
    // ersetzendes labels mit zwei Geschwistern: Fehler
    let err = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                labels: Some(vec!["iOS".into(), "Android".into()]),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(err.details["reason"], "single_select_violation");
    // multiSelect erlaubt mehrere
    let multi_ok = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                add_labels: vec!["A1".into(), "A2".into()],
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    assert_eq!(multi_ok.extras.label_names.len(), 4);
    // Gruppe nicht direkt anwendbar, labels + addLabels zusammen nicht erlaubt
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        add_labels: vec!["Platform".into()],
                        ..update("ENG-1")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        labels: Some(vec![]),
                        add_labels: vec!["Bug".into()],
                        ..update("ENG-1")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // removeLabels, Label unbekannt → not_found
    let removed = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                remove_labels: vec!["Bug".into()],
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    assert!(!removed.extras.label_names.contains(&"Bug".to_string()));
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        add_labels: vec!["gibt-es-nicht".into()],
                        ..update("ENG-1")
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
    // Team-Label eines unverwandten Teams ist nicht verfügbar
    let des_label = label(&fx, "Brand", Some(fx.data.des), None, None).await;
    let _ = des_label;
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        add_labels: vec!["Brand".into()],
                        ..update("ENG-1")
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn labels_of_parent_teams_apply_to_sub_teams(pool: PgPool) {
    let fx = setup(pool).await;
    label(&fx, "Frontend", Some(fx.data.eng), None, None).await;
    let web = fx
        .app
        .create_issue(
            &bob(),
            CreateIssueInput {
                labels: Some(vec!["Frontend".into()]),
                ..create("WEB", "Vererbt")
            },
        )
        .await
        .unwrap();
    assert_eq!(web.extras.label_names, vec!["Frontend"]);
    // retired Labels sind nicht neu vergebbar
    let retired = label(&fx, "Alt", None, None, None).await;
    sqlx::query("UPDATE issue_label SET retired_at = now() WHERE id = $1")
        .bind(retired.0)
        .execute(&fx.pool)
        .await
        .unwrap();
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &bob(),
                    UpdateIssueInput {
                        add_labels: vec!["Alt".into()],
                        ..update("WEB-1")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
}

async fn exec(fx: &Fx, sql: &str, binds: &[Uuid]) {
    let mut q = sqlx::query(sql);
    for b in binds {
        q = q.bind(*b);
    }
    q.execute(&fx.pool).await.unwrap();
}

/// Legt Cycle für `team`, Projekt (mit den Teams) und Meilenstein an; liefert (cycle, project, milestone).
async fn planning(fx: &Fx, team: Uuid, project_teams: &[Uuid]) -> (Uuid, Uuid, Uuid) {
    let (cycle, project, milestone) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let org = fx.data.org.0;
    sqlx::query(
        "INSERT INTO cycle (id, organization_id, team_id, number, name, starts_at, ends_at) \
         VALUES ($1, $2, $3, 1, 'Sprint 1', $4, $5)",
    )
    .bind(cycle)
    .bind(org)
    .bind(team)
    .bind(fx.clock.now() - Duration::days(1))
    .bind(fx.clock.now() + Duration::days(13))
    .execute(&fx.pool)
    .await
    .unwrap();
    let status: Uuid = sqlx::query_scalar("SELECT id FROM project_status LIMIT 1")
        .fetch_one(&fx.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO project (id, organization_id, name, lead_team_id, number, status_id) \
         VALUES ($1, $2, 'Checkout v2', $3, 1, $4)",
    )
    .bind(project)
    .bind(org)
    .bind(project_teams[0])
    .bind(status)
    .execute(&fx.pool)
    .await
    .unwrap();
    for t in project_teams {
        exec(
            fx,
            &format!("INSERT INTO project_team (organization_id, project_id, team_id) VALUES ('{org}', $1, $2)"),
            &[project, *t],
        )
        .await;
    }
    exec(
        fx,
        &format!("INSERT INTO project_milestone (id, organization_id, project_id, name) VALUES ($1, '{org}', $2, 'Beta')"),
        &[milestone, project],
    )
    .await;
    (cycle, project, milestone)
}

#[sqlx::test(migrations = "./migrations")]
async fn cycle_project_milestone_assignment_is_validated(pool: PgPool) {
    let fx = setup(pool).await;
    let (cycle, project, milestone) = planning(&fx, fx.data.eng.0, &[fx.data.eng.0]).await;
    let (web_cycle, _, _) = {
        // Cycle eines anderen Teams
        let c = Uuid::new_v4();
        exec(&fx, &format!(
            "INSERT INTO cycle (id, organization_id, team_id, number, starts_at, ends_at) VALUES ($1, '{}', '{}', 1, now() + interval '30 days', now() + interval '40 days')",
            fx.data.org.0, fx.data.web.0), &[c]).await;
        (c, (), ())
    };
    fx.app
        .create_issue(&alice(), create("ENG", "Zuordnung"))
        .await
        .unwrap();
    // Cycle des falschen Teams
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        cycle: Some(Some(web_cycle.to_string())),
                        ..update("ENG-1")
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
    // Meilenstein ohne Projekt
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        milestone: Some(Some("Beta".into())),
                        ..update("ENG-1")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // alles korrekt; aktiver Cycle schiebt Backlog → Todo
    let ok = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                cycle: Some(Some("Sprint 1".into())),
                project: Some(Some("P-ENG-1".into())),
                milestone: Some(Some(milestone.to_string())),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    assert_eq!(ok.issue.cycle_id.map(|c| c.0), Some(cycle));
    assert_eq!(ok.issue.project_id.map(|p| p.0), Some(project));
    assert_eq!(ok.issue.project_milestone_id.map(|m| m.0), Some(milestone));
    assert_eq!(ok.issue.state_type, StateType::Unstarted);
    assert!(ok.issue.added_to_cycle_at.is_some() && ok.issue.added_to_project_at.is_some());
    assert_eq!(ok.extras.project_identifier.as_deref(), Some("P-ENG-1"));
    // Projektwechsel leert den Meilenstein; Projekt entfernen ebenso
    let cleared = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                project: Some(None),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    assert!(cleared.issue.project_id.is_none() && cleared.issue.project_milestone_id.is_none());
    // Projekt ohne das Team des Issues
    let (_, _, _) = (0, 0, 0);
    fx.app
        .create_issue(&bob(), create("WEB", "Web-Issue"))
        .await
        .unwrap();
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &bob(),
                    UpdateIssueInput {
                        project: Some(Some("Checkout v2".into())),
                        ..update("WEB-1")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // list_issues-Filter nach Projekt/Cycle
    fx.app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                project: Some(Some("Checkout v2".into())),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    let by_project = fx
        .app
        .list_issues(
            &alice(),
            ListIssuesInput {
                project: Some("P-ENG-1".into()),
                limit: 50,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(by_project.items.len(), 1);
    let by_cycle = fx
        .app
        .list_issues(
            &alice(),
            ListIssuesInput {
                cycle: Some("Sprint 1".into()),
                team: Some("ENG".into()),
                limit: 50,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(by_cycle.items.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn sub_issues_forbid_cycles_and_inherit(pool: PgPool) {
    let fx = setup(pool).await;
    let a = fx
        .app
        .create_issue(
            &alice(),
            CreateIssueInput {
                priority: Some(1),
                ..create("ENG", "Parent")
            },
        )
        .await
        .unwrap();
    let b = fx
        .app
        .create_issue(
            &alice(),
            CreateIssueInput {
                parent: Some("ENG-1".into()),
                ..create("ENG", "Child")
            },
        )
        .await
        .unwrap();
    assert_eq!(b.issue.parent_id, Some(a.issue.id));
    assert_eq!(b.issue.priority.value(), 1, "Priorität vom Parent");
    assert_eq!(b.extras.parent_identifier.as_deref(), Some("ENG-1"));
    assert_eq!(b.issue.sub_issue_sort_order, Some(1.0));
    // Zyklus: A unter B
    let err = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                parent: Some(Some("ENG-2".into())),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(err.details["reason"], "parent_cycle");
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        parent: Some(Some("ENG-1".into())),
                        ..update("ENG-1")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // Parent entfernen
    let free = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                parent: Some(None),
                ..update("ENG-2")
            },
        )
        .await
        .unwrap();
    assert!(free.issue.parent_id.is_none());
    // Parent in einem privaten Team ist für Alice nicht auffindbar
    fx.app
        .create_issue(&bob(), create("DES", "Privat"))
        .await
        .unwrap();
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        parent: Some(Some("DES-1".into())),
                        ..update("ENG-2")
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn relations_and_duplicate(pool: PgPool) {
    let fx = setup(pool).await;
    for t in ["Eins", "Zwei", "Drei"] {
        fx.app
            .create_issue(&alice(), create("ENG", t))
            .await
            .unwrap();
    }
    let s = seed_workflow_state_ids(1);
    // blocks / relatedTo
    fx.app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                relations: RelationChanges {
                    blocks: vec!["ENG-2".into()],
                    related_to: vec!["ENG-3".into()],
                    ..Default::default()
                },
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    let d = fx.app.get_issue(&alice(), "ENG-1").await.unwrap();
    assert_eq!(d.relations.blocks.len(), 1);
    assert_eq!(d.relations.related_to[0].identifier, "ENG-3");
    let d2 = fx.app.get_issue(&alice(), "ENG-2").await.unwrap();
    assert_eq!(d2.relations.blocked_by[0].identifier, "ENG-1");
    // idempotent, History auf beiden Issues
    fx.app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                relations: RelationChanges {
                    blocks: vec!["ENG-2".into()],
                    ..Default::default()
                },
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    assert!(
        !fx.app
            .issue_history(&alice(), "ENG-2")
            .await
            .unwrap()
            .is_empty()
    );
    // gegenläufiges blocks und Selbstbezug verboten
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        relations: RelationChanges {
                            blocks: vec!["ENG-1".into()],
                            ..Default::default()
                        },
                        ..update("ENG-2")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        relations: RelationChanges {
                            related_to: vec!["ENG-1".into()],
                            ..Default::default()
                        },
                        ..update("ENG-1")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // Entfernen
    fx.app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                relations: RelationChanges {
                    remove_blocks: vec!["ENG-2".into()],
                    ..Default::default()
                },
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    assert!(
        fx.app
            .get_issue(&alice(), "ENG-1")
            .await
            .unwrap()
            .relations
            .blocks
            .is_empty()
    );
    // Duplikat: ENG-3 ist Duplikat von ENG-1 → Duplicate-State
    let dup = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                relations: RelationChanges {
                    duplicate_of: Some(Some("ENG-1".into())),
                    ..Default::default()
                },
                ..update("ENG-3")
            },
        )
        .await
        .unwrap();
    assert_eq!(dup.issue.state_id, s[5]);
    assert_eq!(dup.issue.state_type, StateType::Duplicate);
    assert!(dup.issue.canceled_at.is_some());
    let d3 = fx.app.get_issue(&alice(), "ENG-3").await.unwrap();
    assert_eq!(
        d3.relations.duplicate_of.as_ref().unwrap().identifier,
        "ENG-1"
    );
    assert_eq!(
        fx.app
            .get_issue(&alice(), "ENG-1")
            .await
            .unwrap()
            .relations
            .duplicates
            .len(),
        1
    );
    // Kette wird aufgelöst: ENG-2 dup von ENG-3 → kanonisch ENG-1
    let chain = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                relations: RelationChanges {
                    duplicate_of: Some(Some("ENG-3".into())),
                    ..Default::default()
                },
                ..update("ENG-2")
            },
        )
        .await
        .unwrap();
    assert_eq!(chain.issue.state_type, StateType::Duplicate);
    assert_eq!(
        fx.app
            .get_issue(&alice(), "ENG-2")
            .await
            .unwrap()
            .relations
            .duplicate_of
            .unwrap()
            .identifier,
        "ENG-1"
    );
    // Entfernen setzt in den Default-State zurück
    let back = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                relations: RelationChanges {
                    duplicate_of: Some(None),
                    ..Default::default()
                },
                ..update("ENG-3")
            },
        )
        .await
        .unwrap();
    assert_eq!(back.issue.state_id, s[0]);
    assert!(back.issue.canceled_at.is_none());
    assert!(
        fx.app
            .get_issue(&alice(), "ENG-3")
            .await
            .unwrap()
            .relations
            .duplicate_of
            .is_none()
    );
    // Duplikat von sich selbst
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        relations: RelationChanges {
                            duplicate_of: Some(Some("ENG-3".into())),
                            ..Default::default()
                        },
                        ..update("ENG-3")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn history_is_coalesced_per_actor_within_the_window(pool: PgPool) {
    let fx = setup(pool).await;
    fx.app
        .create_issue(&bob(), create("ENG", "Titel"))
        .await
        .unwrap();
    fx.clock.advance(600);
    // Alice ändert zweimal kurz hintereinander: ein Eintrag mit ursprünglichem from.
    for p in [2, 3] {
        fx.clock.advance(5);
        fx.app
            .update_issue(
                &alice(),
                UpdateIssueInput {
                    priority: Some(p),
                    ..update("ENG-1")
                },
            )
            .await
            .unwrap();
    }
    let h = fx.app.issue_history(&alice(), "ENG-1").await.unwrap();
    let mine: Vec<_> = h
        .iter()
        .filter(|e| e.actor_id == Some(fx.data.alice))
        .collect();
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].changes["priority"]["from"], 0);
    assert_eq!(mine[0].changes["priority"]["to"], 3);
    // Zurückändern innerhalb des Fensters löscht den Eintrag
    fx.clock.advance(5);
    fx.app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                priority: Some(0),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    let h = fx.app.issue_history(&alice(), "ENG-1").await.unwrap();
    assert!(h.iter().all(|e| e.actor_id != Some(fx.data.alice)));
    // Anderer Akteur: eigener Eintrag
    fx.clock.advance(5);
    fx.app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                title: Some("Neu".into()),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    fx.clock.advance(5);
    fx.app
        .update_issue(
            &bob(),
            UpdateIssueInput {
                priority: Some(1),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    // nach Ablauf des Fensters: neuer Eintrag
    fx.clock.advance(120);
    fx.app
        .update_issue(
            &bob(),
            UpdateIssueInput {
                priority: Some(2),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    let h = fx.app.issue_history(&alice(), "ENG-1").await.unwrap();
    let bobs = h
        .iter()
        .filter(|e| e.actor_id == Some(fx.data.bob) && !e.flags.contains(&"created".to_string()))
        .count();
    assert_eq!(bobs, 2);
    assert!(h.iter().any(|e| e.flags.contains(&"created".to_string())));
    assert!(h.iter().all(|e| e.source == HistorySource::User));
}

#[sqlx::test(migrations = "./migrations")]
async fn team_change_follows_the_spec(pool: PgPool) {
    let fx = setup(pool).await;
    let org = fx.data.org.0;
    let (cycle, project, _) = planning(&fx, fx.data.web.0, &[fx.data.eng.0, fx.data.web.0]).await;
    let web_label = label(&fx, "Frontend", Some(fx.data.web), None, None).await;
    // Ein weiteres Issue im Zielteam, damit die neue Nummer nicht 1 ist.
    fx.app
        .create_issue(&bob(), create("DES", "Schon da"))
        .await
        .unwrap();
    fx.app
        .create_issue(
            &bob(),
            CreateIssueInput {
                state: Some("In Progress".into()),
                labels: Some(vec!["Frontend".into(), "Bug".into()]),
                cycle: Some(cycle.to_string()),
                project: Some(project.to_string()),
                assignee: Some("bob".into()),
                ..create("WEB", "Umzug")
            },
        )
        .await
        .unwrap();
    let web_states = seed_workflow_state_ids(2);
    let des_states = seed_workflow_state_ids(3);
    // Rechte: Alice sieht DES nicht → not_found
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        team: Some("DES".into()),
                        ..update("WEB-1")
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
    // addLabels mit Teamwechsel verboten
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &bob(),
                    UpdateIssueInput {
                        team: Some("DES".into()),
                        add_labels: vec!["Bug".into()],
                        ..update("WEB-1")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    let moved = fx
        .app
        .update_issue(
            &bob(),
            UpdateIssueInput {
                team: Some("DES".into()),
                ..update("WEB-1")
            },
        )
        .await
        .unwrap();
    let i = &moved.issue;
    assert_eq!(i.identifier, "DES-2");
    assert_eq!(i.team_id, fx.data.des);
    assert_eq!(moved.previous_identifiers, vec!["WEB-1".to_string()]);
    assert_ne!(i.state_id, web_states[2]);
    assert_eq!(i.state_id, des_states[2], "In Progress im Zielteam");
    assert!(i.started_at.is_some());
    assert!(i.cycle_id.is_none(), "Cycle geleert");
    assert!(
        i.project_id.is_none(),
        "Projekt geleert (DES nicht beteiligt)"
    );
    assert_eq!(
        moved.extras.label_names,
        vec!["Bug"],
        "Team-Label von WEB entfernt"
    );
    assert_eq!(i.assignee_id, Some(fx.data.bob), "Bob ist Mitglied von DES");
    let h = fx.app.issue_history(&bob(), "DES-2").await.unwrap();
    assert!(h.iter().any(|e| {
        e.removed_label_ids
            .as_ref()
            .is_some_and(|r| r.contains(&web_label))
    }));
    assert!(h.iter().any(|e| e.changes.get("teamId").is_some()));
    // alte Kennung löst auf, neue auch
    let by_old = fx.app.get_issue(&bob(), "WEB-1").await.unwrap();
    assert_eq!(by_old.view.issue.identifier, "DES-2");
    assert_eq!(
        fx.app
            .get_issue(&bob(), "DES-2")
            .await
            .unwrap()
            .view
            .issue
            .id,
        i.id
    );
    // Für Alice (kein DES-Zugriff) bleibt es unsichtbar, auch über die alte Kennung.
    assert_eq!(
        kind(fx.app.get_issue(&alice(), "WEB-1").await),
        ErrorKind::NotFound
    );
    // Zähler: WEB-Zähler bleibt, nächste WEB-Nummer ist 2
    let next = fx
        .app
        .create_issue(&bob(), create("WEB", "Nachfolger"))
        .await
        .unwrap();
    assert_eq!(next.issue.identifier, "WEB-2");
    // Zurück nach WEB: alte Nummer wird wiederverwendet, Alias entfällt
    let back = fx
        .app
        .update_issue(
            &bob(),
            UpdateIssueInput {
                team: Some("WEB".into()),
                ..update("DES-2")
            },
        )
        .await
        .unwrap();
    assert_eq!(back.issue.identifier, "WEB-1");
    assert_eq!(back.previous_identifiers, vec!["DES-2".to_string()]);
    let aliases: i64 = sqlx::query_scalar("SELECT count(*) FROM issue_identifier_alias WHERE organization_id = $1 AND identifier = 'WEB-1'")
        .bind(org)
        .fetch_one(&fx.pool)
        .await
        .unwrap();
    assert_eq!(aliases, 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn triage_for_non_members_and_estimate_mapping_on_team_change(pool: PgPool) {
    let fx = setup(pool).await;
    let org = fx.data.org.0;
    // WEB: Triage aktiv; ENG nutzt keine Schätzung, WEB Fibonacci.
    let triage = Uuid::new_v4();
    exec(&fx, &format!(
        "INSERT INTO workflow_state (id, organization_id, team_id, name, type, color, position) VALUES ($1, '{org}', '{}', 'Triage', 'triage', '#FC7840', 0)",
        fx.data.web.0), &[triage]).await;
    exec(
        &fx,
        &format!(
            "UPDATE team SET triage_enabled = true, triage_issue_state_id = $1 WHERE id = '{}'",
            fx.data.web.0
        ),
        &[triage],
    )
    .await;
    exec(
        &fx,
        &format!(
            "UPDATE team SET issue_estimation_type = 'fibonacci' WHERE id = '{}'",
            fx.data.web.0
        ),
        &[],
    )
    .await;
    // Alice ist kein Mitglied von WEB → Triage-State.
    let t = fx
        .app
        .create_issue(&alice(), create("WEB", "Von außen"))
        .await
        .unwrap();
    assert_eq!(t.issue.state_id.0, triage);
    assert_eq!(t.issue.state_type, StateType::Triage);
    assert!(t.issue.started_triage_at.is_some());
    // Bob ist Mitglied → Default-State; ein expliziter Status hat Vorrang vor Triage.
    let m = fx
        .app
        .create_issue(&bob(), create("WEB", "Intern"))
        .await
        .unwrap();
    assert_eq!(m.issue.state_type, StateType::Backlog);
    let explicit = fx
        .app
        .create_issue(
            &alice(),
            CreateIssueInput {
                state: Some("Todo".into()),
                ..create("WEB", "Explizit")
            },
        )
        .await
        .unwrap();
    assert_eq!(explicit.issue.state_type, StateType::Unstarted);
    // ENG-Issue (Backlog, ohne Schätzung) durch Alice nach WEB verschieben → Triage (Nicht-Mitglied).
    fx.app
        .create_issue(
            &alice(),
            CreateIssueInput {
                state: Some("In Progress".into()),
                ..create("ENG", "Wandert")
            },
        )
        .await
        .unwrap();
    let moved = fx
        .app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                team: Some("WEB".into()),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    assert_eq!(moved.issue.state_type, StateType::Triage);
    assert!(
        moved.issue.started_at.is_none(),
        "Triage setzt startedAt zurück"
    );
    assert!(moved.issue.started_triage_at.is_some());
    // Schätzung: WEB (Fibonacci) → ENG (nicht genutzt) verliert den Wert, Fibonacci → tShirt bleibt.
    let e = fx
        .app
        .create_issue(
            &bob(),
            CreateIssueInput {
                estimate: Some(5.0),
                ..create("WEB", "Schätzung")
            },
        )
        .await
        .unwrap();
    assert_eq!(e.issue.estimate, Some(5));
    let to_eng = fx
        .app
        .update_issue(
            &bob(),
            UpdateIssueInput {
                team: Some("ENG".into()),
                ..update(&e.issue.identifier)
            },
        )
        .await
        .unwrap();
    assert_eq!(to_eng.issue.estimate, None);
}

#[sqlx::test(migrations = "./migrations")]
async fn trash_restore_and_visibility(pool: PgPool) {
    let fx = setup(pool).await;
    fx.app
        .create_issue(&alice(), create("ENG", "Weg damit"))
        .await
        .unwrap();
    fx.clock.advance(600);
    let trashed = fx.app.delete_issue(&alice(), "ENG-1", false).await.unwrap();
    assert!(trashed.trashed && trashed.trashed_at.is_some());
    assert_eq!(
        kind(fx.app.get_issue(&alice(), "ENG-1").await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        title: Some("x".into()),
                        ..update("ENG-1")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    let page = fx
        .app
        .list_issues(
            &alice(),
            ListIssuesInput {
                limit: 50,
                include_archived: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(page.items.is_empty(), "Papierkorb nie in Listen");
    // Nummer bleibt verbraucht
    let n = fx
        .app
        .create_issue(&alice(), create("ENG", "Neu"))
        .await
        .unwrap();
    assert_eq!(n.issue.identifier, "ENG-2");
    let restored = fx.app.restore_issue(&alice(), "ENG-1").await.unwrap();
    assert!(!restored.issue.trashed && restored.issue.archived_at.is_none());
    assert_eq!(
        fx.app
            .get_issue(&alice(), "ENG-1")
            .await
            .unwrap()
            .view
            .issue
            .title,
        "Weg damit"
    );
    // Aufbewahrungsfrist
    fx.app.delete_issue(&alice(), "ENG-1", false).await.unwrap();
    fx.clock.advance(31 * 24 * 3600);
    assert_eq!(
        kind(fx.app.restore_issue(&alice(), "ENG-1").await),
        ErrorKind::InvalidRequest
    );
    // endgültig löschen: nur Admins; Bob (User) nicht
    assert_eq!(
        kind(fx.app.delete_issue(&bob(), "ENG-1", true).await),
        ErrorKind::Forbidden
    );
    fx.app.delete_issue(&alice(), "ENG-1", true).await.unwrap();
    assert_eq!(
        kind(fx.app.restore_issue(&alice(), "ENG-1").await),
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn sharing_grants_access_to_a_single_issue(pool: PgPool) {
    let fx = setup(pool).await;
    sqlx::query(
        "UPDATE organization SET limits = limits || '{\"issueSharingEnabled\": true}'::jsonb",
    )
    .execute(&fx.pool)
    .await
    .unwrap();
    fx.app
        .create_issue(&bob(), create("DES", "Intern"))
        .await
        .unwrap();
    assert_eq!(
        kind(fx.app.get_issue(&alice(), "DES-1").await),
        ErrorKind::NotFound
    );
    // Alice (kein DES-Mitglied) darf nicht teilen
    assert_eq!(
        kind(
            fx.app
                .share_issue(&alice(), "DES-1", "alice@acme.example")
                .await
        ),
        ErrorKind::NotFound
    );
    let shared = fx
        .app
        .share_issue(&bob(), "DES-1", "alice@acme.example")
        .await
        .unwrap();
    assert!(shared.shared && shared.shared_at.is_some());
    assert_eq!(
        fx.app
            .get_issue(&alice(), "DES-1")
            .await
            .unwrap()
            .view
            .issue
            .title,
        "Intern"
    );
    // Schreiben bleibt verwehrt
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        title: Some("x".into()),
                        ..update("DES-1")
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
    // idempotent; Mitglied hat ohnehin Zugriff
    assert!(
        fx.app
            .share_issue(&bob(), "DES-1", "alice@acme.example")
            .await
            .unwrap()
            .shared
    );
    let already = fx
        .app
        .share_issue(&bob(), "DES-1", "bob@acme.example")
        .await
        .unwrap();
    assert!(!already.shared);
    assert_eq!(already.message.as_deref(), Some("User already has access"));
    let unshared = fx
        .app
        .unshare_issue(&bob(), "DES-1", "alice@acme.example")
        .await
        .unwrap();
    assert!(!unshared.shared);
    assert_eq!(
        kind(fx.app.get_issue(&alice(), "DES-1").await),
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn events_carry_mentions_and_changed_fields(pool: PgPool) {
    let fx = setup(pool).await;
    fx.app
        .create_issue(
            &alice(),
            CreateIssueInput {
                description: Some("Bitte prüfen, @bob. Und @carol (Gast, kein ENG).".into()),
                ..create("ENG", "Erwähnung")
            },
        )
        .await
        .unwrap();
    fx.app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                priority: Some(1),
                assignee: Some(Some("bob".into())),
                description: Some(DescriptionEdit::Patch(vec![PatchOp::Append {
                    text: " cc @alice".into(),
                }])),
                ..update("ENG-1")
            },
        )
        .await
        .unwrap();
    let rows: Vec<(String, serde_json::Value)> =
        sqlx::query_as("SELECT type, payload FROM outbox_event ORDER BY sync_id")
            .fetch_all(&fx.pool)
            .await
            .unwrap();
    let created = rows.iter().find(|r| r.0 == "IssueCreated").unwrap();
    assert_eq!(
        created.1["mentioned_user_ids"],
        serde_json::json!([fx.data.bob.to_string()])
    );
    let updated = rows.iter().find(|r| r.0 == "IssueUpdated").unwrap();
    let changed: Vec<String> = serde_json::from_value(updated.1["changed"].clone()).unwrap();
    for f in ["priority", "assigneeId", "description"] {
        assert!(changed.contains(&f.to_string()), "{f} in {changed:?}");
    }
    assert_eq!(
        updated.1["mentioned_user_ids"],
        serde_json::json!([fx.data.alice.to_string()])
    );
    let actor: Option<Uuid> =
        sqlx::query_scalar("SELECT actor_id FROM outbox_event WHERE type = 'IssueCreated'")
            .fetch_one(&fx.pool)
            .await
            .unwrap();
    assert_eq!(actor, Some(fx.data.alice.0));
}

#[sqlx::test(migrations = "./migrations")]
async fn list_issues_filters(pool: PgPool) {
    let fx = setup(pool).await;
    fx.app
        .create_issue(
            &alice(),
            CreateIssueInput {
                priority: Some(1),
                assignee: Some("bob".into()),
                ..create("ENG", "Eins")
            },
        )
        .await
        .unwrap();
    fx.app
        .create_issue(
            &alice(),
            CreateIssueInput {
                labels: Some(vec!["Bug".into()]),
                state: Some("started".into()),
                ..create("ENG", "Zwei")
            },
        )
        .await
        .unwrap();
    fx.app
        .create_issue(&bob(), create("WEB", "Drei"))
        .await
        .unwrap();
    let list = |input: ListIssuesInput| {
        let app = fx.app.clone();
        async move {
            app.list_issues(&alice(), ListIssuesInput { limit: 50, ..input })
                .await
        }
    };
    assert_eq!(
        list(ListIssuesInput::default()).await.unwrap().items.len(),
        3
    );
    assert_eq!(
        list(ListIssuesInput {
            team: Some("ENG".into()),
            ..Default::default()
        })
        .await
        .unwrap()
        .items
        .len(),
        2
    );
    // UUID-Team schließt Sub-Teams ein; per Key nur mit includeSubTeams
    assert_eq!(
        list(ListIssuesInput {
            team: Some(fx.data.eng.to_string()),
            ..Default::default()
        })
        .await
        .unwrap()
        .items
        .len(),
        3
    );
    assert_eq!(
        list(ListIssuesInput {
            team: Some("ENG".into()),
            include_sub_teams: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .items
        .len(),
        3
    );
    assert_eq!(
        list(ListIssuesInput {
            assignee: Some(Some("bob".into())),
            ..Default::default()
        })
        .await
        .unwrap()
        .items
        .len(),
        1
    );
    assert_eq!(
        list(ListIssuesInput {
            assignee: Some(Some("null".into())),
            ..Default::default()
        })
        .await
        .unwrap()
        .items
        .len(),
        2
    );
    assert_eq!(
        list(ListIssuesInput {
            creator: Some("me".into()),
            ..Default::default()
        })
        .await
        .unwrap()
        .items
        .len(),
        2
    );
    assert_eq!(
        list(ListIssuesInput {
            priority: Some(1),
            ..Default::default()
        })
        .await
        .unwrap()
        .items
        .len(),
        1
    );
    assert_eq!(
        list(ListIssuesInput {
            state: Some("started".into()),
            ..Default::default()
        })
        .await
        .unwrap()
        .items
        .len(),
        1
    );
    assert_eq!(
        list(ListIssuesInput {
            state: Some("Backlog".into()),
            ..Default::default()
        })
        .await
        .unwrap()
        .items
        .len(),
        2
    );
    assert_eq!(
        list(ListIssuesInput {
            label: Some("Bug".into()),
            ..Default::default()
        })
        .await
        .unwrap()
        .items
        .len(),
        1
    );
    assert_eq!(
        kind(
            list(ListIssuesInput {
                label: Some("Nope".into()),
                ..Default::default()
            })
            .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(
            list(ListIssuesInput {
                state: Some("Nope".into()),
                ..Default::default()
            })
            .await
        ),
        ErrorKind::NotFound
    );
    // Paginierung
    let p1 = fx
        .app
        .list_issues(
            &alice(),
            ListIssuesInput {
                limit: 2,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(p1.items.len(), 2);
    let next = p1.next.expect("weitere Seite");
    let p2 = fx
        .app
        .list_issues(
            &alice(),
            ListIssuesInput {
                limit: 2,
                cursor: Some(next),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(p2.items.len(), 1);
    assert!(p2.next.is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn counterpart_checks_for_relations_parent_and_assignee(pool: PgPool) {
    let fx = setup(pool).await;
    fx.app
        .create_issue(&bob(), create("ENG", "Quelle"))
        .await
        .unwrap();
    fx.app
        .create_issue(&bob(), create("WEB", "Gegenstück"))
        .await
        .unwrap();
    fx.app
        .create_issue(&bob(), create("DES", "Privat"))
        .await
        .unwrap();
    let rel = |r: RelationChanges| UpdateIssueInput {
        relations: r,
        ..update("ENG-1")
    };
    for changes in [
        RelationChanges {
            blocks: vec!["DES-1".into()],
            ..Default::default()
        },
        RelationChanges {
            related_to: vec!["DES-1".into()],
            ..Default::default()
        },
        RelationChanges {
            duplicate_of: Some(Some("DES-1".into())),
            ..Default::default()
        },
    ] {
        assert_eq!(
            kind(fx.app.update_issue(&alice(), rel(changes)).await),
            ErrorKind::NotFound
        );
    }
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &alice(),
                    UpdateIssueInput {
                        parent: Some(Some("DES-1".into())),
                        ..update("ENG-1")
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
    sqlx::query("UPDATE team SET retired_at = now() WHERE id = $1")
        .bind(fx.data.web.0)
        .execute(&fx.pool)
        .await
        .unwrap();
    for changes in [
        RelationChanges {
            blocks: vec!["WEB-1".into()],
            ..Default::default()
        },
        RelationChanges {
            blocked_by: vec!["WEB-1".into()],
            ..Default::default()
        },
        RelationChanges {
            duplicate_of: Some(Some("WEB-1".into())),
            ..Default::default()
        },
    ] {
        assert_eq!(
            kind(fx.app.update_issue(&bob(), rel(changes)).await),
            ErrorKind::Forbidden
        );
    }
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &bob(),
                    UpdateIssueInput {
                        parent: Some(Some("WEB-1".into())),
                        ..update("ENG-1")
                    }
                )
                .await
        ),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &bob(),
                    UpdateIssueInput {
                        assignee: Some(Some("alice".into())),
                        ..update("DES-1")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            fx.app
                .update_issue(
                    &bob(),
                    UpdateIssueInput {
                        delegate: Some(Some("alice".into())),
                        ..update("DES-1")
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
}
