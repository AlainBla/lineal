#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP10: Dokumente gegen echte Datenbank (Seed `acme`): genau ein Parent, Reparent, Sichtbarkeit
//! über das Elternobjekt, Patch, Snapshot-Historie, Papierkorb, Markdown-Bereinigung.

use lineal_app::ports::Store;
use lineal_app::usecases::document::{
    DocumentView, ListDocumentsInput, ParentInfo, ParentRefs, SaveDocumentInput,
};
use lineal_app::usecases::issue::CreateIssueInput;
use lineal_app::{App, ErrorKind, RequestCtx, Scopes};
use lineal_db::PgStore;
use lineal_domain::rules::patch::PatchOp;
use lineal_testkit::seed::planning::seed_project_id;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;

struct Env {
    app: App,
}

async fn setup(pool: PgPool) -> Env {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let app = Arc::try_unwrap(test_app(Arc::new(store))).ok().unwrap();
    Env { app }
}

fn kind<T>(r: Result<T, lineal_app::AppError>) -> ErrorKind {
    r.err().expect("expected an error").kind
}

fn in_team(team: &str, title: &str, content: &str) -> SaveDocumentInput {
    SaveDocumentInput {
        title: Some(title.into()),
        content: Some(content.into()),
        parent: ParentRefs {
            team: Some(team.into()),
            ..Default::default()
        },
        ..Default::default()
    }
}

async fn create(env: &Env, who: SeedUser, input: SaveDocumentInput) -> DocumentView {
    env.app.save_document(&ctx_for(who), input).await.unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn create_get_list_and_slug_lookup(pool: PgPool) {
    let env = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let d = create(
        &env,
        SeedUser::Alice,
        in_team("ENG", "API Rate Limits", "## Ziel\nAbsichern."),
    )
    .await;
    assert_eq!(d.doc.title, "API Rate Limits");
    assert_eq!(d.doc.content, "## Ziel\nAbsichern.");
    assert_eq!(d.doc.slug_id.len(), 12);
    assert!(
        d.url
            .ends_with(&format!("/acme/document/api-rate-limits-{}", d.doc.slug_id))
    );
    assert!(matches!(d.parent, Some(ParentInfo::Team { .. })));

    // UUID, slugId, Titel-Slug und komplette URL lösen dasselbe Dokument auf.
    for r in [
        d.doc.id.to_string(),
        d.doc.slug_id.clone(),
        format!("stale-title-{}", d.doc.slug_id),
        d.url.clone(),
    ] {
        let got = env.app.get_document(&alice, &r).await.unwrap();
        assert_eq!(got.doc.id, d.doc.id, "{r}");
    }
    let page = env
        .app
        .list_documents(
            &alice,
            ListDocumentsInput {
                limit: 50,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    // Erste Snapshot-Version `created`.
    let h = env
        .app
        .document_history(&alice, &d.doc.id.to_string(), None, 10)
        .await
        .unwrap();
    assert_eq!(h.items.len(), 1);
    assert_eq!(h.items[0].markdown, "## Ziel\nAbsichern.");
}

#[sqlx::test(migrations = "./migrations")]
async fn exactly_one_parent_is_enforced(pool: PgPool) {
    let env = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    // Zwei Parents -> invalid_request
    let err = env
        .app
        .save_document(
            &alice,
            SaveDocumentInput {
                title: Some("X".into()),
                parent: ParentRefs {
                    team: Some("ENG".into()),
                    project: Some("P-ENG-1".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .err()
        .unwrap();
    assert_eq!(err.kind, ErrorKind::InvalidRequest);
    assert_eq!(
        err.details["parameters"],
        serde_json::json!(["project", "team"])
    );
    // Kein Parent -> invalid_request
    let none = SaveDocumentInput {
        title: Some("X".into()),
        ..Default::default()
    };
    assert_eq!(
        kind(env.app.save_document(&alice, none).await),
        ErrorKind::InvalidRequest
    );
    // `team` zählt bei `cycle` nicht mit (Auflösungshilfe): genau ein Parent.
    let cycle_doc = create(
        &env,
        SeedUser::Alice,
        SaveDocumentInput {
            title: Some("Retro".into()),
            parent: ParentRefs {
                cycle: Some("2".into()),
                team: Some("ENG".into()),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await;
    assert!(matches!(
        cycle_doc.parent,
        Some(ParentInfo::Cycle { number: 2, .. })
    ));
    assert!(cycle_doc.doc.team_id.is_some() && cycle_doc.doc.cycle_id.is_some());
    // Cycle per Nummer ohne Team ist nicht eindeutig.
    let r = env
        .app
        .save_document(
            &alice,
            SaveDocumentInput {
                title: Some("Retro".into()),
                parent: ParentRefs {
                    cycle: Some("2".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::AmbiguousReference);
    // Titel ist Pflicht (leer erlaubt).
    let r = env
        .app
        .save_document(
            &alice,
            SaveDocumentInput {
                parent: ParentRefs {
                    team: Some("ENG".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
    let empty = create(&env, SeedUser::Alice, in_team("ENG", "", "")).await;
    assert!(empty.url.contains("/untitled-"));
}

#[sqlx::test(migrations = "./migrations")]
async fn private_team_documents_are_invisible_everywhere(pool: PgPool) {
    let env = setup(pool).await;
    let secret = create(
        &env,
        SeedUser::Bob,
        in_team("DES", "Geheimes Design", "Roadmap-Details xyzzy"),
    )
    .await;
    let public = create(
        &env,
        SeedUser::Alice,
        in_team("ENG", "Oeffentlich", "xyzzy"),
    )
    .await;

    for who in [SeedUser::Alice, SeedUser::Carol] {
        let ctx = ctx_for(who);
        for r in [
            secret.doc.id.to_string(),
            secret.doc.slug_id.clone(),
            secret.url.clone(),
        ] {
            assert_eq!(
                kind(env.app.get_document(&ctx, &r).await),
                ErrorKind::NotFound,
                "{who:?} {r}"
            );
        }
        // Liste und Volltextsuche liefern das Dokument nicht.
        let all = env
            .app
            .list_documents(
                &ctx,
                ListDocumentsInput {
                    limit: 50,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(all.items.iter().all(|d| d.doc.id != secret.doc.id));
        let found = env
            .app
            .list_documents(
                &ctx,
                ListDocumentsInput {
                    query: Some("xyzzy".into()),
                    limit: 50,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(found.items.iter().all(|d| d.doc.id != secret.doc.id));
        // Teamfilter auf ein unsichtbares Team: not_found, nicht „leer“.
        let r = env
            .app
            .list_documents(
                &ctx,
                ListDocumentsInput {
                    team: Some("DES".into()),
                    limit: 50,
                    ..Default::default()
                },
            )
            .await;
        assert_eq!(kind(r), ErrorKind::NotFound);
        // Anlegen, ändern, Historie, Löschen: überall not_found.
        assert_eq!(
            kind(
                env.app
                    .save_document(&ctx, in_team("DES", "Hack", ""))
                    .await
            ),
            ErrorKind::NotFound
        );
        let upd = SaveDocumentInput {
            id: Some(secret.doc.id.to_string()),
            title: Some("Hack".into()),
            ..Default::default()
        };
        assert_eq!(
            kind(env.app.save_document(&ctx, upd).await),
            ErrorKind::NotFound
        );
        assert_eq!(
            kind(
                env.app
                    .document_history(&ctx, &secret.doc.id.to_string(), None, 10)
                    .await
            ),
            ErrorKind::NotFound
        );
        assert_eq!(
            kind(
                env.app
                    .delete_document(&ctx, &secret.doc.id.to_string())
                    .await
            ),
            ErrorKind::NotFound
        );
    }
    // Das öffentliche Dokument sieht Alice, das private nur Bob.
    let alice = ctx_for(SeedUser::Alice);
    let found = env
        .app
        .list_documents(
            &alice,
            ListDocumentsInput {
                query: Some("xyzzy".into()),
                limit: 50,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(found.items.len(), 1);
    assert_eq!(found.items[0].doc.id, public.doc.id);
    let bob = env
        .app
        .list_documents(
            &ctx_for(SeedUser::Bob),
            ListDocumentsInput {
                query: Some("xyzzy".into()),
                limit: 50,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(bob.items.len(), 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn visibility_follows_project_issue_and_initiative_parents(pool: PgPool) {
    let env = setup(pool.clone()).await;
    // Projekt P-ENG-1 (Team ENG, öffentlich): Dokument sichtbar für Bob; Carol (Gast, nur WEB) nicht.
    let pdoc = create(
        &env,
        SeedUser::Alice,
        SaveDocumentInput {
            title: Some("Checkout-Spezifikation".into()),
            parent: ParentRefs {
                project: Some("P-ENG-1".into()),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await;
    assert!(
        matches!(&pdoc.parent, Some(ParentInfo::Project { identifier, .. }) if identifier == "P-ENG-1")
    );
    env.app
        .get_document(&ctx_for(SeedUser::Bob), &pdoc.doc.id.to_string())
        .await
        .unwrap();
    assert_eq!(
        kind(
            env.app
                .get_document(&ctx_for(SeedUser::Carol), &pdoc.doc.id.to_string())
                .await
        ),
        ErrorKind::NotFound
    );
    // Initiative: Gäste sehen sie nie, Mitglieder schon.
    let idoc = create(
        &env,
        SeedUser::Alice,
        SaveDocumentInput {
            title: Some("Zielbild".into()),
            parent: ParentRefs {
                initiative: Some("I-1".into()),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await;
    env.app
        .get_document(&ctx_for(SeedUser::Bob), &idoc.doc.id.to_string())
        .await
        .unwrap();
    assert_eq!(
        kind(
            env.app
                .get_document(&ctx_for(SeedUser::Carol), &idoc.doc.id.to_string())
                .await
        ),
        ErrorKind::NotFound
    );
    // Issue in ENG; wird es in das private Team DES verschoben (hier per SQL), verliert Alice den Zugriff.
    let issue = env
        .app
        .create_issue(
            &ctx_for(SeedUser::Alice),
            CreateIssueInput {
                team: "ENG".into(),
                title: Some("Rollout".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let ddoc = create(
        &env,
        SeedUser::Alice,
        SaveDocumentInput {
            title: Some("Rollout-Checkliste".into()),
            parent: ParentRefs {
                issue: Some(issue.issue.identifier.clone()),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await;
    env.app
        .get_document(&ctx_for(SeedUser::Bob), &ddoc.doc.id.to_string())
        .await
        .unwrap();
    sqlx::query("UPDATE issue SET team_id = $1, state_id = $3 WHERE id = $2")
        .bind(SeedData::acme().des.0)
        .bind(issue.issue.id.0)
        .bind(lineal_testkit::seed::identity::seed_workflow_state_ids(3)[0].0)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        kind(
            env.app
                .get_document(&ctx_for(SeedUser::Alice), &ddoc.doc.id.to_string())
                .await
        ),
        ErrorKind::NotFound
    );
    // Issue-Sharing macht das Dokument lesbar, aber nicht schreibbar (Freigaben müssen im
    // Workspace erlaubt sein, sonst wirken bestehende Freigaben nicht).
    sqlx::query(
        "UPDATE organization SET limits = limits || '{\"issueSharingEnabled\": true}'::jsonb WHERE id = $1",
    )
    .bind(SeedData::acme().org.0)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO issue_share (organization_id, issue_id, user_id) VALUES ($1, $2, $3)")
        .bind(SeedData::acme().org.0)
        .bind(issue.issue.id.0)
        .bind(SeedData::acme().alice.0)
        .execute(&pool)
        .await
        .unwrap();
    let alice = ctx_for(SeedUser::Alice);
    env.app
        .get_document(&alice, &ddoc.doc.id.to_string())
        .await
        .unwrap();
    let upd = SaveDocumentInput {
        id: Some(ddoc.doc.id.to_string()),
        title: Some("Neu".into()),
        ..Default::default()
    };
    assert_eq!(
        kind(env.app.save_document(&alice, upd).await),
        ErrorKind::Forbidden
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn reparent_keeps_slug_and_moves_sort_order(pool: PgPool) {
    let env = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let d = create(&env, SeedUser::Alice, in_team("ENG", "Spec", "Text")).await;
    let other = create(
        &env,
        SeedUser::Alice,
        SaveDocumentInput {
            title: Some("Vorhanden".into()),
            parent: ParentRefs {
                project: Some("P-ENG-1".into()),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await;
    let moved = env
        .app
        .save_document(
            &alice,
            SaveDocumentInput {
                id: Some(d.doc.slug_id.clone()),
                parent: ParentRefs {
                    project: Some("P-ENG-1".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(moved.doc.slug_id, d.doc.slug_id);
    assert_eq!(moved.url, d.url);
    assert!(moved.doc.team_id.is_none() && moved.doc.project_id.is_some());
    assert!(moved.doc.sort_order > other.doc.sort_order);
    assert!(matches!(moved.parent, Some(ParentInfo::Project { .. })));
    // Mehrere Elternparameter beim Reparent -> invalid_request.
    let r = env
        .app
        .save_document(
            &alice,
            SaveDocumentInput {
                id: Some(d.doc.slug_id.clone()),
                parent: ParentRefs {
                    team: Some("ENG".into()),
                    initiative: Some("I-1".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
    // Reparent in ein unsichtbares Team: not_found.
    let r = env
        .app
        .save_document(
            &alice,
            SaveDocumentInput {
                id: Some(d.doc.slug_id.clone()),
                parent: ParentRefs {
                    team: Some("DES".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::NotFound);
    // Zurück in ein Team-Dokument.
    let back = env
        .app
        .save_document(
            &alice,
            SaveDocumentInput {
                id: Some(d.doc.slug_id.clone()),
                parent: ParentRefs {
                    team: Some("WEB".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(back.doc.project_id.is_none() && back.doc.team_id.is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn patch_is_atomic_and_content_is_exclusive(pool: PgPool) {
    let env = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let d = create(
        &env,
        SeedUser::Alice,
        in_team("ENG", "Notizen", "alpha beta alpha"),
    )
    .await;
    let id = d.doc.id.to_string();
    let patch = |ops: Vec<PatchOp>| SaveDocumentInput {
        id: Some(id.clone()),
        patch: Some(ops),
        ..Default::default()
    };
    // Mehrdeutiger Anker -> conflict, Text unverändert (auch nicht teilweise).
    let r = env
        .app
        .save_document(
            &alice,
            patch(vec![
                PatchOp::Append {
                    text: " gamma".into(),
                },
                PatchOp::Replace {
                    old_string: "alpha".into(),
                    new_string: "ALPHA".into(),
                    replace_all: None,
                },
            ]),
        )
        .await;
    assert_eq!(kind(r), ErrorKind::Conflict);
    assert_eq!(
        env.app.get_document(&alice, &id).await.unwrap().doc.content,
        "alpha beta alpha"
    );
    // Gültiger Patch.
    let ok = env
        .app
        .save_document(
            &alice,
            patch(vec![
                PatchOp::Replace {
                    old_string: "beta".into(),
                    new_string: "BETA".into(),
                    replace_all: None,
                },
                PatchOp::Append { text: "!".into() },
            ]),
        )
        .await
        .unwrap();
    assert_eq!(ok.doc.content, "alpha BETA alpha!");
    // content + patch -> invalid_request; patch beim Anlegen -> invalid_request.
    let both = SaveDocumentInput {
        id: Some(id.clone()),
        content: Some("x".into()),
        patch: Some(vec![PatchOp::Append { text: "y".into() }]),
        ..Default::default()
    };
    assert_eq!(
        kind(env.app.save_document(&alice, both).await),
        ErrorKind::InvalidRequest
    );
    let create_patch = SaveDocumentInput {
        title: Some("T".into()),
        patch: Some(vec![PatchOp::Append { text: "y".into() }]),
        parent: ParentRefs {
            team: Some("ENG".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(
        kind(env.app.save_document(&alice, create_patch).await),
        ErrorKind::InvalidRequest
    );
    // Optimistische Sperre.
    let stale = SaveDocumentInput {
        id: Some(id.clone()),
        content: Some("neu".into()),
        expected_version: Some(1),
        ..Default::default()
    };
    let err = env.app.save_document(&alice, stale).await.err().unwrap();
    assert_eq!(err.kind, ErrorKind::Conflict);
    assert_eq!(err.details["currentVersion"], 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn history_snapshots_and_restore(pool: PgPool) {
    let env = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let bob = ctx_for(SeedUser::Bob);
    let d = create(&env, SeedUser::Alice, in_team("ENG", "Plan", "v1")).await;
    let id = d.doc.id.to_string();
    for (who, text) in [(&bob, "v2"), (&alice, "v3")] {
        env.app
            .save_document(
                who,
                SaveDocumentInput {
                    id: Some(id.clone()),
                    content: Some(text.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
    // Gleicher Inhalt erzeugt keinen neuen Snapshot.
    env.app
        .save_document(
            &alice,
            SaveDocumentInput {
                id: Some(id.clone()),
                content: Some("v3".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let h = env
        .app
        .document_history(&alice, &id, None, 10)
        .await
        .unwrap();
    let texts: Vec<_> = h.items.iter().map(|e| e.markdown.as_str()).collect();
    assert_eq!(texts, ["v3", "v2", "v1"]);
    assert_eq!(h.items[2].kind.as_str(), "created");
    assert_eq!(h.items[0].kind.as_str(), "edited");
    assert_eq!(h.items[1].actor_ids, vec![SeedData::acme().bob]);
    // Paging der Historie.
    let p1 = env
        .app
        .document_history(&alice, &id, None, 2)
        .await
        .unwrap();
    assert_eq!(p1.items.len(), 2);
    let p2 = env
        .app
        .document_history(&alice, &id, p1.next, 2)
        .await
        .unwrap();
    assert_eq!(p2.items.len(), 1);
    assert!(p2.next.is_none());
    // Wiederherstellen von v1: neue aktuelle Version, Snapshot `restored`, Historie bleibt.
    let restored = env
        .app
        .restore_document_version(&alice, &id, &h.items[2].id.to_string())
        .await
        .unwrap();
    assert_eq!(restored.doc.content, "v1");
    assert!(restored.doc.restored_at.is_some());
    let h2 = env
        .app
        .document_history(&alice, &id, None, 10)
        .await
        .unwrap();
    assert_eq!(h2.items.len(), 4);
    assert_eq!(h2.items[0].kind.as_str(), "restored");
    assert_eq!(h2.items[0].restored_from_id, Some(h.items[2].id));
    // Fremde Snapshot-ID -> not_found.
    let r = env
        .app
        .restore_document_version(&alice, &id, &uuid::Uuid::now_v7().to_string())
        .await;
    assert_eq!(kind(r), ErrorKind::NotFound);
}

#[sqlx::test(migrations = "./migrations")]
async fn trash_and_restore(pool: PgPool) {
    let env = setup(pool.clone()).await;
    let alice = ctx_for(SeedUser::Alice);
    let d = create(
        &env,
        SeedUser::Alice,
        SaveDocumentInput {
            title: Some("Wegwerf".into()),
            parent: ParentRefs {
                project: Some("P-ENG-1".into()),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await;
    let id = d.doc.id.to_string();
    env.app.delete_document(&alice, &id).await.unwrap();
    assert_eq!(
        kind(env.app.get_document(&alice, &id).await),
        ErrorKind::NotFound
    );
    let list = env
        .app
        .list_documents(
            &alice,
            ListDocumentsInput {
                limit: 50,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(list.items.is_empty());
    let trash = env
        .app
        .list_documents(
            &alice,
            ListDocumentsInput {
                limit: 50,
                trashed: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(trash.items.len(), 1);
    // Elternobjekt im Papierkorb: Wiederherstellen -> invalid_request.
    sqlx::query("UPDATE project SET trashed = true, trashed_at = now() WHERE id = $1")
        .bind(seed_project_id().0)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        kind(env.app.restore_document(&alice, &id).await),
        ErrorKind::InvalidRequest
    );
    sqlx::query("UPDATE project SET trashed = false, trashed_at = NULL WHERE id = $1")
        .bind(seed_project_id().0)
        .execute(&pool)
        .await
        .unwrap();
    let back = env.app.restore_document(&alice, &id).await.unwrap();
    assert!(!back.doc.trashed);
    env.app.get_document(&alice, &id).await.unwrap();
    // Kaskade: Elternobjekt in den Papierkorb -> Dokumente mit; Wiederherstellen holt nur diese zurück.
    let mut tx = env.app.store.begin(SeedData::acme().org).await.unwrap();
    let ids = env
        .app
        .trash_documents_of(
            &mut *tx,
            lineal_app::ports::repos::document::DocParent::Project(seed_project_id()),
        )
        .await
        .unwrap();
    assert_eq!(ids, vec![d.doc.id]);
    let restored = env
        .app
        .restore_documents_of(
            &mut *tx,
            lineal_app::ports::repos::document::DocParent::Project(seed_project_id()),
        )
        .await
        .unwrap();
    assert_eq!(restored, vec![d.doc.id]);
    tx.commit().await.unwrap();
}

#[sqlx::test(migrations = "./migrations")]
async fn markdown_is_sanitized_everywhere(pool: PgPool) {
    let env = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let evil = "Hallo <script>alert(1)</script><img src=x onerror=alert(2)> [klick](javascript:alert(3)) ![b](data:text/html;base64,AAAA) **fett**";
    let d = create(&env, SeedUser::Alice, in_team("ENG", "XSS", evil)).await;
    for bad in ["<script", "onerror", "javascript:", "data:text/html"] {
        assert!(!d.doc.content.contains(bad), "{bad} in {}", d.doc.content);
    }
    assert!(d.doc.content.contains("**fett**"));
    // Patch und Update werden ebenfalls bereinigt, auch die Snapshots.
    let upd = env
        .app
        .save_document(
            &alice,
            SaveDocumentInput {
                id: Some(d.doc.id.to_string()),
                patch: Some(vec![PatchOp::Append {
                    text: "\n<iframe src=\"https://evil.example\"></iframe>[x](vbscript:y)".into(),
                }]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!upd.doc.content.contains("<iframe") && !upd.doc.content.contains("vbscript:"));
    let h = env
        .app
        .document_history(&alice, &d.doc.id.to_string(), None, 10)
        .await
        .unwrap();
    for e in &h.items {
        assert!(!e.markdown.contains("<script") && !e.markdown.contains("<iframe"));
    }
    // Icon und Farbe werden validiert.
    for (icon, color) in [("a b", ""), ("", "red"), ("<x>", "")] {
        let r = env
            .app
            .save_document(
                &alice,
                SaveDocumentInput {
                    id: Some(d.doc.id.to_string()),
                    icon: Some(icon.into()),
                    color: Some(color.into()),
                    ..Default::default()
                },
            )
            .await;
        assert_eq!(kind(r), ErrorKind::InvalidRequest, "{icon} {color}");
    }
    let ok = env
        .app
        .save_document(
            &alice,
            SaveDocumentInput {
                id: Some(d.doc.id.to_string()),
                icon: Some(":memo:".into()),
                color: Some("#5E6AD2".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(ok.doc.icon.as_deref(), Some(":memo:"));
    // Titel über 255 Zeichen.
    let r = env
        .app
        .save_document(&alice, in_team("ENG", &"x".repeat(256), ""))
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
}

#[sqlx::test(migrations = "./migrations")]
async fn permissions_scopes_and_archived_parents(pool: PgPool) {
    let env = setup(pool.clone()).await;
    // Nur-Lese-Scope darf nicht schreiben.
    let ro = RequestCtx {
        scopes: Scopes::read_only(),
        ..ctx_for(SeedUser::Alice)
    };
    assert_eq!(
        kind(env.app.save_document(&ro, in_team("ENG", "X", "")).await),
        ErrorKind::Forbidden
    );
    // Gast Carol: ENG unsichtbar (not_found), WEB (Mitglied) erlaubt.
    let carol = ctx_for(SeedUser::Carol);
    assert_eq!(
        kind(env.app.save_document(&carol, in_team("ENG", "X", "")).await),
        ErrorKind::NotFound
    );
    env.app
        .save_document(&carol, in_team("WEB", "Gast-Notiz", ""))
        .await
        .unwrap();
    // Gast darf keine Initiativen sehen.
    let r = env
        .app
        .save_document(
            &carol,
            SaveDocumentInput {
                title: Some("X".into()),
                parent: ParentRefs {
                    initiative: Some("I-1".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::NotFound);
    // Archiviertes Projekt: anlegen und bearbeiten -> invalid_request; Liste nur mit includeArchived.
    let alice = ctx_for(SeedUser::Alice);
    let d = create(
        &env,
        SeedUser::Alice,
        SaveDocumentInput {
            title: Some("Im Projekt".into()),
            parent: ParentRefs {
                project: Some("P-ENG-1".into()),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .await;
    sqlx::query("UPDATE project SET archived_at = now() WHERE id = $1")
        .bind(seed_project_id().0)
        .execute(&pool)
        .await
        .unwrap();
    let r = env
        .app
        .save_document(
            &alice,
            SaveDocumentInput {
                id: Some(d.doc.id.to_string()),
                title: Some("Neu".into()),
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(r), ErrorKind::InvalidRequest);
    let list = |archived: bool| {
        let alice = alice.clone();
        let env = &env;
        async move {
            env.app
                .list_documents(
                    &alice,
                    ListDocumentsInput {
                        limit: 50,
                        include_archived: archived,
                        project: Some("P-ENG-1".into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .items
                .len()
        }
    };
    assert_eq!(list(false).await, 0);
    assert_eq!(list(true).await, 1);
}

/// B2.04: parallele Patches auf verschiedene Absätze gehen nicht verloren, und bei gleicher
/// `expectedVersion` gewinnt genau ein Aufruf.
#[sqlx::test(migrations = "./migrations")]
async fn parallel_patches_and_expected_version_are_atomic(pool: PgPool) {
    let store = PgStore::new(pool.clone());
    let mut tx = store.begin(SeedData::acme().org).await.unwrap();
    seed::run(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    let app = test_app(Arc::new(store));
    let alice = ctx_for(SeedUser::Alice);
    for round in 0..4 {
        let content = (0..6)
            .map(|i| format!("Absatz p{i}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let d = app
            .save_document(&alice, in_team("ENG", &format!("Parallel {round}"), &content))
            .await
            .unwrap();
        let id = d.doc.id.to_string();
        let mut tasks = Vec::new();
        for i in 0..6 {
            let app = app.clone();
            let ctx = alice.clone();
            let id = id.clone();
            tasks.push(tokio::spawn(async move {
                app.save_document(
                    &ctx,
                    SaveDocumentInput {
                        id: Some(id),
                        patch: Some(vec![PatchOp::Replace {
                            old_string: format!("p{i}"),
                            new_string: format!("neu{i}"),
                            replace_all: None,
                        }]),
                        ..Default::default()
                    },
                )
                .await
            }));
        }
        for t in tasks {
            t.await.unwrap().unwrap();
        }
        let got = app.get_document(&alice, &id).await.unwrap();
        for i in 0..6 {
            assert!(
                got.doc.content.contains(&format!("neu{i}")),
                "Patch {i} verloren in Runde {round}: {}",
                got.doc.content
            );
        }

        // expectedVersion: genau ein Gewinner.
        let version = got.doc.content_version;
        let mut tasks = Vec::new();
        for i in 0..4 {
            let app = app.clone();
            let ctx = alice.clone();
            let id = id.clone();
            tasks.push(tokio::spawn(async move {
                app.save_document(
                    &ctx,
                    SaveDocumentInput {
                        id: Some(id),
                        content: Some(format!("Gewinner {i}")),
                        expected_version: Some(version),
                        ..Default::default()
                    },
                )
                .await
            }));
        }
        let mut wins = 0;
        for t in tasks {
            match t.await.unwrap() {
                Ok(_) => wins += 1,
                Err(e) => assert_eq!(e.kind, ErrorKind::Conflict),
            }
        }
        assert_eq!(wins, 1, "Runde {round}: genau ein Gewinner erwartet");
    }
}
