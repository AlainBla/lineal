#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP11: Filter-Compiler (docs/05 §1) als Tabellentests gegen echte Datenbank. Die Beispiele
//! stammen aus docs/05 §1.6/§1.7 (Assignee, Labels, Datumsfelder, besondere Werte).
//!
//! Fixture (Uhr 2026-10-10 07:15 UTC, Alice sieht ENG/WEB, nicht das private DES):
//!
//! | Issue | Team | Status | Prio | Assignee | Labels | Besonderes |
//! |---|---|---|---|---|---|---|
//! | Login fails | ENG | Todo | 1 | Bob | Bug | fällig 10-05, Cycle 2 (aktiv), erstellt 09-15, blockiert „Blocked task“ |
//! | Cache bug | ENG | In Progress | 2 | Alice | Bug, Feature | fällig heute, Est 3, Cycle 2, Projekt P-ENG-1, Link |
//! | Dark mode | ENG | Backlog | 0 | – | – | Cycle 3 (nächster), Kommentar „design“, Abonnent Bob |
//! | Müller report | ENG | Done | 3 | Bob | Feature | erledigt 08-15, Est 5, Cycle 1 (vorheriger), Projekt P-ENG-1 |
//! | Fix redirect | ENG | Todo | 3 | Alice | – | Sub-Issue von „Login fails“ |
//! | Navbar | WEB | Todo | 4 | Bob | Improvement | Projekt P-WEB-1 in Sub-Initiative |
//! | Logo refresh | DES | Todo | 0 | Bob | – | privates Team |
//! | Triage me | ENG | Triage | 0 | – | – | |
//! | Old idea | ENG | Canceled | 0 | – | – | |
//! | Blocked task | ENG | Todo | 0 | – | – | wird von „Login fails“ blockiert |
//! | Same as cache | ENG | Duplicate | 0 | – | – | Duplikat von „Cache bug“ |
//! | Late SLA | ENG | Todo | 0 | – | – | SLA verletzt |

mod wp11_common;

use lineal_app::RequestCtx;
use lineal_app::usecases::issue::ListIssuesInput;
use lineal_testkit::seed::planning::{seed_cycle_id, seed_initiative_id, seed_project_id};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;
use wp11_common::*;

struct Data {
    fx: Fx,
    ids: HashMap<&'static str, Uuid>,
    triage_state: Uuid,
    sub_initiative: Uuid,
}

async fn fixture(pool: PgPool) -> Data {
    let fx = setup(pool).await;
    let d = fx.data;
    let mut ids = HashMap::new();
    let mut add = |name: &'static str, id: Uuid| {
        ids.insert(name, id);
    };

    // Zusätzlicher Triage-Status für ENG.
    let triage_state = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO workflow_state (id, organization_id, team_id, name, type, color, position) \
         VALUES ($1,$2,$3,'Triage','triage','#CCCCCC',0)",
    )
    .bind(triage_state)
    .bind(d.org.0)
    .bind(d.eng.0)
    .execute(&fx.pool)
    .await
    .unwrap();

    let login = fx.issue(d.eng, 101, "Login fails", 1).await;
    fx.set(
        login,
        &format!(
            "priority = 1, assignee_id = '{}', due_date = '2026-10-05', created_at = '2026-09-15T09:00:00Z', \
             cycle_id = '{}', added_to_cycle_at = '2026-10-05T09:00:00Z'",
            d.bob.0,
            seed_cycle_id(2).0
        ),
    )
    .await;
    fx.add_label(login, label(0)).await;
    add("Login fails", login);

    let cache = fx.issue(d.eng, 102, "Cache bug", 2).await;
    fx.set(
        cache,
        &format!(
            "priority = 2, assignee_id = '{}', due_date = '2026-10-10', estimate = 3, \
             cycle_id = '{}', added_to_cycle_at = '2026-10-05T10:00:00Z', project_id = '{}'",
            d.alice.0,
            seed_cycle_id(2).0,
            seed_project_id().0
        ),
    )
    .await;
    fx.add_label(cache, label(0)).await;
    fx.add_label(cache, label(1)).await;
    add("Cache bug", cache);

    let dark = fx.issue(d.eng, 103, "Dark mode", 0).await;
    fx.set(
        dark,
        &format!("cycle_id = '{}'", seed_cycle_id(3).0),
    )
    .await;
    add("Dark mode", dark);

    let muller = fx.issue(d.eng, 104, "Müller report", 3).await;
    fx.set(
        muller,
        &format!(
            "priority = 3, assignee_id = '{}', estimate = 5, due_date = '2026-12-01', \
             completed_at = '2026-08-15T10:00:00Z', created_at = '2026-07-20T09:00:00Z', \
             cycle_id = '{}', project_id = '{}'",
            d.bob.0,
            seed_cycle_id(1).0,
            seed_project_id().0
        ),
    )
    .await;
    fx.add_label(muller, label(1)).await;
    add("Müller report", muller);

    let redirect = fx.issue(d.eng, 105, "Fix redirect", 1).await;
    fx.set(
        redirect,
        &format!("priority = 3, assignee_id = '{}', parent_id = '{login}'", d.alice.0),
    )
    .await;
    add("Fix redirect", redirect);

    let navbar = fx.issue(d.web, 101, "Navbar", 1).await;
    fx.set(
        navbar,
        &format!("priority = 4, assignee_id = '{}'", d.bob.0),
    )
    .await;
    fx.add_label(navbar, label(2)).await;
    add("Navbar", navbar);

    let logo = fx.issue(d.des, 101, "Logo refresh", 1).await;
    fx.set(logo, &format!("assignee_id = '{}'", d.bob.0)).await;
    add("Logo refresh", logo);

    let triage = fx.issue(d.eng, 106, "Triage me", 0).await;
    fx.set(triage, &format!("state_id = '{triage_state}'")).await;
    add("Triage me", triage);

    let old = fx.issue(d.eng, 107, "Old idea", 4).await;
    add("Old idea", old);

    let blocked = fx.issue(d.eng, 108, "Blocked task", 1).await;
    add("Blocked task", blocked);
    fx.relate(login, blocked, "blocks").await;

    let same = fx.issue(d.eng, 109, "Same as cache", 5).await;
    fx.relate(same, cache, "duplicate").await;
    add("Same as cache", same);

    let late = fx.issue(d.eng, 110, "Late SLA", 1).await;
    fx.set(
        late,
        "sla_type = 'all', sla_started_at = '2026-09-20T00:00:00Z', sla_medium_risk_at = '2026-09-25T00:00:00Z', \
         sla_high_risk_at = '2026-09-30T00:00:00Z', sla_breaches_at = '2026-10-01T00:00:00Z'",
    )
    .await;
    add("Late SLA", late);

    // Link, Kommentar, Abonnent.
    sqlx::query(
        "INSERT INTO attachment (organization_id, issue_id, url, title) VALUES ($1,$2,'https://example.com/spec','Spec')",
    )
    .bind(d.org.0)
    .bind(cache)
    .execute(&fx.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO comment (organization_id, body, user_id, issue_id) VALUES ($1,'needs design review',$2,$3)",
    )
    .bind(d.org.0)
    .bind(d.alice.0)
    .bind(dark)
    .execute(&fx.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO issue_subscription (organization_id, issue_id, user_id, reason) VALUES ($1,$2,$3,'manual')",
    )
    .bind(d.org.0)
    .bind(dark)
    .bind(d.bob.0)
    .execute(&fx.pool)
    .await
    .unwrap();

    // Sub-Initiative I-2 (Kind von I-1) mit Projekt P-WEB-1 (Team WEB), das „Navbar“ enthält.
    let status: Uuid = sqlx::query_scalar(
        "SELECT id FROM project_status WHERE organization_id = $1 AND type = 'started'",
    )
    .bind(d.org.0)
    .fetch_one(&fx.pool)
    .await
    .unwrap();
    let web_project = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO project (id, organization_id, name, lead_team_id, number, status_id) VALUES ($1,$2,'Web refresh',$3,1,$4)",
    )
    .bind(web_project)
    .bind(d.org.0)
    .bind(d.web.0)
    .bind(status)
    .execute(&fx.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO project_team (organization_id, project_id, team_id) VALUES ($1,$2,$3)")
        .bind(d.org.0)
        .bind(web_project)
        .bind(d.web.0)
        .execute(&fx.pool)
        .await
        .unwrap();
    fx.set(navbar, &format!("project_id = '{web_project}'")).await;
    let sub_initiative = Uuid::now_v7();
    sqlx::query("INSERT INTO initiative (id, organization_id, number, name, status) VALUES ($1,$2,2,'Web','Active')")
        .bind(sub_initiative)
        .bind(d.org.0)
        .execute(&fx.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO initiative_parent (organization_id, parent_id, child_id) VALUES ($1,$2,$3)")
        .bind(d.org.0)
        .bind(seed_initiative_id().0)
        .bind(sub_initiative)
        .execute(&fx.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO initiative_to_project (organization_id, initiative_id, project_id) VALUES ($1,$2,$3)",
    )
    .bind(d.org.0)
    .bind(sub_initiative)
    .bind(web_project)
    .execute(&fx.pool)
    .await
    .unwrap();

    Data {
        fx,
        ids,
        triage_state,
        sub_initiative,
    }
}

async fn titles(fx: &Fx, ctx: &RequestCtx, filter: Value) -> Result<Vec<String>, lineal_app::AppError> {
    let page = fx
        .app
        .list_issues(
            ctx,
            ListIssuesInput {
                filter: Some(filter),
                limit: 250,
                ..Default::default()
            },
        )
        .await?;
    let mut t: Vec<String> = page.items.iter().map(|v| v.issue.title.clone()).collect();
    t.sort();
    Ok(t)
}

fn set(names: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = names.iter().map(|s| (*s).to_owned()).collect();
    v.sort();
    v
}

const ALL_VISIBLE: [&str; 10] = [
    "Login fails",
    "Cache bug",
    "Dark mode",
    "Müller report",
    "Fix redirect",
    "Navbar",
    "Old idea",
    "Blocked task",
    "Same as cache",
    "Late SLA",
];

fn all_except(names: &[&str]) -> Vec<String> {
    set(&ALL_VISIBLE
        .iter()
        .copied()
        .filter(|n| !names.contains(n))
        .collect::<Vec<_>>())
}

#[sqlx::test(migrations = "./migrations")]
async fn docs_05_examples_as_table(pool: PgPool) {
    let dx = fixture(pool).await;
    let d = dx.fx.data;
    let (bob_id, alice_id, carol_id) = (d.bob.to_string(), d.alice.to_string(), d.carol.to_string());
    let (bug, feature) = (label(0).to_string(), label(1).to_string());
    let eng = d.eng.to_string();
    let project = seed_project_id().to_string();
    let init1 = seed_initiative_id().to_string();
    let init2 = dx.sub_initiative.to_string();
    let triage_state = dx.triage_state.to_string();

    // (Name, Filter, erwartete Titel) – als Alice (sieht DES nicht).
    let cases: Vec<(&str, Value, Vec<String>)> = vec![
        ("empty filter = all visible, triage hidden", json!({}), set(&ALL_VISIBLE)),
        // --- §1.6 Skalare und To-One ---
        (
            "assignee is Bob",
            json!({"assignee": {"id": {"in": [bob_id]}}}),
            set(&["Login fails", "Müller report", "Navbar"]),
        ),
        (
            "assignee is not Bob (includes unassigned)",
            json!({"assignee": {"id": {"nin": [bob_id]}}}),
            all_except(&["Login fails", "Müller report", "Navbar"]),
        ),
        (
            "assignee is any of Bob, Alice",
            json!({"assignee": {"id": {"in": [bob_id, alice_id]}}}),
            set(&["Login fails", "Müller report", "Navbar", "Cache bug", "Fix redirect"]),
        ),
        (
            "assignee is none of Bob, Carol",
            json!({"assignee": {"id": {"nin": [bob_id, carol_id]}}}),
            all_except(&["Login fails", "Müller report", "Navbar"]),
        ),
        (
            "assignee is No assignee",
            json!({"assignee": {"null": true}}),
            set(&["Dark mode", "Old idea", "Blocked task", "Same as cache", "Late SLA"]),
        ),
        (
            "assignee is any of No assignee, Bob",
            json!({"assignee": {"or": [{"null": true}, {"id": {"in": [bob_id]}}]}}),
            set(&[
                "Dark mode", "Old idea", "Blocked task", "Same as cache", "Late SLA", "Login fails",
                "Müller report", "Navbar",
            ]),
        ),
        (
            "assignee is not No assignee",
            json!({"assignee": {"null": false}}),
            set(&["Login fails", "Cache bug", "Müller report", "Fix redirect", "Navbar"]),
        ),
        (
            "assignee is me (Alice)",
            json!({"assignee": {"isMe": {"eq": true}}}),
            set(&["Cache bug", "Fix redirect"]),
        ),
        // --- Labels ---
        (
            "labels include Bug",
            json!({"labels": {"some": {"id": {"in": [bug]}}}}),
            set(&["Login fails", "Cache bug"]),
        ),
        (
            "labels do not include Bug (also issues without labels)",
            json!({"labels": {"every": {"id": {"nin": [bug]}}}}),
            all_except(&["Login fails", "Cache bug"]),
        ),
        (
            "labels include all of Bug, Feature",
            json!({"and": [
                {"labels": {"some": {"id": {"in": [bug]}}}},
                {"labels": {"some": {"id": {"in": [feature]}}}}
            ]}),
            set(&["Cache bug"]),
        ),
        (
            "labels include any of Bug, Feature",
            json!({"labels": {"some": {"id": {"in": [bug, feature]}}}}),
            set(&["Login fails", "Cache bug", "Müller report"]),
        ),
        (
            "labels exclude if any of Bug, Feature",
            json!({"labels": {"every": {"id": {"nin": [bug, feature]}}}}),
            all_except(&["Login fails", "Cache bug", "Müller report"]),
        ),
        (
            "labels exclude if all of Bug, Feature",
            json!({"or": [
                {"labels": {"every": {"id": {"neq": bug}}}},
                {"labels": {"every": {"id": {"neq": feature}}}}
            ]}),
            all_except(&["Cache bug"]),
        ),
        (
            "No labels (length eq 0)",
            json!({"labels": {"length": {"eq": 0}}}),
            set(&["Dark mode", "Fix redirect", "Old idea", "Blocked task", "Same as cache", "Late SLA"]),
        ),
        (
            "label group via parent alternative",
            json!({"labels": {"some": {"or": [{"id": {"in": [bug]}}, {"parent": {"id": {"in": [bug]}}}]}}}),
            set(&["Login fails", "Cache bug"]),
        ),
        (
            "label name condition (multi-team views)",
            json!({"labels": {"some": {"name": {"eqIgnoreCase": "improvement"}}}}),
            set(&["Navbar"]),
        ),
        // --- Datumsfelder ---
        (
            "due date Overdue",
            json!({"dueDate": {"lt": "P0D"}}),
            set(&["Login fails"]),
        ),
        (
            "due date Due today",
            json!({"dueDate": {"eq": "P0D"}}),
            set(&["Cache bug"]),
        ),
        (
            "due date is not Overdue (or null)",
            json!({"or": [{"dueDate": {"gte": "P0D"}}, {"dueDate": {"null": true}}]}),
            all_except(&["Login fails"]),
        ),
        (
            "due date in the next week",
            json!({"dueDate": {"gte": "P0D", "lte": "P1W"}}),
            set(&["Cache bug"]),
        ),
        (
            "created before 2026-09-01",
            json!({"createdAt": {"lt": "2026-09-01"}}),
            set(&["Müller report"]),
        ),
        (
            "created after 1 week ago",
            json!({"createdAt": {"gt": "-P1W"}}),
            all_except(&["Login fails", "Müller report"]),
        ),
        (
            "completed is Q3 2026 (end of day inclusive)",
            json!({"completedAt": {"gte": "2026-07-01", "lte": "2026-09-30"}}),
            set(&["Müller report"]),
        ),
        (
            "completed on the same day (lte with date only)",
            json!({"completedAt": {"lte": "2026-08-15"}}),
            set(&["Müller report"]),
        ),
        (
            "completed before that day",
            json!({"completedAt": {"lt": "2026-08-15"}}),
            set(&[]),
        ),
        (
            "time in current status longer than 5 days",
            json!({"stateEnteredAt": {"lt": "-P5D"}}),
            set(&ALL_VISIBLE),
        ),
        // --- Zahlen ---
        (
            "priority any of Urgent, High",
            json!({"priority": {"in": [1, 2]}}),
            set(&["Login fails", "Cache bug"]),
        ),
        (
            "priority lte 2 includes 0 (numeric comparison, docs/05 §1.3)",
            json!({"priority": {"lte": 2}}),
            all_except(&["Müller report", "Fix redirect", "Navbar"]),
        ),
        (
            "estimate between 2 and 5",
            json!({"estimate": {"gte": 2, "lte": 5}}),
            set(&["Cache bug", "Müller report"]),
        ),
        (
            "estimate is empty",
            json!({"estimate": {"null": true}}),
            all_except(&["Cache bug", "Müller report"]),
        ),
        // --- Strings ---
        (
            "title contains ignoring case and accent",
            json!({"title": {"containsIgnoreCaseAndAccent": "MULLER"}}),
            set(&["Müller report"]),
        ),
        (
            "title starts with (ignore case)",
            json!({"title": {"startsWithIgnoreCase": "login"}}),
            set(&["Login fails"]),
        ),
        (
            "title does not contain",
            json!({"title": {"notContainsIgnoreCase": "a"}}),
            set(&["Müller report", "Fix redirect"]),
        ),
        (
            "content search (searchableContent)",
            json!({"searchableContent": {"contains": "login"}}),
            set(&["Login fails"]),
        ),
        // --- Cycle ---
        ("current cycle", json!({"cycle": {"isActive": {"eq": true}}}), set(&["Login fails", "Cache bug"])),
        ("upcoming cycle", json!({"cycle": {"isNext": {"eq": true}}}), set(&["Dark mode"])),
        ("previous cycle", json!({"cycle": {"isPrevious": {"eq": true}}}), set(&["Müller report"])),
        (
            "no cycle",
            json!({"cycle": {"null": true}}),
            set(&["Fix redirect", "Navbar", "Old idea", "Blocked task", "Same as cache", "Late SLA"]),
        ),
        (
            "added to cycle after its start (during)",
            json!({"addedToCyclePeriod": {"eq": "during"}}),
            set(&["Login fails", "Cache bug"]),
        ),
        // --- Projekt, Initiative ---
        ("project is P-ENG-1", json!({"project": {"id": {"in": [project]}}}), set(&["Cache bug", "Müller report"])),
        ("no project", json!({"project": {"null": true}}), all_except(&["Cache bug", "Müller report", "Navbar"])),
        (
            "project status type",
            json!({"project": {"status": {"type": {"eq": "started"}}}}),
            set(&["Cache bug", "Müller report", "Navbar"]),
        ),
        (
            "initiative X is recursive over sub-initiatives",
            json!({"project": {"initiatives": {"some": {"id": {"in": [init1]}}}}}),
            set(&["Cache bug", "Müller report", "Navbar"]),
        ),
        (
            "sub-initiative only",
            json!({"project": {"initiatives": {"some": {"id": {"in": [init2]}}}}}),
            set(&["Navbar"]),
        ),
        // --- Hierarchie, Relationen ---
        ("parent issues (children length > 0)", json!({"children": {"length": {"gt": 0}}}), set(&["Login fails"])),
        ("sub-issues (parent not null)", json!({"parent": {"null": false}}), set(&["Fix redirect"])),
        (
            "parent title (nested relation)",
            json!({"parent": {"title": {"contains": "Login"}}}),
            set(&["Fix redirect"]),
        ),
        ("blocked issues", json!({"hasBlockedByRelations": {"eq": true}}), set(&["Blocked task"])),
        ("blocking issues", json!({"hasBlockingRelations": {"eq": true}}), set(&["Login fails"])),
        (
            "issues with duplicate relations",
            json!({"hasDuplicateRelations": {"eq": true}}),
            set(&["Cache bug", "Same as cache"]),
        ),
        ("duplicates (state type)", json!({"state": {"type": {"eq": "duplicate"}}}), set(&["Same as cache"])),
        // --- Triage, SLA, Links, Kommentare, Abonnenten ---
        ("triage lifts the implicit hiding", json!({"state": {"type": {"eq": "triage"}}}), set(&["Triage me"])),
        (
            "status is the triage state (by id)",
            json!({"state": {"id": {"in": [triage_state]}}}),
            set(&["Triage me"]),
        ),
        (
            "status type any of triage, started",
            json!({"state": {"type": {"in": ["triage", "started"]}}}),
            set(&["Triage me", "Cache bug"]),
        ),
        ("sla breached", json!({"slaStatus": {"eq": "breached"}}), set(&["Late SLA"])),
        ("sla low risk", json!({"slaStatus": {"in": ["lowRisk"]}}), Vec::new()),
        (
            "links include",
            json!({"attachments": {"some": {"url": {"containsIgnoreCase": "example.com"}}}}),
            set(&["Cache bug"]),
        ),
        ("links is empty", json!({"attachments": {"length": {"eq": 0}}}), all_except(&["Cache bug"])),
        (
            "comments contain",
            json!({"comments": {"some": {"body": {"containsIgnoreCase": "design"}}}}),
            set(&["Dark mode"]),
        ),
        (
            "subscribers include Bob",
            json!({"subscribers": {"some": {"id": {"in": [bob_id]}}}}),
            set(&["Dark mode"]),
        ),
        // --- Teams, Sub-Teams ---
        (
            "team is ENG (without sub-teams)",
            json!({"team": {"id": {"in": [eng]}}}),
            all_except(&["Navbar"]),
        ),
        (
            "ENG including sub-teams via ancestors",
            json!({"team": {"or": [{"id": {"eq": eng}}, {"ancestors": {"some": {"id": {"eq": eng}}}}]}}),
            set(&ALL_VISIBLE),
        ),
        (
            "sub-team only via ancestors",
            json!({"team": {"ancestors": {"some": {"id": {"eq": eng}}}}}),
            set(&["Navbar"]),
        ),
        // --- Logik ---
        (
            "and/or nesting",
            json!({"and": [
                {"assignee": {"id": {"in": [bob_id]}}},
                {"or": [{"priority": {"in": [1]}}, {"priority": {"in": [3]}}]}
            ]}),
            set(&["Login fails", "Müller report"]),
        ),
        (
            "a top-level or cannot lift the permission filter",
            json!({"or": [{"assignee": {"id": {"in": [bob_id]}}}, {"priority": {"in": [0, 1, 2, 3, 4]}}]}),
            set(&ALL_VISIBLE),
        ),
    ];

    for (name, filter, expected) in cases {
        let got = titles(&dx.fx, &alice(), filter.clone())
            .await
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(got, expected, "{name}\nfilter: {filter}");
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn bob_sees_the_private_team_and_isme_follows_the_caller(pool: PgPool) {
    let dx = fixture(pool).await;
    let d = dx.fx.data;
    let got = titles(&dx.fx, &bob(), json!({"assignee": {"isMe": {"eq": true}}}))
        .await
        .unwrap();
    assert_eq!(got, set(&["Login fails", "Müller report", "Navbar", "Logo refresh"]));
    let des = d.des.to_string();
    let got = titles(&dx.fx, &bob(), json!({"team": {"id": {"in": [des]}}}))
        .await
        .unwrap();
    assert_eq!(got, set(&["Logo refresh"]));
}

#[sqlx::test(migrations = "./migrations")]
async fn filter_on_a_private_team_is_not_found_for_non_members(pool: PgPool) {
    let dx = fixture(pool).await;
    let d = dx.fx.data;
    let des = d.des.to_string();
    for filter in [
        json!({"team": {"id": {"in": [des]}}}),
        json!({"team": {"id": {"nin": [des]}}}),
        json!({"or": [{"team": {"id": {"eq": des}}}, {"priority": {"in": [1]}}]}),
        json!({"team": {"ancestors": {"some": {"id": {"eq": des}}}}}),
    ] {
        assert_eq!(
            kind(titles(&dx.fx, &alice(), filter.clone()).await),
            lineal_app::ErrorKind::NotFound,
            "{filter}"
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn unknown_references_and_invalid_filters_fail_loudly(pool: PgPool) {
    let dx = fixture(pool).await;
    let ghost = Uuid::new_v4().to_string();
    for filter in [
        json!({"assignee": {"id": {"in": [ghost]}}}),
        json!({"labels": {"some": {"id": {"in": [ghost]}}}}),
        json!({"state": {"id": {"eq": ghost}}}),
        json!({"project": {"id": {"eq": ghost}}}),
        json!({"cycle": {"id": {"eq": ghost}}}),
    ] {
        assert_eq!(
            kind(titles(&dx.fx, &alice(), filter.clone()).await),
            lineal_app::ErrorKind::NotFound,
            "{filter}"
        );
    }
    for filter in [
        json!({"bogus": {"eq": 1}}),
        json!({"priority": {"in": [7]}}),
        json!({"title": {"contains": "x"}, "assignee": {"email": {"eq": "a@b.c"}}}),
        json!({"assignee": {"id": {"in": ["not-a-uuid"]}}}),
    ] {
        assert_eq!(
            kind(titles(&dx.fx, &alice(), filter.clone()).await),
            lineal_app::ErrorKind::InvalidRequest,
            "{filter}"
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn values_are_parameterized_not_interpolated(pool: PgPool) {
    let dx = fixture(pool).await;
    // Ein Wert mit SQL-Syntax ist nur ein Textwert: kein Fehler, kein Treffer, Tabelle intakt.
    let evil = "x'); DELETE FROM issue; --";
    let got = titles(&dx.fx, &alice(), json!({"title": {"containsIgnoreCase": evil}}))
        .await
        .unwrap();
    assert!(got.is_empty());
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM issue")
        .fetch_one(&dx.fx.pool)
        .await
        .unwrap();
    assert!(n >= 12);
}

#[sqlx::test(migrations = "./migrations")]
async fn sub_issue_of_an_invisible_parent_does_not_leak_through_nested_filters(pool: PgPool) {
    let dx = fixture(pool).await;
    let d = dx.fx.data;
    // „Fix redirect“ (ENG) bekommt einen Parent im privaten DES-Team.
    let logo = dx.ids["Logo refresh"];
    let redirect = dx.ids["Fix redirect"];
    dx.fx.set(redirect, &format!("parent_id = '{logo}'")).await;
    // Alice sieht den Parent nicht: weder dessen Titel noch dessen Team ist filterbar.
    let got = titles(&dx.fx, &alice(), json!({"parent": {"title": {"contains": "Logo"}}}))
        .await
        .unwrap();
    assert!(got.is_empty(), "{got:?}");
    // Bob (Mitglied von DES) sieht ihn.
    let got = titles(&dx.fx, &bob(), json!({"parent": {"title": {"contains": "Logo"}}}))
        .await
        .unwrap();
    assert_eq!(got, set(&["Fix redirect"]));
    // `children.length` zählt nur sichtbare Kinder: Alice sieht das DES-Elternteil nicht.
    let got = titles(&dx.fx, &alice(), json!({"children": {"length": {"gt": 0}}}))
        .await
        .unwrap();
    assert!(got.is_empty(), "{got:?}");
    let got = titles(&dx.fx, &bob(), json!({"children": {"length": {"gt": 0}}}))
        .await
        .unwrap();
    assert_eq!(got, set(&["Logo refresh"]));
    let _ = d;
}
