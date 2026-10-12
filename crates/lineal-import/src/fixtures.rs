//! Fixtures mit erfundenen Daten für die Fake-Quelle (Workspace `acme`: Alice, Bob, Carol, Dave).
//!
//! `acceptance_workspace()` entspricht dem Testworkspace aus docs/13 §9 Nr. 1: 3 Teams plus ein
//! Sub-Team (davon ein privates), 200 Issues mit Sub-Issues, Relationen, Kommentaren, Anhängen,
//! 3 Projekte, 2 Initiativen, 2 Cycles. `ENG-123` existiert (Nummern haben Lücken).

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::{Value, json};

/// Zeitpunkt `offset_hours` Stunden nach dem Fixture-Start (2025-01-06 09:00 UTC) als ISO-Text.
pub fn at(offset_hours: i64) -> String {
    let base: DateTime<Utc> = Utc
        .with_ymd_and_hms(2025, 1, 6, 9, 0, 0)
        .single()
        .unwrap_or_default();
    (base + Duration::hours(offset_hours)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn id(v: &str) -> Value {
    json!({ "id": v })
}

/// Fixture im Format von `FakeSource::from_json`.
pub fn acceptance_workspace() -> Value {
    let users = json!([
        { "id": "u-alice", "name": "Alice Admin", "displayName": "alice", "email": "alice@acme.test", "admin": true, "guest": false, "active": true, "createdAt": at(0), "updatedAt": at(0) },
        { "id": "u-bob", "name": "Bob Builder", "displayName": "bob", "email": "bob@acme.test", "admin": false, "guest": false, "active": true, "createdAt": at(1), "updatedAt": at(1) },
        { "id": "u-carol", "name": "Carol Guest", "displayName": "carol", "email": "Carol@Acme.test", "admin": false, "guest": true, "active": true, "createdAt": at(2), "updatedAt": at(2) },
        { "id": "u-dave", "name": "Dave Dev", "displayName": "dave", "email": "dave@acme.test", "admin": false, "guest": false, "active": true, "createdAt": at(3), "updatedAt": at(3) },
    ]);
    let teams = json!([
        { "id": "t-eng", "key": "ENG", "name": "Engineering", "description": "Product engineering", "color": "#4F46E5", "private": false,
          "timezone": "Europe/Berlin", "createdAt": at(0), "updatedAt": at(0),
          "cyclesEnabled": true, "cycleDuration": 2, "cycleCooldownTime": 0, "cycleStartDay": 1, "upcomingCycleCount": 2,
          "issueEstimationType": "fibonacci", "triageEnabled": true, "autoClosePeriod": 6, "autoArchivePeriod": 6,
          "defaultIssueState": id("s-ENG-todo"), "triageIssueState": id("s-ENG-triage") },
        { "id": "t-plt", "key": "PLT", "name": "Platform", "private": false, "timezone": "Europe/Berlin", "createdAt": at(0), "updatedAt": at(0),
          "defaultIssueState": id("s-PLT-todo") },
        { "id": "t-web", "key": "WEB", "name": "Web", "private": false, "parent": id("t-eng"), "timezone": "Europe/Berlin", "createdAt": at(0), "updatedAt": at(0),
          "defaultIssueState": id("s-WEB-todo") },
        { "id": "t-des", "key": "DES", "name": "Design", "private": true, "timezone": "Europe/Berlin", "createdAt": at(0), "updatedAt": at(0),
          "defaultIssueState": id("s-DES-todo") },
    ]);
    let memberships = json!([
        { "id": "m-1", "user": id("u-alice"), "team": id("t-eng"), "owner": true, "createdAt": at(0) },
        { "id": "m-2", "user": id("u-alice"), "team": id("t-plt"), "owner": false, "createdAt": at(0) },
        { "id": "m-3", "user": id("u-alice"), "team": id("t-web"), "owner": false, "createdAt": at(0) },
        { "id": "m-4", "user": id("u-alice"), "team": id("t-des"), "owner": true, "createdAt": at(0) },
        { "id": "m-5", "user": id("u-bob"), "team": id("t-eng"), "owner": false, "createdAt": at(1) },
        { "id": "m-6", "user": id("u-bob"), "team": id("t-web"), "owner": false, "createdAt": at(1) },
        { "id": "m-7", "user": id("u-carol"), "team": id("t-des"), "owner": false, "createdAt": at(2) },
        { "id": "m-8", "user": id("u-dave"), "team": id("t-plt"), "owner": false, "createdAt": at(3) },
    ]);
    let mut states: Vec<Value> = Vec::new();
    for (key, team) in [
        ("ENG", "t-eng"),
        ("PLT", "t-plt"),
        ("WEB", "t-web"),
        ("DES", "t-des"),
    ] {
        let mut defs = vec![
            ("backlog", "Backlog", "backlog", 0.0),
            ("todo", "Todo", "unstarted", 1.0),
            ("progress", "In Progress", "started", 2.0),
            ("review", "In Review", "started", 3.0),
            ("done", "Done", "completed", 4.0),
            ("canceled", "Canceled", "canceled", 5.0),
            ("duplicate", "Duplicate", "canceled", 6.0),
        ];
        if key == "ENG" {
            defs.push(("triage", "Triage", "triage", -1.0));
        }
        for (slug, name, ty, pos) in defs {
            states.push(json!({ "id": format!("s-{key}-{slug}"), "name": name, "type": ty, "color": "#95A2B3",
                "position": pos, "team": id(team), "createdAt": at(0), "updatedAt": at(0) }));
        }
    }
    let labels = json!([
        { "id": "l-bug", "name": "Bug", "color": "#EB5757", "isGroup": false, "createdAt": at(0), "updatedAt": at(0), "creator": id("u-alice") },
        { "id": "l-feature", "name": "Feature", "color": "#4EA7FC", "isGroup": false, "createdAt": at(0), "updatedAt": at(0) },
        { "id": "l-area", "name": "Area", "color": "#BB87FC", "isGroup": true, "team": id("t-eng"), "createdAt": at(0), "updatedAt": at(0) },
        { "id": "l-area-api", "name": "Api", "color": "#BB87FC", "parent": id("l-area"), "team": id("t-eng"), "createdAt": at(0), "updatedAt": at(0) },
        { "id": "l-area-ui", "name": "Ui", "color": "#BB87FC", "parent": id("l-area"), "team": id("t-eng"), "createdAt": at(0), "updatedAt": at(0) },
        { "id": "l-frontend", "name": "Frontend", "color": "#26B5CE", "team": id("t-web"), "createdAt": at(0), "updatedAt": at(0) },
    ]);
    let project_statuses = json!([
        { "id": "ps-backlog", "name": "Backlog", "type": "backlog", "color": "#BEC2C8", "position": 0, "indefinite": true, "createdAt": at(0), "updatedAt": at(0) },
        { "id": "ps-planned", "name": "Planned", "type": "planned", "color": "#BEC2C8", "position": 1, "indefinite": true, "createdAt": at(0), "updatedAt": at(0) },
        { "id": "ps-started", "name": "In Progress", "type": "started", "color": "#F2C94C", "position": 2, "indefinite": false, "createdAt": at(0), "updatedAt": at(0) },
        { "id": "ps-done", "name": "Completed", "type": "completed", "color": "#5E6AD2", "position": 3, "indefinite": false, "createdAt": at(0), "updatedAt": at(0) },
        { "id": "ps-canceled", "name": "Canceled", "type": "canceled", "color": "#95A2B3", "position": 4, "indefinite": false, "createdAt": at(0), "updatedAt": at(0) },
    ]);
    let project_labels = json!([
        { "id": "pl-customer", "name": "Customer", "color": "#F2994A", "isGroup": false, "createdAt": at(0), "updatedAt": at(0) },
    ]);
    let templates = json!([
        { "id": "tpl-1", "name": "Bug report", "type": "issue", "description": "Standard bug", "team": id("t-eng"),
          "templateData": { "title": "Bug: ", "description": "Steps to reproduce", "priority": 2, "labelIds": ["l-bug"],
                            "descriptionData": { "type": "doc" }, "sla": 3 }, "creator": id("u-alice"), "createdAt": at(0), "updatedAt": at(0) },
        { "id": "tpl-2", "name": "Spec", "type": "document", "templateData": { "title": "Spec", "content": "# Spec" }, "createdAt": at(0), "updatedAt": at(0) },
    ]);
    let cycles = json!([
        { "id": "c-1", "number": 1, "name": "Sprint 1", "startsAt": at(0), "endsAt": at(24 * 14), "completedAt": at(24 * 14), "team": id("t-eng"),
          "scopeHistory": [3.0, 5.0, 8.0], "completedScopeHistory": [0.0, 3.0, 8.0], "createdAt": at(0), "updatedAt": at(24 * 14) },
        { "id": "c-2", "number": 2, "startsAt": at(24 * 14), "endsAt": at(24 * 28), "team": id("t-eng"), "createdAt": at(24 * 7), "updatedAt": at(24 * 15) },
    ]);
    let initiatives = json!([
        { "id": "i-1", "name": "Platform 2026", "description": "Make the platform fly", "content": "## Goals\nSee https://linear.app/acme/project/checkout-redesign-0123456789ab",
          "status": "Active", "owner": id("u-alice"), "creator": id("u-alice"), "slugId": "a1b2c3d4e5f6", "targetDate": "2026-06-30", "sortOrder": 1.0,
          "createdAt": at(10), "updatedAt": at(11) },
        { "id": "i-2", "name": "Reliability", "status": "Planned", "owner": id("u-bob"), "parentInitiative": id("i-1"), "sortOrder": 2.0, "createdAt": at(12), "updatedAt": at(12) },
    ]);
    let projects = json!([
        { "id": "p-1", "name": "Checkout redesign", "description": "New checkout flow", "content": "Details with <issue id=\"x\" href=\"h\">ENG-10</issue>",
          "color": "#4F46E5", "status": id("ps-started"), "priority": 2, "lead": id("u-alice"), "creator": id("u-alice"),
          "slugId": "0123456789ab", "startDate": "2025-02-01", "targetDate": "2025-06-30", "health": "onTrack",
          "teams": [id("t-eng"), id("t-web")], "members": [id("u-alice"), id("u-bob")], "labels": [id("pl-customer")], "initiatives": [id("i-1")],
          "createdAt": at(20), "updatedAt": at(21) },
        { "id": "p-2", "name": "API v2", "color": "#26B5CE", "status": id("ps-planned"), "priority": 3, "lead": id("u-dave"),
          "teams": [id("t-eng"), id("t-plt")], "members": [id("u-dave")], "initiatives": [id("i-1"), id("i-2")], "createdAt": at(22), "updatedAt": at(22) },
        { "id": "p-3", "name": "Brand refresh", "color": "#F2994A", "status": id("ps-backlog"), "priority": 0, "lead": id("u-carol"),
          "teams": [id("t-des")], "members": [id("u-carol")], "createdAt": at(23), "updatedAt": at(23) },
    ]);
    let milestones = json!([
        { "id": "ms-1-1", "name": "Design done", "project": id("p-1"), "targetDate": "2025-03-01", "sortOrder": 1.0, "createdAt": at(24), "updatedAt": at(24) },
        { "id": "ms-1-2", "name": "Beta", "project": id("p-1"), "targetDate": "2025-05-01", "sortOrder": 2.0, "createdAt": at(24), "updatedAt": at(24) },
        { "id": "ms-2-1", "name": "Contract frozen", "project": id("p-2"), "targetDate": "2025-04-01", "sortOrder": 1.0, "createdAt": at(24), "updatedAt": at(24) },
    ]);

    // --- Issues -------------------------------------------------------------------------
    let eng_gaps = [
        7, 21, 33, 50, 64, 77, 88, 99, 101, 110, 111, 112, 115, 118, 119, 120, 121, 122, 124, 125,
    ];
    let mut plan: Vec<(&str, &str, i64)> = Vec::new();
    for n in 1..=130 {
        if !eng_gaps.contains(&n) {
            plan.push(("ENG", "t-eng", n));
        }
    }
    for n in 1..=30 {
        plan.push(("PLT", "t-plt", n));
    }
    for n in 1..=40 {
        plan.push(("WEB", "t-web", n));
    }
    for n in 1..=20 {
        plan.push(("DES", "t-des", n));
    }
    let user_ids = ["u-alice", "u-bob", "u-carol", "u-dave"];
    let state_slugs = ["backlog", "todo", "progress", "review", "done", "canceled"];
    let mut issues: Vec<Value> = Vec::new();
    for (idx, (key, team, n)) in plan.iter().enumerate() {
        let (key, team, n) = (*key, *team, *n);
        let slot = state_slugs[(n as usize) % state_slugs.len()];
        let mut state = slot.to_owned();
        if key == "ENG" && n == 70 {
            state = "duplicate".into();
        }
        let created = (idx as i64) * 2 + 30;
        let assignee = if n % 7 == 0 {
            Value::Null
        } else {
            id(user_ids[(n as usize) % 2])
        };
        let mut labels: Vec<Value> = Vec::new();
        if n % 7 == 0 {
            labels.push(id("l-bug"));
        }
        if n % 11 == 0 {
            labels.push(id("l-feature"));
        }
        if key == "ENG" && n % 5 == 0 {
            labels.push(id("l-area-api"));
        }
        if key == "ENG" && n % 5 == 1 {
            labels.push(id("l-area-ui"));
        }
        if key == "WEB" && n % 4 == 0 {
            labels.push(id("l-frontend"));
        }
        let (project, milestone) = match (key, n) {
            ("ENG", n) if n % 13 == 0 => (
                id("p-1"),
                if n % 2 == 0 {
                    id("ms-1-1")
                } else {
                    id("ms-1-2")
                },
            ),
            ("WEB", n) if n % 9 == 0 => (id("p-1"), id("ms-1-1")),
            ("ENG", n) if n % 17 == 0 => (id("p-2"), id("ms-2-1")),
            ("PLT", n) if n % 10 == 0 => (id("p-2"), Value::Null),
            ("DES", n) if n % 5 == 0 => (id("p-3"), Value::Null),
            _ => (Value::Null, Value::Null),
        };
        let cycle = match (key, n) {
            ("ENG", 60..=70) => id("c-1"),
            ("ENG", 90..=100) => id("c-2"),
            _ => Value::Null,
        };
        let parent = match (key, n) {
            ("ENG", 3) => id("i-ENG-12"),
            ("ENG", 13..=15) => id("i-ENG-10"),
            ("WEB", 2) => id("i-ENG-10"),
            ("PLT", 4) | ("PLT", 5) => id("i-PLT-3"),
            _ => Value::Null,
        };
        let mut description = format!("Task {key}-{n} in the acme workspace.");
        match (key, n) {
            (_, n) if n % 10 == 0 => description.push_str("\n\nSee <issue id=\"x\" href=\"https://linear.app/acme/issue/ENG-12/foo\">ENG-12</issue> and https://linear.app/acme/issue/WEB-3/bar"),
            ("ENG", 5) => description.push_str("\n\n<script>alert(1)</script> [x](javascript:alert(1)) <linear-image>{}</linear-image>"),
            ("ENG", 8) => description.push_str("\n\n![shot](https://uploads.linear.app/acme/abc/shot.png)"),
            ("ENG", 9) => description.push_str("\n\n[@Bob](https://linear.app/acme/profiles/bob) please look"),
            _ => {}
        }
        let ident = format!("{key}-{n}");
        let mut v = json!({
            "id": format!("i-{key}-{n}"), "number": n, "identifier": ident, "title": format!("{key} task {n}"),
            "description": description, "priority": n % 5,
            "estimate": if key == "ENG" { json!([1, 2, 3, 5, 8][(n as usize) % 5]) } else { Value::Null },
            "sortOrder": -(n as f64), "createdAt": at(created), "updatedAt": at(created + 5),
            "team": id(team), "state": id(&format!("s-{key}-{state}")), "assignee": assignee, "creator": id("u-alice"),
            "parent": parent, "project": project, "projectMilestone": milestone, "cycle": cycle, "labels": labels,
            "trashed": false,
        });
        if slot == "progress" || slot == "review" {
            v["startedAt"] = json!(at(created + 1));
        }
        if slot == "done" {
            v["startedAt"] = json!(at(created + 1));
            v["completedAt"] = json!(at(created + 4));
        }
        if slot == "canceled" {
            v["canceledAt"] = json!(at(created + 4));
        }
        if key == "ENG" && n == 123 {
            v["previousIdentifiers"] = json!(["OLD-4"]);
            v["title"] = json!("The famous ENG-123");
        }
        if key == "ENG" && n == 60 {
            v["trashed"] = json!(true);
        }
        if key == "ENG" && n == 61 {
            v["archivedAt"] = json!(at(created + 100));
        }
        if key == "ENG" && n == 2 {
            v["slaType"] = json!("all");
            v["slaStartedAt"] = json!(at(created));
            v["slaBreachesAt"] = json!(at(created + 72));
        }
        issues.push(v);
    }

    let relations: Vec<Value> = [
        ("r-1", "blocks", "ENG-1", "ENG-2"),
        ("r-2", "blocks", "ENG-2", "ENG-4"),
        ("r-3", "related", "WEB-1", "ENG-3"),
        ("r-4", "related", "ENG-3", "WEB-1"), // Gegenrichtung desselben Paars -> wird übersprungen
        ("r-5", "duplicate", "ENG-70", "ENG-71"),
        ("r-6", "similar", "PLT-1", "PLT-2"),
        ("r-7", "blocks", "PLT-3", "PLT-4"),
        ("r-8", "related", "DES-1", "DES-2"),
        ("r-9", "blocks", "WEB-5", "WEB-6"),
        ("r-10", "related", "ENG-10", "PLT-10"),
        ("r-11", "blocks", "ENG-12", "ENG-13"),
        ("r-12", "related", "ENG-20", "ENG-22"),
    ]
    .iter()
    .enumerate()
    .map(|(i, (rid, ty, a, b))| {
        json!({ "id": rid, "type": ty, "issue": id(&format!("i-{a}")), "relatedIssue": id(&format!("i-{b}")),
                "createdAt": at(300 + i as i64), "updatedAt": at(300 + i as i64) })
    })
    .collect();

    let mut history: Vec<Value> = Vec::new();
    for n in 1..=100 {
        if eng_gaps.contains(&n) {
            continue;
        }
        let slot = state_slugs[(n as usize) % state_slugs.len()];
        history.push(json!({
            "id": format!("h-ENG-{n}"), "issue": id(&format!("i-ENG-{n}")), "actor": id("u-alice"),
            "createdAt": at(400 + n), "fromState": id("s-ENG-backlog"), "toState": id(&format!("s-ENG-{slot}")),
        }));
        if n % 3 == 0 {
            history.push(json!({
                "id": format!("h-ENG-{n}-a"), "issue": id(&format!("i-ENG-{n}")), "actor": id("u-bob"),
                "createdAt": at(500 + n), "fromAssignee": Value::Null, "toAssignee": id("u-bob"), "fromPriority": 0, "toPriority": 2,
            }));
        }
    }

    let mut subscriptions: Vec<Value> = Vec::new();
    for n in 1..=20 {
        if eng_gaps.contains(&n) {
            continue;
        }
        subscriptions.push(json!({ "id": format!("sub-bob-{n}"), "issue": id(&format!("i-ENG-{n}")), "user": id("u-bob") }));
        if n % 5 == 0 {
            subscriptions.push(json!({ "id": format!("sub-alice-{n}"), "issue": id(&format!("i-ENG-{n}")), "user": id("u-alice") }));
        }
    }

    let mut comments: Vec<Value> = Vec::new();
    for n in 1..=30 {
        if eng_gaps.contains(&n) {
            continue;
        }
        let top = format!("c-ENG-{n}");
        let mut c = json!({
            "id": top, "body": format!("Looks good, comment on ENG-{n}. See https://linear.app/acme/issue/ENG-1/x"),
            "user": id(if n % 4 == 0 { "u-dave" } else { "u-bob" }), "issue": id(&format!("i-ENG-{n}")),
            "createdAt": at(600 + n * 2), "updatedAt": at(600 + n * 2),
            "reactions": [{ "id": format!("rx-{n}"), "emoji": "👍", "user": id("u-alice"), "createdAt": at(601 + n * 2) }],
        });
        if n == 1 {
            c["quotedText"] = json!("quoted part");
            c["body"] = json!(
                "Inline comment <img src=x onerror=alert(1)> with file https://uploads.linear.app/acme/abc/diagram.png"
            );
        }
        comments.push(c);
        comments.push(json!({
            "id": format!("c-ENG-{n}-r"), "body": "Thanks, will do.", "user": id("u-alice"), "issue": id(&format!("i-ENG-{n}")),
            "parent": id(&top), "createdAt": at(601 + n * 2), "updatedAt": at(601 + n * 2), "editedAt": at(602 + n * 2),
        }));
    }

    let mut attachments: Vec<Value> = Vec::new();
    for n in 1..=15 {
        if eng_gaps.contains(&n) {
            continue;
        }
        attachments.push(json!({
            "id": format!("at-{n}"), "url": format!("https://github.com/acme/app/pull/{n}"), "title": format!("PR #{n}"),
            "subtitle": "open", "sourceType": "github", "metadata": { "number": n }, "issue": id(&format!("i-ENG-{n}")),
            "creator": id("u-bob"), "createdAt": at(700 + n), "updatedAt": at(700 + n),
        }));
    }
    attachments.push(json!({
        "id": "at-file-1", "url": "https://uploads.linear.app/acme/files/spec.pdf", "title": "spec.pdf", "issue": id("i-ENG-1"),
        "creator": id("u-alice"), "createdAt": at(720), "updatedAt": at(720),
    }));
    attachments.push(json!({
        "id": "at-file-2", "url": "https://uploads.linear.app/acme/files/missing.png", "title": "missing.png", "issue": id("i-ENG-2"),
        "createdAt": at(721), "updatedAt": at(721),
    }));

    let documents = json!([
        { "id": "d-1", "title": "Checkout spec", "content": "# Spec\nSee <issue id=\"x\" href=\"h\">ENG-10</issue>", "slugId": "fedcba987654", "project": id("p-1"),
          "creator": id("u-alice"), "createdAt": at(800), "updatedAt": at(801) },
        { "id": "d-2", "title": "Initiative brief", "content": "Brief", "initiative": id("i-1"), "creator": id("u-alice"), "createdAt": at(802), "updatedAt": at(802) },
        { "id": "d-3", "title": "Notes", "content": "Notes on ENG-1", "issue": id("i-ENG-1"), "creator": id("u-bob"), "createdAt": at(803), "updatedAt": at(803) },
    ]);
    let project_updates = json!([
        { "id": "pu-1", "body": "Kickoff done", "health": "onTrack", "user": id("u-alice"), "project": id("p-1"), "createdAt": at(900), "updatedAt": at(900) },
        { "id": "pu-2", "body": "Slight delay", "health": "atRisk", "user": id("u-bob"), "project": id("p-1"), "createdAt": at(950), "updatedAt": at(950) },
        { "id": "pu-3", "body": "Planning", "health": "onTrack", "user": id("u-dave"), "project": id("p-2"), "createdAt": at(960), "updatedAt": at(960) },
    ]);
    let initiative_updates = json!([
        { "id": "iu-1", "body": "On track", "health": "onTrack", "user": id("u-alice"), "initiative": id("i-1"), "createdAt": at(970), "updatedAt": at(970) },
    ]);

    // Eine kleine PNG-Datei (1x1) und ein PDF-Stub.
    let png = "89504e470d0a1a0a0000000d49484452000000010000000108060000001f15c4890000000d49444154789c6360000002000001e221bc330000000049454e44ae426082";
    json!({
        "workspace": { "id": "lin-org-acme", "name": "Acme", "urlKey": "acme", "viewerEmail": "alice@acme.test" },
        "entities": {
            "User": users, "Team": teams, "TeamMembership": memberships, "WorkflowState": states, "IssueLabel": labels,
            "ProjectStatus": project_statuses, "ProjectLabel": project_labels, "Template": templates, "Cycle": cycles,
            "Initiative": initiatives, "Project": projects, "ProjectMilestone": milestones, "Issue": issues,
            "IssueRelation": relations, "IssueHistory": history, "IssueSubscription": subscriptions, "Comment": comments,
            "Attachment": attachments, "Document": documents, "ProjectUpdate": project_updates, "InitiativeUpdate": initiative_updates,
        },
        "files": {
            "https://uploads.linear.app/acme/abc/shot.png": { "contentType": "image/png", "hex": png },
            "https://uploads.linear.app/acme/abc/diagram.png": { "contentType": "image/png", "hex": png },
            "https://uploads.linear.app/acme/files/spec.pdf": { "contentType": "application/pdf", "text": "%PDF-1.4 fake" },
        },
    })
}

/// Kleines Fixture (2 Teams, wenige Issues) für Delta- und Konflikttests.
pub fn small_workspace() -> Value {
    let mut v = acceptance_workspace();
    // Nur ENG und WEB behalten ist Sache der Team-Auswahl im Test; hier bleibt alles erhalten.
    v["workspace"]["name"] = json!("Acme (small)");
    v
}

/// Fixture nach dem Muster eines Teilimports (WP28): zwei Wurzelteams (`KOL` „KollegePlus“ und
/// `PLN`), 20 Projekte (davon 5 über beide Teams), eigene Status je Team (13 bzw. 11, mehrere je
/// Kategorie, Namen unterschiedlich, je ein `Duplicate`), 18 **Workspace**-Labels mit Beschreibung
/// (eines zurückgezogen), Agent-Delegierung, unbekannte Enum-Werte und `issues_per_team` Issues je
/// Team. Alles erfunden.
pub fn scope_workspace(issues_per_team: i64) -> Value {
    let users = json!([
        { "id": "u-alice", "name": "Alice Admin", "displayName": "alice", "email": "alice@acme.test", "admin": true, "active": true, "createdAt": at(0), "updatedAt": at(0) },
        { "id": "u-erin", "name": "Erin Extern", "displayName": "erin", "email": "erin@partner.test", "admin": false, "active": true, "createdAt": at(0), "updatedAt": at(0) },
        { "id": "u-frank", "name": "Frank Fremd", "displayName": "frank", "email": "frank@partner.test", "admin": false, "active": true, "createdAt": at(0), "updatedAt": at(0) },
        { "id": "u-gina", "name": "Gina Unbeteiligt", "displayName": "gina", "email": "gina@partner.test", "admin": false, "active": true, "createdAt": at(0), "updatedAt": at(0) },
        { "id": "u-bot", "name": "Helper Agent", "displayName": "helper-agent", "email": "agent@partner.test", "admin": false, "active": true, "createdAt": at(0), "updatedAt": at(0) },
    ]);
    let team = |idv: &str, key: &str, name: &str| {
        json!({ "id": idv, "key": key, "name": name, "visibility": "public", "timezone": "Europe/Berlin",
                "createdAt": at(0), "updatedAt": at(0), "cyclesEnabled": true, "cycleDuration": 2.0, "cycleStartDay": 1.0,
                "upcomingCycleCount": 2.0, "triageEnabled": key == "KOL" })
    };
    let teams = json!([
        team("t-kol", "KOL", "KollegePlus"),
        team("t-pln", "PLN", "Planung")
    ]);
    let memberships = json!([
        { "id": "m-1", "user": id("u-alice"), "team": id("t-kol"), "owner": true, "createdAt": at(0) },
        { "id": "m-2", "user": id("u-erin"), "team": id("t-kol"), "owner": false, "createdAt": at(0) },
        { "id": "m-3", "user": id("u-frank"), "team": id("t-pln"), "owner": false, "createdAt": at(0) },
    ]);
    // Status je Team: (Slug, Name, Typ, Farbe)
    let kol_states = [
        ("triage", "Eingang", "triage", "#BEC2C8"),
        ("backlog", "Backlog", "backlog", "#E2E2E2"),
        ("ready", "Bereit", "unstarted", "#E2E2E2"),
        ("next", "Als Naechstes", "unstarted", "#F2C94C"),
        ("dev", "In Entwicklung", "started", "#F2994A"),
        ("review", "Im Review", "started", "#F2994A"),
        ("qa", "Abnahme", "started", "#BB87FC"),
        ("blocked", "Blockiert", "started", "#EB5757"),
        ("staging", "Auf Staging", "started", "#26B5CE"),
        ("done", "Erledigt", "completed", "#5E6AD2"),
        ("released", "Ausgeliefert", "completed", "#4CB782"),
        ("archived", "Abgenommen", "completed", "#4EA7FC"),
        ("dup", "Duplicate", "canceled", "#95A2B3"),
    ];
    let pln_states = [
        ("idea", "Idee", "backlog", "#BEC2C8"),
        ("later", "Spaeter", "backlog", "#E2E2E2"),
        ("todo", "Geplant", "unstarted", "#E2E2E2"),
        ("soon", "Demnaechst", "unstarted", "#F2C94C"),
        ("work", "In Arbeit", "started", "#F2994A"),
        ("wait", "Wartet auf Kunde", "started", "#EB5757"),
        ("check", "Pruefung", "started", "#BB87FC"),
        ("fin", "Fertig", "completed", "#5E6AD2"),
        ("paid", "Abgerechnet", "completed", "#4CB782"),
        ("drop", "Verworfen", "canceled", "#95A2B3"),
        ("dup", "duplicate", "canceled", "#95A2B3"),
    ];
    let mut states: Vec<Value> = Vec::new();
    for (team_id, prefix, defs) in [
        ("t-kol", "kol", &kol_states[..]),
        ("t-pln", "pln", &pln_states[..]),
    ] {
        for (i, (slug, name, ty, color)) in defs.iter().enumerate() {
            states.push(json!({ "id": format!("s-{prefix}-{slug}"), "name": name, "type": ty, "color": color,
                "description": format!("{name} ({prefix})"), "position": (i as f64) * 1.5 + 0.25, "team": id(team_id),
                "createdAt": at(0), "updatedAt": at(0) }));
        }
    }
    let label_names = [
        "Bug",
        "Feature",
        "Kunde",
        "Intern",
        "Dringend",
        "Design",
        "Backend",
        "Frontend",
        "Doku",
        "Recht",
        "Finanzen",
        "Hosting",
        "Sicherheit",
        "Wartung",
        "Idee",
        "Support",
        "Altlast",
        "Pilot",
    ];
    let labels: Vec<Value> = label_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let mut l = json!({ "id": format!("l-{i}"), "name": name, "color": format!("#{:02X}{:02X}{:02X}", 40 + i * 9, 200 - i * 7, 90 + i * 5),
                "description": format!("Label {name}"), "isGroup": false, "createdAt": at(0), "updatedAt": at(0) });
            if *name == "Altlast" {
                l["retiredAt"] = json!(at(5));
            }
            l
        })
        .collect();
    let project_statuses = json!([
        { "id": "ps-backlog", "name": "Backlog", "type": "backlog", "color": "#BEC2C8", "position": 0, "indefinite": true, "createdAt": at(0), "updatedAt": at(0) },
        { "id": "ps-planned", "name": "Planned", "type": "planned", "color": "#BEC2C8", "position": 1, "indefinite": true, "createdAt": at(0), "updatedAt": at(0) },
        { "id": "ps-started", "name": "In Progress", "type": "started", "color": "#F2C94C", "position": 2, "indefinite": false, "createdAt": at(0), "updatedAt": at(0) },
        { "id": "ps-done", "name": "Completed", "type": "completed", "color": "#5E6AD2", "position": 3, "indefinite": false, "createdAt": at(0), "updatedAt": at(0) },
        { "id": "ps-canceled", "name": "Canceled", "type": "canceled", "color": "#95A2B3", "position": 4, "indefinite": false, "createdAt": at(0), "updatedAt": at(0) },
    ]);
    let status_ids = [
        "ps-backlog",
        "ps-planned",
        "ps-started",
        "ps-done",
        "ps-canceled",
    ];
    // 20 Projekte: 0–4 über beide Teams, 5–12 nur KOL, 13–19 nur PLN.
    let projects: Vec<Value> = (0..20)
        .map(|i| {
            let teams = match i {
                0..=4 => vec![id("t-kol"), id("t-pln")],
                5..=12 => vec![id("t-kol")],
                _ => vec![id("t-pln")],
            };
            let mut p = json!({ "id": format!("p-{i}"), "name": format!("Projekt {i:02}"), "color": "#4F46E5",
                "status": { "id": status_ids[i % 5], "name": "x" }, "priority": (i % 5) as i64,
                "lead": id(if i % 2 == 0 { "u-alice" } else { "u-erin" }), "slugId": format!("{:012x}", 0xabc000 + i),
                "teams": teams, "members": [id("u-alice"), id("u-frank")], "labels": [],
                "health": "onTrack", "createdAt": at(10 + i as i64), "updatedAt": at(10 + i as i64) });
            if i == 3 {
                p["health"] = json!("somethingNew");
            }
            p
        })
        .collect();
    let milestones: Vec<Value> = (0..20)
        .step_by(4)
        .map(|i| {
            json!({ "id": format!("ms-{i}"), "name": format!("Meilenstein {i}"), "project": id(&format!("p-{i}")),
                    "targetDate": "2026-12-01", "sortOrder": 1.0, "createdAt": at(30), "updatedAt": at(30) })
        })
        .collect();
    let project_updates: Vec<Value> = (0..20)
        .step_by(5)
        .map(|i| {
            json!({ "id": format!("pu-{i}"), "body": format!("Stand Projekt {i}"), "health": "onTrack", "user": id("u-alice"),
                    "project": id(&format!("p-{i}")), "createdAt": at(40), "updatedAt": at(40) })
        })
        .collect();
    let mut cycles: Vec<Value> = Vec::new();
    for (team_id, prefix) in [("t-kol", "kol"), ("t-pln", "pln")] {
        for n in 1..=3i64 {
            cycles.push(json!({ "id": format!("c-{prefix}-{n}"), "number": n, "startsAt": at(24 * 14 * (n - 1)),
                "endsAt": at(24 * 14 * n), "team": id(team_id), "createdAt": at(0), "updatedAt": at(0) }));
        }
    }
    // Issues, Kommentare, Anhänge, Relationen
    let mut issues: Vec<Value> = Vec::new();
    let mut comments: Vec<Value> = Vec::new();
    let mut attachments: Vec<Value> = Vec::new();
    let mut relations: Vec<Value> = Vec::new();
    let people = ["u-alice", "u-erin", "u-frank", "u-bot"];
    for (team_id, key, prefix, defs, own_projects) in [
        (
            "t-kol",
            "KOL",
            "kol",
            &kol_states[..],
            (0..13).collect::<Vec<usize>>(),
        ),
        (
            "t-pln",
            "PLN",
            "pln",
            &pln_states[..],
            (0..5).chain(13..20).collect(),
        ),
    ] {
        for n in 1..=issues_per_team {
            let nu = n as usize;
            let issue_id = format!("i-{prefix}-{n}");
            let state = defs[nu % defs.len()].0;
            let project = if nu % 3 == 0 {
                Value::Null
            } else {
                id(&format!("p-{}", own_projects[nu % own_projects.len()]))
            };
            let mut labels = vec![id(&format!("l-{}", nu % 18))];
            if nu % 4 == 0 {
                labels.push(id(&format!("l-{}", (nu + 5) % 18)));
            }
            let mut issue = json!({ "id": issue_id, "number": n, "identifier": format!("{key}-{n}"),
                "url": format!("https://linear.app/acme/issue/{key}-{n}/slug"), "title": format!("Aufgabe {n} ({key})"),
                "description": format!("Beschreibung {n}"), "priority": (nu % 5) as i64,
                "team": id(team_id), "state": id(&format!("s-{prefix}-{state}")), "creator": id("u-alice"),
                "project": project, "labels": labels, "createdAt": at(50 + n), "updatedAt": at(50 + n) });
            if nu % 5 != 0 {
                issue["assignee"] = id(people[nu % 3]);
            }
            if nu % 17 == 0 {
                issue["delegate"] = id("u-bot");
            }
            if nu % 23 == 0 {
                issue["slaType"] = json!("brandNewKind");
            }
            if nu % 10 == 0 && n > 1 {
                issue["parent"] = id(&format!("i-{prefix}-{}", n - 1));
            }
            issues.push(issue);
            if nu % 5 == 0 {
                comments.push(json!({ "id": format!("cm-{prefix}-{n}"), "body": format!("Kommentar zu {n}"), "user": id(people[nu % 3]),
                    "issue": id(&issue_id), "createdAt": at(60 + n), "updatedAt": at(60 + n) }));
            }
            if nu % 7 == 0 {
                attachments.push(json!({ "id": format!("at-{prefix}-{n}"), "url": format!("https://example.test/doc/{prefix}/{n}"),
                    "title": format!("Link {n}"), "issue": id(&issue_id), "creator": id("u-alice"), "createdAt": at(60 + n), "updatedAt": at(60 + n) }));
            }
            // Relationen: innerhalb des Teams, und KOL-n -> PLN-n über die Teams hinweg
            if nu % 6 == 0 && n > 2 {
                relations.push(json!({ "id": format!("r-{prefix}-{n}"), "type": "blocks", "createdAt": at(70), "updatedAt": at(70),
                    "issue": { "id": issue_id, "identifier": format!("{key}-{n}"), "url": format!("https://linear.app/acme/issue/{key}-{n}") },
                    "relatedIssue": { "id": format!("i-{prefix}-{}", n - 1), "identifier": format!("{key}-{}", n - 1), "url": format!("https://linear.app/acme/issue/{key}-{}", n - 1) } }));
            }
            if prefix == "kol" && nu % 50 == 0 {
                relations.push(json!({ "id": format!("rx-{n}"), "type": "related", "createdAt": at(70), "updatedAt": at(70),
                    "issue": { "id": issue_id, "identifier": format!("KOL-{n}"), "url": format!("https://linear.app/acme/issue/KOL-{n}") },
                    "relatedIssue": { "id": format!("i-pln-{n}"), "identifier": format!("PLN-{n}"), "url": format!("https://linear.app/acme/issue/PLN-{n}") } }));
            }
            if nu % 40 == 0 {
                relations.push(json!({ "id": format!("rm-{prefix}-{n}"), "type": "mysteryRelation", "createdAt": at(70), "updatedAt": at(70),
                    "issue": id(&issue_id), "relatedIssue": id(&format!("i-{prefix}-1")) }));
            }
        }
    }
    json!({
        "workspace": { "id": "ws-scope", "name": "Partner Workspace", "urlKey": "acme", "viewerEmail": "alice@acme.test" },
        "entities": {
            "User": users, "Team": teams, "TeamMembership": memberships, "WorkflowState": states,
            "IssueLabel": labels, "ProjectStatus": project_statuses, "Cycle": cycles, "Project": projects,
            "ProjectMilestone": milestones, "ProjectUpdate": project_updates, "Issue": issues,
            "IssueRelation": relations, "Comment": comments, "Attachment": attachments,
        },
    })
}
