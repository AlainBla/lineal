#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP33: Mehrfachaktionen, Label-Merge/-Delete, Issue-Erwähnungen, Duplikat-Abos, `subscribed`.

use lineal_app::ports::Store;
use lineal_app::ports::repos::label::LabelKind;
use lineal_app::usecases::comment::{CreateCommentInput, ParentSpec};
use lineal_app::usecases::issue::{
    BatchPatch, BatchUpdateInput, CreateIssueInput, DescriptionEdit, RelationChanges,
    UpdateIssueInput,
};
use lineal_app::usecases::label::SaveLabelInput;
use lineal_app::{App, ErrorKind, RequestCtx};
use lineal_db::PgStore;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;

async fn setup(pool: PgPool) -> (Arc<App>, SeedData) {
    let store = PgStore::new(pool);
    let data = SeedData::acme();
    let mut tx = store.begin(data.org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    (test_app(Arc::new(store)), data)
}

fn alice_id() -> lineal_domain::ids::UserId {
    SeedData::acme().alice
}
fn alice() -> RequestCtx {
    ctx_for(SeedUser::Alice)
}
fn bob() -> RequestCtx {
    ctx_for(SeedUser::Bob)
}

async fn issue(app: &App, ctx: &RequestCtx, team: &str, title: &str) -> String {
    app.create_issue(
        ctx,
        CreateIssueInput {
            team: team.into(),
            title: Some(title.into()),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .issue
    .identifier
}

fn batch(ids: &[&str], patch: BatchPatch) -> BatchUpdateInput {
    BatchUpdateInput {
        ids: ids.iter().map(|s| (*s).to_owned()).collect(),
        patch,
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn batch_update_changes_state_priority_and_reports_failures(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let a = issue(&app, &alice(), "ENG", "Eins").await;
    let b = issue(&app, &alice(), "ENG", "Zwei").await;
    let c = issue(&app, &alice(), "ENG", "Drei").await;
    let hidden = issue(&app, &bob(), "DES", "Geheim").await;
    let res = app
        .issue_batch_update(
            &alice(),
            batch(
                &[&a, &b, &c, &hidden, "ENG-999"],
                BatchPatch {
                    state: Some("In Progress".into()),
                    priority: Some(2),
                    ..Default::default()
                },
            ),
        )
        .await
        .unwrap();
    assert_eq!(res.updated.len(), 3);
    assert!(
        res.updated
            .iter()
            .all(|v| v.extras.state_name == "In Progress")
    );
    assert!(res.updated.iter().all(|v| v.issue.priority.value() == 2));
    // Unsichtbar und unbekannt sind nicht unterscheidbar.
    assert_eq!(res.failures.len(), 2);
    assert!(res.failures.iter().all(|f| f.code == "not_found"));
}

#[sqlx::test(migrations = "./migrations")]
async fn batch_update_validates_input(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let a = issue(&app, &alice(), "ENG", "Eins").await;
    let kind = |r: Result<_, lineal_app::AppError>| -> ErrorKind {
        let r: Result<lineal_app::usecases::issue::BatchResult, _> = r;
        r.unwrap_err().kind
    };
    assert_eq!(
        kind(
            app.issue_batch_update(&alice(), batch(&[], BatchPatch::default()))
                .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            app.issue_batch_update(&alice(), batch(&[&a], BatchPatch::default()))
                .await
        ),
        ErrorKind::InvalidRequest
    );
    let many: Vec<String> = (0..251).map(|i| format!("ENG-{}", i + 1)).collect();
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    assert_eq!(
        kind(
            app.issue_batch_update(
                &alice(),
                batch(
                    &refs,
                    BatchPatch {
                        priority: Some(1),
                        ..Default::default()
                    }
                )
            )
            .await
        ),
        ErrorKind::LimitExceeded
    );
    assert_eq!(
        kind(
            app.issue_batch_update(
                &alice(),
                batch(
                    &[&a],
                    BatchPatch {
                        team: Some("WEB".into()),
                        add_labels: vec!["Bug".into()],
                        ..Default::default()
                    }
                )
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn batch_labels_archive_and_delete(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let a = issue(&app, &alice(), "ENG", "Eins").await;
    let b = issue(&app, &alice(), "ENG", "Zwei").await;
    let res = app
        .issue_batch_update(
            &alice(),
            batch(
                &[&a, &b],
                BatchPatch {
                    add_labels: vec!["Bug".into()],
                    ..Default::default()
                },
            ),
        )
        .await
        .unwrap();
    assert!(res.updated.iter().all(|v| v.extras.label_ids.len() == 1));
    let res = app
        .issue_batch_update(
            &alice(),
            batch(
                &[&a, &b],
                BatchPatch {
                    remove_labels: vec!["Bug".into()],
                    ..Default::default()
                },
            ),
        )
        .await
        .unwrap();
    assert!(res.updated.iter().all(|v| v.extras.label_ids.is_empty()));
    // Archivieren: read-only, wiederherstellbar.
    let res = app
        .issue_batch_update(
            &alice(),
            batch(
                &[&a],
                BatchPatch {
                    archive: true,
                    ..Default::default()
                },
            ),
        )
        .await
        .unwrap();
    assert!(res.updated[0].issue.archived_at.is_some());
    assert!(!res.updated[0].issue.trashed);
    let e = app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                id: a.clone(),
                priority: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    app.restore_issue(&alice(), &a).await.unwrap();
    // Löschen = Papierkorb; Kombination mit Änderungen verboten.
    let res = app
        .issue_batch_update(
            &alice(),
            batch(
                &[&b],
                BatchPatch {
                    delete: true,
                    ..Default::default()
                },
            ),
        )
        .await
        .unwrap();
    assert!(res.updated[0].issue.trashed);
    assert!(
        app.issue_batch_update(
            &alice(),
            batch(
                &[&a],
                BatchPatch {
                    delete: true,
                    priority: Some(1),
                    ..Default::default()
                },
            ),
        )
        .await
        .is_err()
    );
    // Gast ohne Schreibrecht im Team ENG: Teilfehler statt Erfolg.
    let res = app
        .issue_batch_update(
            &ctx_for(SeedUser::Carol),
            batch(
                &[&a],
                BatchPatch {
                    priority: Some(1),
                    ..Default::default()
                },
            ),
        )
        .await
        .unwrap();
    assert!(res.updated.is_empty());
    assert_eq!(res.failures.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn bulk_move_with_and_without_sub_issues(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let parent = issue(&app, &alice(), "ENG", "Eltern").await;
    let child = app
        .create_issue(
            &alice(),
            CreateIssueInput {
                team: "ENG".into(),
                title: Some("Kind".into()),
                parent: Some(parent.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .issue
        .identifier;
    let solo = issue(&app, &alice(), "ENG", "Solo").await;
    // Ohne moveSubIssues bleibt das Kind im alten Team.
    let res = app
        .issue_batch_update(
            &alice(),
            batch(
                &[&solo, &parent],
                BatchPatch {
                    team: Some("WEB".into()),
                    ..Default::default()
                },
            ),
        )
        .await
        .unwrap();
    assert_eq!(res.updated.len(), 2);
    assert!(
        res.updated
            .iter()
            .all(|v| v.issue.identifier.starts_with("WEB-"))
    );
    let c = app.get_issue(&alice(), &child).await.unwrap();
    assert_eq!(c.view.issue.identifier, child);
    // Zurück, diesmal mit Sub-Issues.
    let moved = app
        .update_issue(
            &alice(),
            UpdateIssueInput {
                id: parent.clone(),
                team: Some("DES".into()),
                ..Default::default()
            },
        )
        .await;
    assert!(moved.is_err(), "DES ist für Alice nicht sichtbar");
    let res = app
        .issue_batch_update(
            &alice(),
            batch(
                &[&parent],
                BatchPatch {
                    team: Some("WEB".into()),
                    move_sub_issues: true,
                    ..Default::default()
                },
            ),
        )
        .await
        .unwrap();
    // Das Eltern-Issue ist schon im Zielteam: Kein Wechsel, aber Kinder dürfen nachziehen.
    assert!(res.failures.is_empty());
    let c = app.get_issue(&alice(), &child).await.unwrap();
    assert!(
        c.view.issue.identifier.starts_with("WEB-"),
        "child moved: {}",
        c.view.issue.identifier
    );
    assert!(c.view.previous_identifiers.contains(&child));
}

#[sqlx::test(migrations = "./migrations")]
async fn issue_mentions_create_related_relations_without_leaks(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let a = issue(&app, &alice(), "ENG", "Eins").await;
    let b = issue(&app, &alice(), "ENG", "Zwei").await;
    let c = issue(&app, &alice(), "ENG", "Drei").await;
    let secret = issue(&app, &bob(), "DES", "Geheim").await;
    app.update_issue(
        &alice(),
        UpdateIssueInput {
            id: b.clone(),
            description: Some(DescriptionEdit::Set(format!(
                "Siehe {a} und {secret} sowie ENG-4711."
            ))),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let d = app.get_issue(&alice(), &a).await.unwrap();
    assert_eq!(d.relations.related_to.len(), 1);
    assert_eq!(d.relations.related_to[0].identifier, b);
    let db = app.get_issue(&alice(), &b).await.unwrap();
    assert_eq!(db.relations.related_to.len(), 1, "nur das sichtbare Issue");
    // Bob sieht DES-1 weiterhin ohne Relation zu ENG-2.
    let ds = app.get_issue(&bob(), &secret).await.unwrap();
    assert!(ds.relations.related_to.is_empty());
    // Bestehende Relation wird nicht überschrieben.
    app.update_issue(
        &alice(),
        UpdateIssueInput {
            id: c.clone(),
            relations: RelationChanges {
                blocks: vec![a.clone()],
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await
    .unwrap();
    app.update_issue(
        &alice(),
        UpdateIssueInput {
            id: c.clone(),
            description: Some(DescriptionEdit::Set(format!("wie {a}"))),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let dc = app.get_issue(&alice(), &c).await.unwrap();
    assert!(dc.relations.related_to.is_empty());
    assert_eq!(dc.relations.blocks.len(), 1);
    // Entfernen der Erwähnung entfernt die Relation nicht.
    app.update_issue(
        &alice(),
        UpdateIssueInput {
            id: b.clone(),
            description: Some(DescriptionEdit::Set("nichts mehr".into())),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        app.get_issue(&alice(), &a)
            .await
            .unwrap()
            .relations
            .related_to
            .len(),
        1
    );
    // Kommentare erzeugen ebenfalls Relationen.
    let d = issue(&app, &alice(), "ENG", "Vier").await;
    app.create_comment(
        &alice(),
        CreateCommentInput {
            parent: Some(ParentSpec::Issue(d.clone())),
            body: format!("Doppelt zu {c}?"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let dd = app.get_issue(&alice(), &d).await.unwrap();
    assert_eq!(dd.relations.related_to.len(), 1);
    assert_eq!(dd.relations.related_to[0].identifier, c);
}

#[sqlx::test(migrations = "./migrations")]
async fn comment_mention_links_issues_for_members_with_edit_right(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let a = issue(&app, &alice(), "WEB", "Eins").await;
    let b = issue(&app, &alice(), "WEB", "Zwei").await;
    // Carol ist Gast mit Mitgliedschaft in WEB und darf dort bearbeiten: Relation entsteht.
    app.create_comment(
        &ctx_for(SeedUser::Carol),
        CreateCommentInput {
            parent: Some(ParentSpec::Issue(a.clone())),
            body: format!("siehe {b}"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let d = app.get_issue(&alice(), &a).await.unwrap();
    assert_eq!(d.relations.related_to.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn duplicate_transfers_subscribers_and_subscribed_reflects_state(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let orig = issue(&app, &alice(), "ENG", "Original").await;
    let dup = issue(&app, &alice(), "ENG", "Duplikat").await;
    assert!(!app.issue_subscribed(&bob(), &dup).await.unwrap());
    app.subscribe_issue(&bob(), &dup, None).await.unwrap();
    assert!(app.issue_subscribed(&bob(), &dup).await.unwrap());
    assert!(!app.issue_subscribed(&bob(), &orig).await.unwrap());
    app.update_issue(
        &alice(),
        UpdateIssueInput {
            id: dup.clone(),
            relations: RelationChanges {
                duplicate_of: Some(Some(orig.clone())),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(app.issue_subscribed(&bob(), &orig).await.unwrap());
    assert!(
        app.issue_subscribed(&bob(), &dup).await.unwrap(),
        "Abo bleibt"
    );
    app.unsubscribe_issue(&bob(), &orig, None).await.unwrap();
    assert!(!app.issue_subscribed(&bob(), &orig).await.unwrap());
}

#[sqlx::test(migrations = "./migrations")]
async fn duplicate_does_not_subscribe_users_without_access(pool: PgPool) {
    let (app, _) = setup(pool).await;
    // Carol (Gast) sieht nur WEB; Original liegt in ENG (nicht sichtbar für sie).
    let web = issue(&app, &alice(), "WEB", "Web-Dup").await;
    let eng = issue(&app, &alice(), "ENG", "Eng-Original").await;
    app.subscribe_issue(&ctx_for(SeedUser::Carol), &web, None)
        .await
        .unwrap();
    app.update_issue(
        &alice(),
        UpdateIssueInput {
            id: web.clone(),
            relations: RelationChanges {
                duplicate_of: Some(Some(eng.clone())),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let subs = app.list_issue_subscriptions(&alice(), &eng).await.unwrap();
    assert!(subs.iter().all(|s| s.reason != "addedByOther"));
}

async fn save_label(app: &App, ctx: &RequestCtx, name: &str, team: Option<&str>) -> String {
    app.save_label(
        ctx,
        LabelKind::Issue,
        SaveLabelInput {
            name: Some(name.into()),
            team_id: team.map(Into::into),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .label
    .id
    .to_string()
}

#[sqlx::test(migrations = "./migrations")]
async fn label_merge_moves_issues_and_deletes_source(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let src = save_label(&app, &alice(), "Defekt", None).await;
    let a = issue(&app, &alice(), "ENG", "Eins").await;
    let b = issue(&app, &alice(), "ENG", "Zwei").await;
    let c = issue(&app, &alice(), "ENG", "Drei").await;
    for (i, labels) in [
        (&a, vec!["Defekt"]),
        (&b, vec!["Defekt", "Bug"]),
        (&c, vec!["Feature"]),
    ] {
        app.update_issue(
            &alice(),
            UpdateIssueInput {
                id: i.clone(),
                add_labels: labels.into_iter().map(String::from).collect(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    assert_eq!(app.issue_label_usage(&alice(), &src).await.unwrap(), 2);
    // Ein Gast darf Workspace-Labels nicht verwalten, ein normales Mitglied nicht zusammenführen.
    let carol = ctx_for(SeedUser::Carol);
    assert!(app.merge_issue_label(&carol, &src, "Bug").await.is_err());
    assert!(app.issue_label_usage(&carol, &src).await.is_err());
    assert_eq!(
        app.merge_issue_label(&bob(), &src, "Bug")
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Forbidden
    );
    assert_eq!(
        app.merge_issue_label(&alice(), &src, &src)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    let r = app.merge_issue_label(&alice(), &src, "Bug").await.unwrap();
    assert_eq!(r.affected_issues, 2);
    let bug = r.target.unwrap().label.id;
    for i in [&a, &b] {
        let v = app.get_issue(&alice(), i).await.unwrap();
        assert_eq!(v.view.extras.label_ids, vec![bug], "genau ein Bug-Label");
    }
    assert_eq!(
        app.issue_label_usage(&alice(), &src)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn label_delete_removes_from_issues_and_checks_rights(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let l = save_label(&app, &alice(), "Weg damit", None).await;
    let a = issue(&app, &bob(), "ENG", "Eins").await;
    app.update_issue(
        &bob(),
        UpdateIssueInput {
            id: a.clone(),
            add_labels: vec!["Weg damit".into()],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(app.issue_label_usage(&alice(), &l).await.unwrap(), 1);
    assert!(
        app.delete_issue_label(&ctx_for(SeedUser::Carol), &l)
            .await
            .is_err()
    );
    // Löschen ist Admins/Team-Owner vorbehalten, auch wenn Mitglieder Labels verwalten dürfen.
    assert_eq!(
        app.delete_issue_label(&bob(), &l).await.unwrap_err().kind,
        ErrorKind::Forbidden
    );
    let r = app.delete_issue_label(&alice(), &l).await.unwrap();
    assert_eq!(r.affected_issues, 1);
    let v = app.get_issue(&alice(), &a).await.unwrap();
    assert!(v.view.extras.label_ids.is_empty());
    // Eintrag des löschenden Users (anderer Akteur: nicht mit der Erstellung verrechnet).
    let h = app.issue_history(&alice(), &a).await.unwrap();
    assert!(
        h.iter()
            .any(|e| e.actor_id == Some(alice_id()) && e.removed_label_ids.is_some()),
        "history of the delete: {h:?}"
    );
    assert_eq!(
        app.delete_issue_label(&alice(), &l).await.unwrap_err().kind,
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn label_group_delete_removes_children(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let g = app
        .save_label(
            &alice(),
            LabelKind::Issue,
            SaveLabelInput {
                name: Some("Bereich".into()),
                is_group: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    app.save_label(
        &alice(),
        LabelKind::Issue,
        SaveLabelInput {
            name: Some("Kind".into()),
            parent: Some(Some(g.label.id.to_string())),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let a = issue(&app, &alice(), "ENG", "Eins").await;
    app.update_issue(
        &alice(),
        UpdateIssueInput {
            id: a.clone(),
            add_labels: vec!["Kind".into()],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let r = app
        .delete_issue_label(&alice(), &g.label.id.to_string())
        .await
        .unwrap();
    assert_eq!(r.affected_issues, 1);
    assert!(
        app.get_issue(&alice(), &a)
            .await
            .unwrap()
            .view
            .extras
            .label_ids
            .is_empty()
    );
}
