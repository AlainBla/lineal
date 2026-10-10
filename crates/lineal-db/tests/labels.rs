#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP06: Labels (Issue-, Projekt-, Initiative-Labels) gegen echte Datenbank (Seed `acme`).

use lineal_app::ports::Store;
use lineal_app::ports::repos::label::LabelKind;
use lineal_app::usecases::label::{ListLabelsInput, SaveLabelInput};
use lineal_app::{App, ErrorKind};
use lineal_db::PgStore;
use lineal_domain::enums::LabelGroupType;
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

fn kind<T>(r: Result<T, lineal_app::AppError>) -> ErrorKind {
    r.err().expect("expected an error").kind
}

fn save(name: &str) -> SaveLabelInput {
    SaveLabelInput {
        name: Some(name.into()),
        ..Default::default()
    }
}

const ISSUE: LabelKind = LabelKind::Issue;

#[sqlx::test(migrations = "./migrations")]
async fn create_scopes_names_and_permissions(pool: PgPool) {
    let (app, data) = setup(pool).await;
    let (alice, bob, carol) = (
        ctx_for(SeedUser::Alice),
        ctx_for(SeedUser::Bob),
        ctx_for(SeedUser::Carol),
    );
    // Workspace-Label
    let ws = app
        .save_label(&alice, ISSUE, save("Security"))
        .await
        .unwrap();
    assert!(ws.label.team_id.is_none() && ws.team.is_none());
    assert_eq!(ws.label.color, "#95A2B3");
    // Team-Label (Bob ist Mitglied von WEB, Team-Setting labelManagement = member)
    let fe = app
        .save_label(
            &bob,
            ISSUE,
            SaveLabelInput {
                team_id: Some("WEB".into()),
                color: Some("#112233".into()),
                ..save("Frontend")
            },
        )
        .await
        .unwrap();
    assert_eq!(fe.team.as_ref().unwrap().1, "WEB");
    // Namenskollision Team-Label vs. Workspace-Label und Vorfahren-Team (case-insensitiv)
    let e = app
        .save_label(
            &bob,
            ISSUE,
            SaveLabelInput {
                team_id: Some("WEB".into()),
                ..save("bug")
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Conflict);
    assert!(e.details["conflictingLabels"].as_array().is_some());
    app.save_label(
        &alice,
        ISSUE,
        SaveLabelInput {
            team_id: Some("ENG".into()),
            ..save("Platform-Team")
        },
    )
    .await
    .unwrap();
    assert_eq!(
        kind(
            app.save_label(
                &bob,
                ISSUE,
                SaveLabelInput {
                    team_id: Some("WEB".into()),
                    ..save("PLATFORM-team")
                }
            )
            .await
        ),
        ErrorKind::Conflict
    );
    // Workspace-Label mit dem Namen eines Team-Labels: conflict mit Liste
    let e = app
        .save_label(&alice, ISSUE, save("Frontend"))
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Conflict);
    // Gleichnamig in unverwandten Teams ist erlaubt (ENG-Label vs DES-Label).
    app.save_label(
        &bob,
        ISSUE,
        SaveLabelInput {
            team_id: Some("DES".into()),
            ..save("Brand")
        },
    )
    .await
    .unwrap();
    app.save_label(
        &alice,
        ISSUE,
        SaveLabelInput {
            team_id: Some("ENG".into()),
            ..save("Brand")
        },
    )
    .await
    .unwrap();
    // Reservierte Namen, Farbe, Pflichtfeld
    for bad in ["Status", "priority"] {
        assert_eq!(
            kind(app.save_label(&alice, ISSUE, save(bad)).await),
            ErrorKind::InvalidRequest
        );
    }
    assert_eq!(
        kind(
            app.save_label(
                &alice,
                ISSUE,
                SaveLabelInput {
                    color: Some("red".into()),
                    ..save("X1")
                }
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            app.save_label(&alice, ISSUE, SaveLabelInput::default())
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // Rechte: Carol (Gast) darf keine Workspace-Labels; privates Team DES ist für Alice nicht auffindbar
    assert_eq!(
        kind(app.save_label(&carol, ISSUE, save("Gast")).await),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(
            app.save_label(
                &alice,
                ISSUE,
                SaveLabelInput {
                    team_id: Some("DES".into()),
                    ..save("Heimlich")
                }
            )
            .await
        ),
        ErrorKind::NotFound
    );
    let _ = data;
}

#[sqlx::test(migrations = "./migrations")]
async fn update_by_name_with_team_disambiguation(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let (alice, bob) = (ctx_for(SeedUser::Alice), ctx_for(SeedUser::Bob));
    app.save_label(
        &bob,
        ISSUE,
        SaveLabelInput {
            team_id: Some("DES".into()),
            ..save("Brand")
        },
    )
    .await
    .unwrap();
    let eng = app
        .save_label(
            &alice,
            ISSUE,
            SaveLabelInput {
                team_id: Some("ENG".into()),
                ..save("Brand")
            },
        )
        .await
        .unwrap();
    // Alice sieht nur ihr eigenes „Brand“ (DES ist privat) → eindeutig
    let u = app
        .save_label(
            &alice,
            ISSUE,
            SaveLabelInput {
                id: Some("brand".into()),
                color: Some("#ABCDEF".into()),
                description: Some(Some("Marke".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(u.label.id, eng.label.id);
    assert_eq!(u.label.color, "#ABCDEF");
    assert_eq!(u.label.description.as_deref(), Some("Marke"));
    // Bob sieht beide → ambiguous; mit teamId eindeutig
    let e = app
        .save_label(
            &bob,
            ISSUE,
            SaveLabelInput {
                id: Some("Brand".into()),
                color: Some("#000000".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::AmbiguousReference);
    let ok = app
        .save_label(
            &bob,
            ISSUE,
            SaveLabelInput {
                id: Some("Brand".into()),
                team_id: Some("DES".into()),
                color: Some("#000000".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(ok.team.unwrap().1, "DES");
    // teamId-Änderung abgelehnt
    assert_eq!(
        kind(
            app.save_label(
                &alice,
                ISSUE,
                SaveLabelInput {
                    id: Some(eng.label.id.to_string()),
                    team_id: Some("WEB".into()),
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            app.save_label(
                &alice,
                ISSUE,
                SaveLabelInput {
                    id: Some("gibt-es-nicht".into()),
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn groups_children_retire_and_restore(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let group = app
        .save_label(
            &alice,
            ISSUE,
            SaveLabelInput {
                is_group: Some(true),
                ..save("Type")
            },
        )
        .await
        .unwrap();
    assert!(group.label.is_group);
    assert_eq!(
        group.label.group_type,
        Some(LabelGroupType::SingleSelect),
        "Default singleSelect"
    );
    let multi = app
        .save_label(
            &alice,
            ISSUE,
            SaveLabelInput {
                is_group: Some(true),
                group_type: Some(LabelGroupType::MultiSelect),
                ..save("Area")
            },
        )
        .await
        .unwrap();
    assert_eq!(multi.label.group_type, Some(LabelGroupType::MultiSelect));
    let a = app
        .save_label(
            &alice,
            ISSUE,
            SaveLabelInput {
                parent: Some(Some("Type".into())),
                ..save("Defect")
            },
        )
        .await
        .unwrap();
    app.save_label(
        &alice,
        ISSUE,
        SaveLabelInput {
            parent: Some(Some(group.label.id.to_string())),
            ..save("Chore")
        },
    )
    .await
    .unwrap();
    assert_eq!(a.parent.as_ref().unwrap().1, "Type");
    // Eine Ebene: Kind ist keine Gruppe; Gruppe hat keinen Parent; Parent muss Gruppe sein
    assert_eq!(
        kind(
            app.save_label(
                &alice,
                ISSUE,
                SaveLabelInput {
                    parent: Some(Some("Defect".into())),
                    ..save("Tief")
                }
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
    assert_eq!(
        kind(
            app.save_label(
                &alice,
                ISSUE,
                SaveLabelInput {
                    is_group: Some(true),
                    parent: Some(Some("Type".into())),
                    ..save("G2")
                }
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
    // Gruppe mit Kindern kann keine normale Label werden
    assert_eq!(
        kind(
            app.save_label(
                &alice,
                ISSUE,
                SaveLabelInput {
                    id: Some("Type".into()),
                    is_group: Some(false),
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
    // gleicher Name in verschiedenen Gruppen ist erlaubt, in derselben nicht
    app.save_label(
        &alice,
        ISSUE,
        SaveLabelInput {
            parent: Some(Some("Area".into())),
            ..save("Defect")
        },
    )
    .await
    .unwrap();
    assert_eq!(
        kind(
            app.save_label(
                &alice,
                ISSUE,
                SaveLabelInput {
                    parent: Some(Some("Type".into())),
                    ..save("defect")
                }
            )
            .await
        ),
        ErrorKind::Conflict
    );
    // Retire kaskadiert auf Kinder, idempotent
    let retired = app.retire_label(&alice, ISSUE, "Type").await.unwrap();
    assert!(retired.label.retired_at.is_some());
    assert_eq!(retired.affected_children.len(), 2);
    let again = app.retire_label(&alice, ISSUE, "Type").await.unwrap();
    assert!(again.affected_children.is_empty());
    assert_eq!(again.label.version, retired.label.version, "idempotent");
    // Kind einer retired-Gruppe kann nicht einzeln wiederhergestellt werden
    assert_eq!(
        kind(
            app.restore_label(&alice, ISSUE, &a.label.id.to_string())
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // Restore der Gruppe stellt alle Kinder wieder her (auch separat retired)
    let restored = app.restore_label(&alice, ISSUE, "Type").await.unwrap();
    assert!(restored.label.retired_at.is_none());
    assert_eq!(restored.affected_children.len(), 2);
    app.retire_label(&alice, ISSUE, &a.label.id.to_string())
        .await
        .unwrap();
    let r2 = app.restore_label(&alice, ISSUE, "Type").await.unwrap();
    assert_eq!(r2.affected_children, vec![a.label.id]);
    // Liste: Gruppen nur mit includeGroups; retired enthalten; Teilstring-Filter
    let page = app
        .list_labels(
            &alice,
            ISSUE,
            ListLabelsInput {
                limit: 50,
                include_groups: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let names: Vec<_> = page.items.iter().map(|l| l.label.name.clone()).collect();
    assert!(names.contains(&"Bug".to_string()) && !names.contains(&"Type".to_string()));
    let with_groups = app
        .list_labels(
            &alice,
            ISSUE,
            ListLabelsInput {
                limit: 50,
                include_groups: true,
                name: Some("typ".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(with_groups.items.len(), 1);
    assert!(with_groups.items[0].label.is_group);
}

#[sqlx::test(migrations = "./migrations")]
async fn list_scopes_follow_the_team_hierarchy(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let (alice, bob) = (ctx_for(SeedUser::Alice), ctx_for(SeedUser::Bob));
    app.save_label(
        &alice,
        ISSUE,
        SaveLabelInput {
            team_id: Some("ENG".into()),
            ..save("Backend")
        },
    )
    .await
    .unwrap();
    app.save_label(
        &bob,
        ISSUE,
        SaveLabelInput {
            team_id: Some("WEB".into()),
            ..save("Frontend")
        },
    )
    .await
    .unwrap();
    app.save_label(
        &bob,
        ISSUE,
        SaveLabelInput {
            team_id: Some("DES".into()),
            ..save("Brand")
        },
    )
    .await
    .unwrap();
    let names = |input: ListLabelsInput, who| {
        let app = app.clone();
        async move {
            app.list_labels(&who, ISSUE, ListLabelsInput { limit: 50, ..input })
                .await
                .unwrap()
                .items
                .into_iter()
                .map(|l| l.label.name)
                .collect::<std::collections::BTreeSet<_>>()
        }
    };
    // WEB sieht Workspace + eigene + die von ENG (Vorfahre), nicht DES
    let web = names(
        ListLabelsInput {
            team: Some("WEB".into()),
            ..Default::default()
        },
        alice.clone(),
    )
    .await;
    assert!(web.contains("Bug") && web.contains("Backend") && web.contains("Frontend"));
    assert!(!web.contains("Brand"));
    // ENG sieht Frontend (Sub-Team) nicht
    let eng = names(
        ListLabelsInput {
            team: Some("ENG".into()),
            ..Default::default()
        },
        alice.clone(),
    )
    .await;
    assert!(eng.contains("Backend") && !eng.contains("Frontend"));
    // ohne team: alles Sichtbare; DES nur für Mitglieder
    let all_alice = names(ListLabelsInput::default(), alice.clone()).await;
    assert!(all_alice.contains("Frontend") && !all_alice.contains("Brand"));
    let all_bob = names(ListLabelsInput::default(), bob.clone()).await;
    assert!(all_bob.contains("Brand"));
    assert_eq!(
        kind(
            app.list_labels(
                &alice,
                ISSUE,
                ListLabelsInput {
                    team: Some("DES".into()),
                    limit: 50,
                    ..Default::default()
                }
            )
            .await
        ),
        ErrorKind::NotFound
    );
    // Paginierung
    let p1 = app
        .list_labels(
            &alice,
            ISSUE,
            ListLabelsInput {
                limit: 2,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(p1.items.len(), 2);
    let p2 = app
        .list_labels(
            &alice,
            ISSUE,
            ListLabelsInput {
                limit: 50,
                cursor: p1.next,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(p2.items.len() >= 3);
}

#[sqlx::test(migrations = "./migrations")]
async fn project_and_initiative_labels(pool: PgPool) {
    let (app, _) = setup(pool).await;
    let alice = ctx_for(SeedUser::Alice);
    let pl = app
        .save_label(&alice, LabelKind::Project, save("Strategic"))
        .await
        .unwrap();
    assert!(pl.team.is_none());
    let tl = app
        .save_label(
            &alice,
            LabelKind::Project,
            SaveLabelInput {
                team_id: Some("ENG".into()),
                ..save("Eng-only")
            },
        )
        .await
        .unwrap();
    assert_eq!(tl.team.unwrap().1, "ENG");
    // Ohne team: nur Workspace-Labels; mit team zusätzlich die des Teams
    let ws_only = app
        .list_labels(
            &alice,
            LabelKind::Project,
            ListLabelsInput {
                limit: 50,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(ws_only.items.len(), 1);
    let with_team = app
        .list_labels(
            &alice,
            LabelKind::Project,
            ListLabelsInput {
                limit: 50,
                team: Some("ENG".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(with_team.items.len(), 2);
    // Projekt-Gruppen sind immer multiSelect
    let g = app
        .save_label(
            &alice,
            LabelKind::Project,
            SaveLabelInput {
                is_group: Some(true),
                ..save("Themes")
            },
        )
        .await
        .unwrap();
    assert_eq!(g.label.effective_group_type(), LabelGroupType::MultiSelect);
    // Initiative-Labels: nur Workspace, kein teamId
    let il = app
        .save_label(&alice, LabelKind::Initiative, save("Q4"))
        .await
        .unwrap();
    assert!(il.team.is_none());
    assert_eq!(
        kind(
            app.save_label(
                &alice,
                LabelKind::Initiative,
                SaveLabelInput {
                    team_id: Some("ENG".into()),
                    ..save("Q5")
                }
            )
            .await
        ),
        ErrorKind::InvalidRequest
    );
    app.retire_label(&alice, LabelKind::Initiative, "Q4")
        .await
        .unwrap();
    let list = app
        .list_labels(
            &alice,
            LabelKind::Initiative,
            ListLabelsInput {
                limit: 50,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        list.items[0].label.retired_at.is_some(),
        "retired bleibt in der Liste"
    );
    app.restore_label(&alice, LabelKind::Initiative, "Q4")
        .await
        .unwrap();
    // gleiche Namen in verschiedenen Familien sind unabhängig
    app.save_label(&alice, LabelKind::Project, save("Q4"))
        .await
        .unwrap();
}
