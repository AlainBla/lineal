#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Paket 1 (Nebenläufigkeit): parallele Aufrufe gegen echte Datenbank, je mehrere Runden.
//! Erwartung: nie `internal`, Invarianten per SQL geprüft.

use lineal_app::ports::Store;
use lineal_app::ports::repos::label::LabelKind;
use lineal_app::ports::repos::view::ViewModel;
use lineal_app::usecases::attachment::LinkAttachmentInput;
use lineal_app::usecases::comment::{CreateCommentInput, ParentSpec};
use lineal_app::usecases::initiative::SaveInitiativeInput;
use lineal_app::usecases::issue::{CreateIssueInput, RelationChanges, UpdateIssueInput};
use lineal_app::usecases::label::SaveLabelInput;
use lineal_app::usecases::project::SaveProjectInput;
use lineal_app::usecases::state::CreateStateInput;
use lineal_app::usecases::team::{CreateTeamInput, NestOptions};
use lineal_app::usecases::template::SaveTemplateInput;
use lineal_app::usecases::view::{CreateViewInput, UpdateViewInput};
use lineal_app::{App, AppError, ErrorKind, RequestCtx};
use lineal_db::PgStore;
use lineal_domain::enums::{StateType, TemplateType};
use lineal_domain::ids::StateId;
use lineal_testkit::{SeedData, SeedUser, ctx_for, seed, test_app};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

const ROUNDS: usize = 20;

struct Fx {
    app: Arc<App>,
    data: SeedData,
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
        data,
        pool,
    }
}

fn alice() -> RequestCtx {
    ctx_for(SeedUser::Alice)
}

/// Kein Fehler darf `internal` oder ein (nach Wiederholungen verbliebener) Deadlock sein.
fn assert_no_internal<T: std::fmt::Debug>(what: &str, results: &[Result<T, AppError>]) {
    for r in results {
        if let Err(e) = r {
            assert_ne!(e.kind, ErrorKind::Internal, "{what}: internal error: {e:?}");
            assert!(!e.is_retryable(), "{what}: deadlock/serialization: {e:?}");
        }
    }
}

impl Fx {
    async fn issue(&self, title: &str) -> Uuid {
        self.app
            .create_issue(
                &alice(),
                CreateIssueInput {
                    team: "ENG".into(),
                    title: Some(title.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .issue
            .id
            .0
    }

    async fn set_parent(&self, id: Uuid, parent: Option<Uuid>) -> Result<(), AppError> {
        self.app
            .update_issue(
                &alice(),
                UpdateIssueInput {
                    id: id.to_string(),
                    parent: Some(parent.map(|p| p.to_string())),
                    ..Default::default()
                },
            )
            .await
            .map(|_| ())
    }
}

/// B1.01: parallele Kommentare auf dasselbe Issue; auch bei gesetztem Snooze.
#[sqlx::test(migrations = "./migrations")]
async fn b1_01_parallel_comments_on_one_issue(pool: PgPool) {
    let fx = setup(pool).await;
    for round in 0..ROUNDS {
        let issue = fx.issue(&format!("Comment race {round}")).await;
        if round % 2 == 1 {
            // Triage-Snooze durch einen anderen User: jeder Kommentar will ihn beenden.
            sqlx::query(
                "UPDATE issue SET snoozed_until_at = now() + interval '1 day', snoozed_by_id = $2 WHERE id = $1",
            )
            .bind(issue)
            .bind(fx.data.carol.0)
            .execute(&fx.pool)
            .await
            .unwrap();
        }
        let mut set = tokio::task::JoinSet::new();
        for n in 0..6 {
            let app = fx.app.clone();
            set.spawn(async move {
                app.create_comment(
                    &alice(),
                    CreateCommentInput {
                        parent: Some(ParentSpec::Issue(issue.to_string())),
                        body: format!("c{n}"),
                        ..Default::default()
                    },
                )
                .await
            });
        }
        let mut results = Vec::new();
        while let Some(r) = set.join_next().await {
            results.push(r.unwrap());
        }
        assert!(
            results.iter().all(Result::is_ok),
            "round {round}: {results:?}"
        );
        assert_no_internal("comment", &results);
        assert!(results.iter().all(Result::is_ok), "{results:?}");
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM comment WHERE issue_id = $1")
            .bind(issue)
            .fetch_one(&fx.pool)
            .await
            .unwrap();
        assert_eq!(n, 6);
    }
}

/// B1.02 + B1.17: A unter B und B unter A gleichzeitig: genau einer gewinnt, kein Zyklus.
#[sqlx::test(migrations = "./migrations")]
async fn b1_02_mutual_parent_swap_has_one_winner(pool: PgPool) {
    let fx = setup(pool).await;
    for round in 0..ROUNDS {
        let a = fx.issue(&format!("A{round}")).await;
        let b = fx.issue(&format!("B{round}")).await;
        let (r1, r2) = tokio::join!(fx.set_parent(a, Some(b)), fx.set_parent(b, Some(a)));
        assert_no_internal("parent swap", &[r1.clone(), r2.clone()]);
        // Zusätzlich mehrere Paare gleichzeitig (mehr Verzahnung).
        let mut set = tokio::task::JoinSet::new();
        for _ in 0..3 {
            let (x, y) = (fx.issue("X").await, fx.issue("Y").await);
            for (c, p) in [(x, y), (y, x)] {
                let fx2 = (fx.app.clone(), c, p);
                set.spawn(async move {
                    fx2.0
                        .update_issue(
                            &alice(),
                            UpdateIssueInput {
                                id: fx2.1.to_string(),
                                parent: Some(Some(fx2.2.to_string())),
                                ..Default::default()
                            },
                        )
                        .await
                        .map(|_| ())
                });
            }
        }
        let mut extra = Vec::new();
        while let Some(r) = set.join_next().await {
            extra.push(r.unwrap());
        }
        assert_no_internal("parent swap pairs", &extra);
        assert_eq!(extra.iter().filter(|r| r.is_ok()).count(), 3, "{extra:?}");
        assert_eq!(
            [r1.is_ok(), r2.is_ok()].iter().filter(|x| **x).count(),
            1,
            "{r1:?} {r2:?}"
        );
        let cyc: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM issue x JOIN issue y ON x.parent_id = y.id AND y.parent_id = x.id \
             WHERE x.id IN ($1, $2)",
        )
        .bind(a)
        .bind(b)
        .fetch_one(&fx.pool)
        .await
        .unwrap();
        assert_eq!(cyc, 0, "Zyklus entstanden");
    }
}

/// B1.17: Verschachtelungstiefe (max 10) ist durch paralleles Umhängen nicht umgehbar.
#[sqlx::test(migrations = "./migrations")]
async fn b1_17_parallel_reparenting_respects_max_depth(pool: PgPool) {
    let fx = setup(pool).await;
    for round in 0..ROUNDS / 2 {
        // Kette D (Länge 6), Kette C (Länge 4) und ein einzelnes Issue U.
        let mut chain_d = vec![fx.issue(&format!("d0.{round}")).await];
        let mut chain_c = vec![fx.issue(&format!("c0.{round}")).await];
        for i in 1..6 {
            let d = fx.issue(&format!("d{i}")).await;
            fx.set_parent(d, Some(chain_d[i - 1])).await.unwrap();
            chain_d.push(d);
        }
        for i in 1..4 {
            let c = fx.issue(&format!("c{i}")).await;
            fx.set_parent(c, Some(chain_c[i - 1])).await.unwrap();
            chain_c.push(c);
        }
        let u = fx.issue(&format!("u{round}")).await;
        // Einzeln jeweils erlaubt (Tiefe 5 bzw. 7), zusammen 6 + 1 + 4 = 11 > 10.
        let (r1, r2) = tokio::join!(
            fx.set_parent(chain_c[0], Some(u)),
            fx.set_parent(u, Some(*chain_d.last().unwrap()))
        );
        assert_no_internal("depth", &[r1.clone(), r2.clone()]);
        assert!(!(r1.is_ok() && r2.is_ok()), "beide gelungen: {r1:?} {r2:?}");
        let max_depth: i32 = sqlx::query_scalar(
            "WITH RECURSIVE t AS (SELECT id, 1 AS d FROM issue WHERE parent_id IS NULL \
             UNION ALL SELECT i.id, t.d + 1 FROM issue i JOIN t ON i.parent_id = t.id) \
             SELECT max(d) FROM t",
        )
        .fetch_one(&fx.pool)
        .await
        .unwrap();
        assert!(max_depth <= 10, "Tiefe {max_depth}");
    }
}

/// B1.03: gegenläufige Relationen und Relation + Parent-Setzen, Kommentar + Relation.
#[sqlx::test(migrations = "./migrations")]
async fn b1_03_opposite_relations_do_not_deadlock(pool: PgPool) {
    let fx = setup(pool).await;
    let blocks = |other: Uuid| RelationChanges {
        blocks: vec![other.to_string()],
        ..Default::default()
    };
    for round in 0..ROUNDS {
        let i = fx.issue(&format!("I{round}")).await;
        let j = fx.issue(&format!("J{round}")).await;
        let upd = |id: Uuid, rel: RelationChanges| {
            let app = fx.app.clone();
            async move {
                app.update_issue(
                    &alice(),
                    UpdateIssueInput {
                        id: id.to_string(),
                        relations: rel,
                        ..Default::default()
                    },
                )
                .await
                .map(|_| ())
            }
        };
        let (r1, r2) = tokio::join!(upd(i, blocks(j)), upd(j, blocks(i)));
        assert_no_internal("relations", &[r1.clone(), r2.clone()]);
        let both: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM issue_relation a JOIN issue_relation b \
             ON a.issue_id = b.related_issue_id AND a.related_issue_id = b.issue_id \
             WHERE a.type = 'blocks' AND b.type = 'blocks' AND a.issue_id IN ($1,$2)",
        )
        .bind(i)
        .bind(j)
        .fetch_one(&fx.pool)
        .await
        .unwrap();
        assert_eq!(both, 0, "beide Richtungen blocks: {r1:?} {r2:?}");
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn b1_03_relation_parent_comment_mix_does_not_deadlock(pool: PgPool) {
    let fx = setup(pool).await;
    for round in 0..ROUNDS {
        let i = fx.issue(&format!("I{round}")).await;
        let j = fx.issue(&format!("J{round}")).await;
        let a1 = fx.app.clone();
        let a2 = fx.app.clone();
        let a3 = fx.app.clone();
        let rel = async move {
            a1.update_issue(
                &alice(),
                UpdateIssueInput {
                    id: i.to_string(),
                    relations: RelationChanges {
                        related_to: vec![j.to_string()],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .await
            .map(|_| ())
        };
        let par = async move {
            a2.update_issue(
                &alice(),
                UpdateIssueInput {
                    id: j.to_string(),
                    parent: Some(Some(i.to_string())),
                    ..Default::default()
                },
            )
            .await
            .map(|_| ())
        };
        let com = async move {
            a3.create_comment(
                &alice(),
                CreateCommentInput {
                    parent: Some(ParentSpec::Issue(j.to_string())),
                    body: "x".into(),
                    ..Default::default()
                },
            )
            .await
            .map(|_| ())
        };
        let (r1, r2, r3) = tokio::join!(rel, par, com);
        assert_no_internal("mix", &[r1, r2, r3]);
    }
}

/// B1.04: Auto-Close (Kinder schließen den Parent, Parent schließt Kinder) ohne Deadlock.
#[sqlx::test(migrations = "./migrations")]
async fn b1_04_auto_close_parent_and_children_in_parallel(pool: PgPool) {
    let fx = setup(pool).await;
    sqlx::query("UPDATE team SET auto_close_parent_issues = true, auto_close_child_issues = true")
        .execute(&fx.pool)
        .await
        .unwrap();
    let done = |app: Arc<App>, id: Uuid| async move {
        app.update_issue(
            &alice(),
            UpdateIssueInput {
                id: id.to_string(),
                state: Some("Done".into()),
                ..Default::default()
            },
        )
        .await
        .map(|_| ())
    };
    for round in 0..ROUNDS {
        let parent = fx.issue(&format!("P{round}")).await;
        let mut kids = Vec::new();
        for k in 0..3 {
            let c = fx.issue(&format!("K{round}.{k}")).await;
            fx.set_parent(c, Some(parent)).await.unwrap();
            kids.push(c);
        }
        let mut set = tokio::task::JoinSet::new();
        set.spawn(done(fx.app.clone(), parent));
        for c in &kids {
            set.spawn(done(fx.app.clone(), *c));
        }
        let mut results = Vec::new();
        while let Some(r) = set.join_next().await {
            results.push(r.unwrap());
        }
        assert_no_internal("auto-close", &results);
        assert!(
            results.iter().all(Result::is_ok),
            "round {round}: {results:?}"
        );
        let open: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM issue WHERE (id = $1 OR parent_id = $1) AND state_type <> 'completed'",
        )
        .bind(parent)
        .fetch_one(&fx.pool)
        .await
        .unwrap();
        assert_eq!(open, 0, "round {round}: nicht alle abgeschlossen");
    }
}

/// B1.05: Initiative-Hierarchie (parentInitiatives) parallel zu Initiative-Updates und Inserts.
#[sqlx::test(migrations = "./migrations")]
async fn b1_05_initiative_hierarchy_does_not_block_inserts(pool: PgPool) {
    let fx = setup(pool).await;
    let new_init = |name: String| {
        let app = fx.app.clone();
        async move {
            app.save_initiative(
                &alice(),
                SaveInitiativeInput {
                    name: Some(name),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .identifier
        }
    };
    for round in 0..ROUNDS {
        let a = new_init(format!("Init {round}")).await;
        let b = new_init(format!("Parent {round}")).await;
        let mut set = tokio::task::JoinSet::new();
        for n in 0..3 {
            // Bestehende Initiative ändern und Eltern setzen (Update, dann Hierarchie-Sperre).
            let (app, a_, b_) = (fx.app.clone(), a.clone(), b.clone());
            set.spawn(async move {
                app.save_initiative(
                    &alice(),
                    SaveInitiativeInput {
                        id: Some(a_),
                        name: Some(format!("Renamed {round}.{n}")),
                        parent_initiatives: vec![b_],
                        ..Default::default()
                    },
                )
                .await
                .map(|_| ())
            });
            let (app, a_) = (fx.app.clone(), a.clone());
            set.spawn(async move {
                app.post_initiative_update(&alice(), &a_, &format!("update {n}"), None, false)
                    .await
                    .map(|_| ())
            });
            let (app, a_) = (fx.app.clone(), a.clone());
            set.spawn(async move {
                app.create_comment(
                    &alice(),
                    CreateCommentInput {
                        parent: Some(ParentSpec::Initiative(a_)),
                        body: "hi".into(),
                        ..Default::default()
                    },
                )
                .await
                .map(|_| ())
            });
            let (app, b_) = (fx.app.clone(), b.clone());
            set.spawn(async move {
                app.save_initiative(
                    &alice(),
                    SaveInitiativeInput {
                        name: Some(format!("Child {round}.{n}")),
                        parent_initiatives: vec![b_],
                        ..Default::default()
                    },
                )
                .await
                .map(|_| ())
            });
        }
        let mut results = Vec::new();
        while let Some(r) = set.join_next().await {
            results.push(r.unwrap());
        }
        assert_no_internal("initiative", &results);
    }
}

/// B1.06: parallele View-Updates mit disjunkten Feldern verlieren keine Änderung;
/// `expectedVersion` ist atomar (genau ein Gewinner).
#[sqlx::test(migrations = "./migrations")]
async fn b1_06_view_updates_are_not_lost(pool: PgPool) {
    let fx = setup(pool).await;
    let upd = |app: Arc<App>,
               reference: String,
               f: Box<dyn FnOnce(&mut UpdateViewInput) + Send>| async move {
        let mut input = UpdateViewInput {
            reference,
            ..Default::default()
        };
        f(&mut input);
        app.update_view(&alice(), input).await
    };
    for round in 0..ROUNDS {
        let created = fx
            .app
            .create_view(
                &alice(),
                CreateViewInput {
                    name: format!("V{round}"),
                    description: None,
                    icon: None,
                    color: None,
                    model: ViewModel::Issue,
                    filter: serde_json::json!({}),
                    shared: false,
                    team: None,
                    project: None,
                    initiative: None,
                    preferences: None,
                },
            )
            .await
            .unwrap();
        let id = created.view.id.to_string();
        let mut set = tokio::task::JoinSet::new();
        set.spawn(upd(
            fx.app.clone(),
            id.clone(),
            Box::new(|i| i.name = Some("Renamed".into())),
        ));
        set.spawn(upd(
            fx.app.clone(),
            id.clone(),
            Box::new(|i| i.description = Some(Some("Desc".into()))),
        ));
        set.spawn(upd(
            fx.app.clone(),
            id.clone(),
            Box::new(|i| i.color = Some(Some("#112233".into()))),
        ));
        set.spawn(upd(
            fx.app.clone(),
            id.clone(),
            Box::new(|i| i.sort_order = Some(42.0)),
        ));
        let mut results = Vec::new();
        while let Some(r) = set.join_next().await {
            results.push(r.unwrap());
        }
        assert_no_internal("view update", &results);
        assert!(results.iter().all(Result::is_ok), "{results:?}");
        let row: (String, Option<String>, Option<String>, f64, i32) = sqlx::query_as(
            "SELECT name, description, color, sort_order::float8, version FROM custom_view WHERE id = $1::uuid",
        )
        .bind(&id)
        .fetch_one(&fx.pool)
        .await
        .unwrap();
        assert_eq!(
            row,
            (
                "Renamed".into(),
                Some("Desc".into()),
                Some("#112233".into()),
                42.0,
                created.view.version + 4
            ),
            "round {round}: Update verloren"
        );

        // Gleiche expectedVersion: genau ein Gewinner.
        let v = row.4;
        let mut set = tokio::task::JoinSet::new();
        for n in 0..3 {
            set.spawn(upd(
                fx.app.clone(),
                id.clone(),
                Box::new(move |i| {
                    i.expected_version = Some(v);
                    i.name = Some(format!("W{n}"));
                }),
            ));
        }
        let mut results = Vec::new();
        while let Some(r) = set.join_next().await {
            results.push(r.unwrap());
        }
        assert_no_internal("view expectedVersion", &results);
        assert_eq!(
            results.iter().filter(|r| r.is_ok()).count(),
            1,
            "{results:?}"
        );
        for r in results.iter().filter_map(|r| r.as_ref().err()) {
            assert_eq!(r.kind, ErrorKind::Conflict);
        }
    }
}

/// B1.06: parallele Template-Updates mit disjunkten Feldern.
#[sqlx::test(migrations = "./migrations")]
async fn b1_06_template_updates_are_not_lost(pool: PgPool) {
    let fx = setup(pool).await;
    for round in 0..ROUNDS {
        let t = fx
            .app
            .save_template(
                &alice(),
                SaveTemplateInput {
                    template_type: Some(TemplateType::Document),
                    name: Some(format!("T{round}")),
                    template_data: Some(
                        serde_json::json!({ "title": "PRD", "content": "## Problem" }),
                    ),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let id = t.template.id.to_string();
        let mk = |f: Box<dyn FnOnce(&mut SaveTemplateInput) + Send>| {
            let app = fx.app.clone();
            let mut input = SaveTemplateInput {
                id: Some(id.clone()),
                ..Default::default()
            };
            f(&mut input);
            async move { app.save_template(&alice(), input).await }
        };
        let mut set = tokio::task::JoinSet::new();
        set.spawn(mk(Box::new(|i| i.name = Some(format!("Renamed{round}")))));
        set.spawn(mk(Box::new(|i| i.description = Some("Desc".into()))));
        set.spawn(mk(Box::new(|i| i.color = Some("#112233".into()))));
        set.spawn(mk(Box::new(|i| i.icon = Some("star".into()))));
        let mut results = Vec::new();
        while let Some(r) = set.join_next().await {
            results.push(r.unwrap());
        }
        assert_no_internal("template update", &results);
        assert!(results.iter().all(Result::is_ok), "{results:?}");
        let row: (String, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT name, description, color, icon FROM template WHERE id = $1::uuid",
        )
        .bind(&id)
        .fetch_one(&fx.pool)
        .await
        .unwrap();
        assert_eq!(
            row,
            (
                format!("Renamed{round}"),
                Some("Desc".into()),
                Some("#112233".into()),
                Some("star".into())
            ),
            "round {round}: Update verloren"
        );
    }
}

impl Fx {
    async fn label(&self, name: &str) -> String {
        self.app
            .save_label(
                &alice(),
                LabelKind::Issue,
                SaveLabelInput {
                    name: Some(name.into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .label
            .id
            .to_string()
    }

    fn add_label(
        &self,
        issue: Uuid,
        label: &str,
    ) -> impl std::future::Future<Output = Result<(), AppError>> + Send + 'static {
        let app = self.app.clone();
        let label = label.to_owned();
        async move {
            app.update_issue(
                &alice(),
                UpdateIssueInput {
                    id: issue.to_string(),
                    add_labels: vec![label],
                    ..Default::default()
                },
            )
            .await
            .map(|_| ())
        }
    }
}

/// B1.07: Label-Merge parallel zur Zuordnung: nichts geht still verloren.
#[sqlx::test(migrations = "./migrations")]
async fn b1_07_label_merge_does_not_lose_assignments(pool: PgPool) {
    let fx = setup(pool).await;
    for round in 0..ROUNDS {
        let src = fx.label(&format!("src{round}")).await;
        let dst = fx.label(&format!("dst{round}")).await;
        let mut issues = Vec::new();
        for n in 0..6 {
            issues.push(fx.issue(&format!("L{round}.{n}")).await);
        }
        let merge = {
            let app = fx.app.clone();
            let (src, dst) = (src.clone(), dst.clone());
            async move {
                app.merge_issue_label(&alice(), &src, &dst)
                    .await
                    .map(|_| ())
            }
        };
        let adds = issues.iter().map(|i| fx.add_label(*i, &src));
        let (m, rs) = tokio::join!(merge, futures_join_all(adds));
        assert_no_internal("label merge", &[m]);
        assert_no_internal("label add", &rs);
        for (i, r) in issues.iter().zip(&rs) {
            let has_dst: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM issue_to_label WHERE issue_id = $1 AND label_id = $2::uuid)",
            )
            .bind(i)
            .bind(&dst)
            .fetch_one(&fx.pool)
            .await
            .unwrap();
            if r.is_ok() {
                assert!(has_dst, "round {round}: erfolgreiche Zuordnung verloren");
            } else {
                assert_eq!(r.as_ref().unwrap_err().kind, ErrorKind::NotFound, "{r:?}");
            }
        }
    }
}

/// B1.07: Label löschen parallel zur Zuordnung: kein internal.
#[sqlx::test(migrations = "./migrations")]
async fn b1_07_label_delete_parallel_to_assignment(pool: PgPool) {
    let fx = setup(pool).await;
    for round in 0..ROUNDS {
        let label = fx.label(&format!("del{round}")).await;
        let mut issues = Vec::new();
        for n in 0..6 {
            issues.push(fx.issue(&format!("D{round}.{n}")).await);
        }
        let del = {
            let app = fx.app.clone();
            let label = label.clone();
            async move { app.delete_issue_label(&alice(), &label).await.map(|_| ()) }
        };
        let adds = issues.iter().map(|i| fx.add_label(*i, &label));
        let (d, rs) = tokio::join!(del, futures_join_all(adds));
        assert_no_internal("label delete", &[d]);
        assert_no_internal("label add", &rs);
    }
}

/// Führt Futures gleichzeitig aus (ohne futures-Crate).
async fn futures_join_all<F, T>(it: impl IntoIterator<Item = F>) -> Vec<T>
where
    F: std::future::Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let mut set = tokio::task::JoinSet::new();
    for (n, f) in it.into_iter().enumerate() {
        set.spawn(async move { (n, f.await) });
    }
    let mut out = Vec::new();
    while let Some(r) = set.join_next().await {
        out.push(r.unwrap());
    }
    out.sort_by_key(|(n, _)| *n);
    out.into_iter().map(|(_, t)| t).collect()
}

/// B1.19: Nutzungsanzahl eines Labels ignoriert Issues im Papierkorb.
#[sqlx::test(migrations = "./migrations")]
async fn b1_19_label_usage_ignores_trashed_issues(pool: PgPool) {
    let fx = setup(pool).await;
    let label = fx.label("usage").await;
    let a = fx.issue("A").await;
    let b = fx.issue("B").await;
    fx.add_label(a, &label).await.unwrap();
    fx.add_label(b, &label).await.unwrap();
    assert_eq!(fx.app.issue_label_usage(&alice(), &label).await.unwrap(), 2);
    fx.app
        .delete_issue(&alice(), &b.to_string(), false)
        .await
        .unwrap();
    assert_eq!(fx.app.issue_label_usage(&alice(), &label).await.unwrap(), 1);
}

impl Fx {
    async fn team(&self, name: &str, key: &str, parent: Option<&str>) -> Uuid {
        self.app
            .create_team(
                &alice(),
                CreateTeamInput {
                    name: name.into(),
                    key: Some(key.into()),
                    parent: parent.map(str::to_owned),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .team
            .id
            .0
    }

    async fn nest(&self, team: Uuid, parent: Uuid) -> Result<(), AppError> {
        self.app
            .nest_team(
                &alice(),
                &team.to_string(),
                Some(&parent.to_string()),
                NestOptions::default(),
            )
            .await
            .map(|_| ())
    }
}

/// B1.08: parallele nestTeam: kein Zyklus, maxTeamDepth (5) bleibt eingehalten.
#[sqlx::test(migrations = "./migrations")]
async fn b1_08_parallel_nest_team_keeps_tree_valid(pool: PgPool) {
    let fx = setup(pool).await;
    for round in 0..ROUNDS / 2 {
        let a = fx
            .team(&format!("A{round}"), &format!("A{round:02}"), None)
            .await;
        let b = fx
            .team(&format!("B{round}"), &format!("B{round:02}"), None)
            .await;
        let (r1, r2) = tokio::join!(fx.nest(a, b), fx.nest(b, a));
        assert_no_internal("nest cycle", &[r1.clone(), r2.clone()]);
        assert_eq!(
            [r1.is_ok(), r2.is_ok()].iter().filter(|x| **x).count(),
            1,
            "round {round}: {r1:?} {r2:?}"
        );
        // Tiefe: x1 > x2 > x3 (Tiefe 3), y einzeln, z1 > z2 (Höhe 2): einzeln je erlaubt,
        // zusammen 3 + 1 + 2 = 6 > 5.
        let k = |p: &str, n: usize| format!("{p}{round:02}{n}");
        let mut x = vec![fx.team(&k("x", 1), &k("X", 1), None).await];
        for n in 2..=3 {
            let t = fx
                .team(&k("x", n), &k("X", n), Some(&x[n - 2].to_string()))
                .await;
            x.push(t);
        }
        let y = fx.team(&k("y", 1), &k("Y", 1), None).await;
        let z1 = fx.team(&k("z", 1), &k("Z", 1), None).await;
        let _z2 = fx.team(&k("z", 2), &k("Z", 2), Some(&z1.to_string())).await;
        let (r1, r2) = tokio::join!(fx.nest(z1, y), fx.nest(y, x[2]));
        assert_no_internal("nest depth", &[r1.clone(), r2.clone()]);
        assert!(!(r1.is_ok() && r2.is_ok()), "Tiefe umgangen: {r1:?} {r2:?}");
        let max_depth: i32 = sqlx::query_scalar(
            "WITH RECURSIVE t AS (SELECT id, 1 AS d FROM team WHERE parent_id IS NULL \
             UNION ALL SELECT c.id, t.d + 1 FROM team c JOIN t ON c.parent_id = t.id) \
             SELECT max(d) FROM t",
        )
        .fetch_one(&fx.pool)
        .await
        .unwrap();
        assert!(max_depth <= 5, "round {round}: Tiefe {max_depth}");
    }
}

/// B1.09: Mitglied eines Sub-Teams ist immer auch Mitglied des Parents.
#[sqlx::test(migrations = "./migrations")]
async fn b1_09_membership_invariant_under_parallel_changes(pool: PgPool) {
    let fx = setup(pool).await;
    let bob = fx.data.bob.to_string();
    for round in 0..ROUNDS {
        let p = fx
            .team(&format!("P{round}"), &format!("P{round:02}"), None)
            .await;
        let c = fx
            .team(
                &format!("C{round}"),
                &format!("C{round:02}"),
                Some(&p.to_string()),
            )
            .await;
        fx.app
            .add_team_member(&alice(), &p.to_string(), &bob, false)
            .await
            .unwrap();
        let (r1, r2) = {
            let (a1, a2) = (fx.app.clone(), fx.app.clone());
            let (ps, cs, b1, b2) = (p.to_string(), c.to_string(), bob.clone(), bob.clone());
            tokio::join!(
                async move { a1.remove_team_member(&alice(), &ps, &b1).await.map(|_| ()) },
                async move {
                    a2.add_team_member(&alice(), &cs, &b2, false)
                        .await
                        .map(|_| ())
                }
            )
        };
        assert_no_internal("membership", &[r1, r2]);
        let orphan: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM team_membership c JOIN team t ON t.id = c.team_id \
             JOIN \"user\" u ON u.id = c.user_id \
             WHERE u.role <> 'guest' AND t.parent_id IS NOT NULL AND NOT EXISTS ( \
               SELECT 1 FROM team_membership p WHERE p.team_id = t.parent_id AND p.user_id = c.user_id)",
        )
        .fetch_one(&fx.pool)
        .await
        .unwrap();
        assert_eq!(
            orphan, 0,
            "round {round}: Sub-Team-Mitglied ohne Parent-Mitgliedschaft"
        );
    }
}

/// B1.10: Status archivieren parallel zur Issue-Anlage: kein Issue im archivierten Status.
#[sqlx::test(migrations = "./migrations")]
async fn b1_10_state_archive_vs_issue_create(pool: PgPool) {
    let fx = setup(pool).await;
    let backlog_like = |round: usize| CreateStateInput {
        team: "ENG".into(),
        name: format!("Extra{round}"),
        state_type: StateType::Started,
        color: "#112233".into(),
        description: None,
        position: None,
    };
    for round in 0..ROUNDS {
        let st = fx
            .app
            .create_workflow_state(&alice(), backlog_like(round))
            .await
            .unwrap();
        let sid: StateId = st.state.id;
        let del = {
            let app = fx.app.clone();
            async move { app.delete_workflow_state(&alice(), sid, None).await }
        };
        let creates = (0..6).map(|n| {
            let app = fx.app.clone();
            let name = st.state.name.clone();
            async move {
                app.create_issue(
                    &alice(),
                    CreateIssueInput {
                        team: "ENG".into(),
                        title: Some(format!("S{round}.{n}")),
                        state: Some(name),
                        ..Default::default()
                    },
                )
                .await
                .map(|_| ())
            }
        });
        let (d, rs) = tokio::join!(del, futures_join_all(creates));
        assert_no_internal("state archive", &[d.clone()]);
        assert_no_internal("issue create", &rs);
        let stuck: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM issue i JOIN workflow_state s ON s.id = i.state_id \
             WHERE s.archived_at IS NOT NULL",
        )
        .fetch_one(&fx.pool)
        .await
        .unwrap();
        assert_eq!(
            stuck, 0,
            "round {round}: Issues in archiviertem Status ({d:?})"
        );
    }
}

/// B1.11: removeTeams und leadTeam parallel: das Lead-Team bleibt Projektteam.
#[sqlx::test(migrations = "./migrations")]
async fn b1_11_project_remove_team_vs_lead_team(pool: PgPool) {
    let fx = setup(pool).await;
    for round in 0..ROUNDS {
        let created = fx
            .app
            .save_project(
                &alice(),
                SaveProjectInput {
                    name: Some(format!("Proj{round}")),
                    add_teams: vec!["ENG".into(), "WEB".into()],
                    lead_team: Some("ENG".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let id = created.identifier.clone();
        let (a1, a2) = (fx.app.clone(), fx.app.clone());
        let (i1, i2) = (id.clone(), id.clone());
        let (r1, r2) = tokio::join!(
            async move {
                a1.save_project(
                    &alice(),
                    SaveProjectInput {
                        id: Some(i1),
                        remove_teams: vec!["WEB".into()],
                        ..Default::default()
                    },
                )
                .await
                .map(|_| ())
            },
            async move {
                a2.save_project(
                    &alice(),
                    SaveProjectInput {
                        id: Some(i2),
                        lead_team: Some("WEB".into()),
                        ..Default::default()
                    },
                )
                .await
                .map(|_| ())
            }
        );
        assert_no_internal("project teams", &[r1, r2]);
        let orphan: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM project p WHERE NOT EXISTS ( \
               SELECT 1 FROM project_team t WHERE t.project_id = p.id AND t.team_id = p.lead_team_id)",
        )
        .fetch_one(&fx.pool)
        .await
        .unwrap();
        assert_eq!(
            orphan, 0,
            "round {round}: Lead-Team ohne Projektbeteiligung"
        );
    }
}

/// B1.14: parallele issueSubscribe sind idempotent (kein internal, genau ein `changed`).
#[sqlx::test(migrations = "./migrations")]
async fn b1_14_parallel_subscribe_is_idempotent(pool: PgPool) {
    let fx = setup(pool).await;
    for round in 0..ROUNDS {
        let issue = fx.issue(&format!("Sub{round}")).await;
        let calls = (0..4).map(|_| {
            let app = fx.app.clone();
            async move {
                app.subscribe_issue(&ctx_for(SeedUser::Bob), &issue.to_string(), None)
                    .await
            }
        });
        let rs = futures_join_all(calls).await;
        assert_no_internal("subscribe", &rs);
        assert!(rs.iter().all(Result::is_ok), "{rs:?}");
        let changed = rs.iter().filter(|r| r.as_ref().unwrap().changed).count();
        assert_eq!(changed, 1, "round {round}: {rs:?}");
        let n: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM issue_subscription WHERE issue_id = $1 AND user_id = $2",
        )
        .bind(issue)
        .bind(fx.data.bob.0)
        .fetch_one(&fx.pool)
        .await
        .unwrap();
        assert_eq!(n, 1);
    }
}

/// B1.13: Issue endgültig löschen parallel zu Kommentar/Anhang/Sub-Issue: kein internal
/// (FK-Verletzung wird not_found, Deadlocks werden wiederholt).
#[sqlx::test(migrations = "./migrations")]
async fn b1_13_purge_parallel_to_dependent_inserts(pool: PgPool) {
    let fx = setup(pool).await;
    for round in 0..ROUNDS {
        let issue = fx.issue(&format!("Gone{round}")).await;
        let del = {
            let app = fx.app.clone();
            async move {
                app.delete_issue(&alice(), &issue.to_string(), true)
                    .await
                    .map(|_| ())
            }
        };
        let mut others = Vec::new();
        for n in 0..3 {
            let app = fx.app.clone();
            others.push(tokio::spawn(async move {
                app.create_comment(
                    &alice(),
                    CreateCommentInput {
                        parent: Some(ParentSpec::Issue(issue.to_string())),
                        body: format!("c{n}"),
                        ..Default::default()
                    },
                )
                .await
                .map(|_| ())
            }));
            let app = fx.app.clone();
            others.push(tokio::spawn(async move {
                app.create_issue(
                    &alice(),
                    CreateIssueInput {
                        team: "ENG".into(),
                        title: Some(format!("child{n}")),
                        parent: Some(issue.to_string()),
                        ..Default::default()
                    },
                )
                .await
                .map(|_| ())
            }));
            let app = fx.app.clone();
            others.push(tokio::spawn(async move {
                app.link_attachment(
                    &alice(),
                    LinkAttachmentInput {
                        issue: issue.to_string(),
                        url: format!("https://example.com/{n}"),
                        title: None,
                        subtitle: None,
                        metadata: None,
                    },
                )
                .await
                .map(|_| ())
            }));
        }
        let d = del.await;
        let mut results = vec![d];
        for h in others {
            results.push(h.await.unwrap());
        }
        assert_no_internal("purge", &results);
        for r in results.iter().filter_map(|r| r.as_ref().err()) {
            assert!(
                matches!(r.kind, ErrorKind::NotFound | ErrorKind::Conflict),
                "{r:?}"
            );
        }
    }
}
