//! WP35: View-Abo-Benachrichtigungen (`ViewSubscriptions`, docs/05 §4.7) gegen echte Datenbank
//! (Seed `acme`). Geprüft wird der registrierte Konsument aus `registry::consumers()`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::Utc;
use lineal_app::App;
use lineal_app::ports::Store;
use lineal_app::ports::repos::StoredEvent;
use lineal_app::ports::repos::view::ViewModel;
use lineal_app::usecases::view::{CreateViewInput, UpdateViewInput};
use lineal_db::PgStore;
use lineal_domain::events::{DomainEvent, EventOrigin, IssuePrevious};
use lineal_domain::ids::*;
use lineal_jobs::registry;
use lineal_testkit::seed::identity::seed_workflow_state_ids;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

struct Env {
    app: Arc<App>,
    pool: PgPool,
    d: SeedData,
    bug: Uuid,
    next: i32,
}

async fn env(pool: PgPool) -> Env {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let d = SeedData::acme();
    let bug: Uuid = sqlx::query_scalar(
        "SELECT id FROM issue_label WHERE organization_id = $1 AND team_id IS NULL AND name = 'Bug'",
    )
    .bind(d.org.0)
    .fetch_one(&pool)
    .await
    .unwrap();
    Env {
        app: test_app(Arc::new(store)),
        pool,
        d,
        bug,
        next: 700,
    }
}

impl Env {
    /// Der in der Registry eingetragene Konsument `notifications`.
    async fn handle(&self, actor: UserId, event: DomainEvent) {
        let ev = StoredEvent {
            id: EventId::new_v7(),
            org_id: self.d.org,
            origin: EventOrigin::User,
            actor_id: Some(actor),
            sync_id: 1,
            event,
            created_at: Utc::now(),
        };
        let consumer = registry::consumers()
            .into_iter()
            .find(|c| c.name() == "notifications")
            .unwrap();
        consumer.handle(&self.app, &ev).await.unwrap();
    }

    async fn issue(&mut self, team: TeamId, slot: u128, creator: UserId, bug: bool) -> IssueId {
        self.next += 1;
        let id = IssueId::new_v7();
        sqlx::query(
            "INSERT INTO issue (id, organization_id, team_id, number, title, state_id, creator_id) \
             VALUES ($1,$2,$3,$4,'Login schlägt fehl',$5,$6)",
        )
        .bind(id.0)
        .bind(self.d.org.0)
        .bind(team.0)
        .bind(self.next)
        .bind(seed_workflow_state_ids(slot)[2].0)
        .bind(creator.0)
        .execute(&self.pool)
        .await
        .unwrap();
        if bug {
            sqlx::query(
                "INSERT INTO issue_to_label (organization_id, issue_id, label_id) VALUES ($1,$2,$3)",
            )
            .bind(self.d.org.0)
            .bind(id.0)
            .bind(self.bug)
            .execute(&self.pool)
            .await
            .unwrap();
        }
        id
    }

    async fn created(&self, actor: UserId, issue: IssueId, team: TeamId) {
        self.handle(
            actor,
            DomainEvent::IssueCreated {
                issue_id: issue,
                team_id: team,
                mentioned_user_ids: vec![],
            },
        )
        .await;
    }

    async fn view(&self, owner: SeedUser, name: &str, team: Option<&str>) -> String {
        let v = self
            .app
            .create_view(
                &ctx_for(owner),
                CreateViewInput {
                    name: name.into(),
                    description: None,
                    icon: None,
                    color: None,
                    model: ViewModel::Issue,
                    filter: json!({ "labels": { "some": { "id": { "in": [self.bug.to_string()] } } } }),
                    shared: true,
                    team: team.map(str::to_owned),
                    project: None,
                    initiative: None,
                    preferences: None,
                },
            )
            .await
            .unwrap();
        v.view.id.0.to_string()
    }

    async fn subscribe(&self, user: SeedUser, view: &str, types: &[&str]) {
        self.app
            .set_view_subscription(
                &ctx_for(user),
                view,
                true,
                Some(types.iter().map(|t| (*t).to_owned()).collect()),
            )
            .await
            .unwrap();
    }

    async fn kinds(&self, user: UserId) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT type FROM notification WHERE user_id = $1 ORDER BY created_at, id",
        )
        .bind(user.0)
        .fetch_all(&self.pool)
        .await
        .unwrap()
    }

    async fn set_state(&self, issue: IssueId, slot: u128, index: usize) {
        sqlx::query("UPDATE issue SET state_id = $2 WHERE id = $1")
            .bind(issue.0)
            .bind(seed_workflow_state_ids(slot)[index].0)
            .execute(&self.pool)
            .await
            .unwrap();
    }
}

fn state_changed(issue: IssueId, team: TeamId, previous: &str) -> DomainEvent {
    DomainEvent::IssueUpdated {
        issue_id: issue,
        team_id: team,
        changed: vec!["stateId".into()],
        mentioned_user_ids: vec![],
        previous: Some(IssuePrevious {
            assignee_id: None,
            state_type: Some(previous.into()),
        }),
    }
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn new_issue_matching_the_view_notifies_subscribers_but_not_the_actor(pool: PgPool) {
    let mut e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let view = e.view(SeedUser::Alice, "Bugs in ENG", Some("ENG")).await;
    e.subscribe(SeedUser::Bob, &view, &["issueAddedToView"])
        .await;
    e.subscribe(SeedUser::Alice, &view, &["issueAddedToView"])
        .await;

    // Kein Bug: erfüllt den Filter nicht.
    let plain = e.issue(e.d.eng, 1, alice, false).await;
    e.created(alice, plain, e.d.eng).await;
    assert!(e.kinds(bob).await.is_empty());

    let bug = e.issue(e.d.eng, 1, alice, true).await;
    e.created(alice, bug, e.d.eng).await;
    assert_eq!(e.kinds(bob).await, vec!["issueAddedToView"]);
    // Alice ist die Akteurin.
    assert!(e.kinds(alice).await.is_empty());

    // Bug in WEB (Sub-Team): der Team-Scope der View gilt ohne Sub-Teams.
    let web = e.issue(e.d.web, 2, alice, true).await;
    e.created(alice, web, e.d.web).await;
    assert_eq!(e.kinds(bob).await.len(), 1);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn issues_in_teams_the_subscriber_cannot_see_stay_silent(pool: PgPool) {
    let mut e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    // Workspace-weite View auf Bugs; DES ist privat, Alice (Owner ohne Mitgliedschaft) sieht es nicht.
    let view = e.view(SeedUser::Alice, "Alle Bugs", None).await;
    e.subscribe(SeedUser::Alice, &view, &["issueAddedToView"])
        .await;
    e.subscribe(SeedUser::Bob, &view, &["issueAddedToView"])
        .await;

    let des = e.issue(e.d.des, 3, bob, true).await;
    e.created(bob, des, e.d.des).await;
    assert!(e.kinds(alice).await.is_empty());

    // Gegenprobe: derselbe Bug in ENG von Bob erreicht Alice.
    let eng = e.issue(e.d.eng, 1, bob, true).await;
    e.created(bob, eng, e.d.eng).await;
    assert_eq!(e.kinds(alice).await, vec!["issueAddedToView"]);
    assert!(e.kinds(bob).await.is_empty());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn closing_an_issue_of_the_view_sends_status_notifications(pool: PgPool) {
    let mut e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let view = e.view(SeedUser::Alice, "Bugs in ENG", Some("ENG")).await;
    e.subscribe(SeedUser::Bob, &view, &["issueStatusChanged"])
        .await;
    let bug = e.issue(e.d.eng, 1, alice, true).await;

    // In Bearbeitung → Todo: kein Abschluss, Bob hat nur issueStatusChanged abonniert.
    e.set_state(bug, 1, 1).await;
    e.handle(alice, state_changed(bug, e.d.eng, "started"))
        .await;
    assert!(e.kinds(bob).await.is_empty());

    // → Done.
    e.set_state(bug, 1, 3).await;
    e.handle(alice, state_changed(bug, e.d.eng, "unstarted"))
        .await;
    assert_eq!(e.kinds(bob).await, vec!["issueStatusChanged"]);

    // Kein Bug in der View: Statuswechsel bleibt ohne Wirkung.
    let other = e.issue(e.d.eng, 1, alice, false).await;
    e.set_state(other, 1, 3).await;
    e.handle(alice, state_changed(other, e.d.eng, "started"))
        .await;
    assert_eq!(e.kinds(bob).await.len(), 1);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn status_changed_all_reports_every_transition_once_per_issue_change(pool: PgPool) {
    let mut e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let view = e.view(SeedUser::Alice, "Bugs in ENG", Some("ENG")).await;
    e.subscribe(SeedUser::Bob, &view, &["issueStatusChangedAll"])
        .await;
    let bug = e.issue(e.d.eng, 1, alice, true).await;
    e.set_state(bug, 1, 1).await;
    e.handle(alice, state_changed(bug, e.d.eng, "started"))
        .await;
    assert_eq!(e.kinds(bob).await, vec!["issueStatusChangedAll"]);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn updates_only_add_to_the_view_when_a_filtered_field_changed(pool: PgPool) {
    let mut e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let view = e.view(SeedUser::Alice, "Bugs in ENG", Some("ENG")).await;
    e.subscribe(SeedUser::Bob, &view, &["issueAddedToView"])
        .await;
    let bug = e.issue(e.d.eng, 1, alice, true).await;
    let update = |changed: &[&str]| DomainEvent::IssueUpdated {
        issue_id: bug,
        team_id: e.d.eng,
        changed: changed.iter().map(|c| (*c).to_owned()).collect(),
        mentioned_user_ids: vec![],
        previous: None,
    };
    // Titel ist im Filter nicht verwendet: das Issue kann dadurch nicht in die View gekommen sein.
    e.handle(alice, update(&["title"])).await;
    assert!(e.kinds(bob).await.is_empty());
    // Labels sind im Filter: Issue erfüllt ihn jetzt.
    e.handle(alice, update(&["labelIds"])).await;
    assert_eq!(e.kinds(bob).await, vec!["issueAddedToView"]);
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn a_view_that_is_no_longer_shared_notifies_nobody_but_the_owner(pool: PgPool) {
    let mut e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let view = e.view(SeedUser::Alice, "Bugs in ENG", Some("ENG")).await;
    e.subscribe(SeedUser::Bob, &view, &["issueAddedToView"])
        .await;
    e.subscribe(SeedUser::Alice, &view, &["issueAddedToView"])
        .await;

    e.app
        .update_view(
            &ctx_for(SeedUser::Alice),
            UpdateViewInput {
                reference: view.clone(),
                shared: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let bug = e.issue(e.d.eng, 1, alice, true).await;
    e.created(alice, bug, e.d.eng).await;
    assert!(e.kinds(bob).await.is_empty());

    // Selbst ein (fälschlich) aktives Abo löst nichts aus: die View ist für Bob unsichtbar.
    sqlx::query(
        "UPDATE notification_subscription SET active = true WHERE subscriber_id = $1 AND target_type = 'customView'",
    )
    .bind(bob.0)
    .execute(&e.pool)
    .await
    .unwrap();
    let again = e.issue(e.d.eng, 1, alice, true).await;
    e.created(alice, again, e.d.eng).await;
    assert!(e.kinds(bob).await.is_empty());
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn moving_a_bug_into_the_team_scope_adds_it_to_the_view(pool: PgPool) {
    let mut e = env(pool).await;
    let (alice, bob) = (e.d.alice, e.d.bob);
    let view = e.view(SeedUser::Alice, "Bugs in ENG", Some("ENG")).await;
    e.subscribe(SeedUser::Bob, &view, &["issueAddedToView"])
        .await;
    let bug = e.issue(e.d.web, 2, alice, true).await;
    e.created(alice, bug, e.d.web).await;
    assert!(e.kinds(bob).await.is_empty());

    sqlx::query("UPDATE issue SET team_id = $2, state_id = $3 WHERE id = $1")
        .bind(bug.0)
        .bind(e.d.eng.0)
        .bind(seed_workflow_state_ids(1)[2].0)
        .execute(&e.pool)
        .await
        .unwrap();
    e.handle(
        alice,
        DomainEvent::IssueUpdated {
            issue_id: bug,
            team_id: e.d.eng,
            changed: vec!["teamId".into()],
            mentioned_user_ids: vec![],
            previous: None,
        },
    )
    .await;
    assert_eq!(e.kinds(bob).await, vec!["issueAddedToView"]);

    // Reiner Statuswechsel innerhalb desselben Typs: kein „added“.
    e.handle(alice, state_changed(bug, e.d.eng, "started"))
        .await;
    assert_eq!(e.kinds(bob).await.len(), 1);
}
