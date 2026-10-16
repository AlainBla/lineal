#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP11: Custom Views (personal/Team/Workspace) mit Rechten, `customView` + Zusatzfilter,
//! Anzeigeoptionen und Favoriten gegen echte Datenbank (docs/05 §2–4).

mod wp11_common;

use lineal_app::ErrorKind;
use lineal_app::RequestCtx;
use lineal_app::ports::repos::user::PageRequest;
use lineal_app::ports::repos::view::ViewModel;
use lineal_app::usecases::issue::ListIssuesInput;
use lineal_app::usecases::view::{
    CreateFavoriteInput, CreateViewInput, ListViewsInput, UpdateViewInput, ViewInfo, ViewKeyInput,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;
use wp11_common::*;

fn cv(name: &str, filter: Value) -> CreateViewInput {
    CreateViewInput {
        name: name.into(),
        description: None,
        icon: None,
        color: None,
        model: ViewModel::Issue,
        filter,
        shared: false,
        team: None,
        project: None,
        initiative: None,
        preferences: None,
    }
}

async fn create(fx: &Fx, ctx: &RequestCtx, input: CreateViewInput) -> ViewInfo {
    fx.app.create_view(ctx, input).await.unwrap()
}

async fn view_titles(fx: &Fx, ctx: &RequestCtx, input: ListIssuesInput) -> Vec<String> {
    let page = fx
        .app
        .list_issues(
            ctx,
            ListIssuesInput {
                limit: 250,
                ..input
            },
        )
        .await
        .unwrap();
    let mut t: Vec<String> = page.items.iter().map(|v| v.issue.title.clone()).collect();
    t.sort();
    t
}

fn set(names: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = names.iter().map(|s| (*s).to_owned()).collect();
    v.sort();
    v
}

async fn names(fx: &Fx, ctx: &RequestCtx) -> Vec<String> {
    let page = fx
        .app
        .list_views(
            ctx,
            ListViewsInput {
                page: PageRequest {
                    limit: 250,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let mut n: Vec<String> = page.items.into_iter().map(|v| v.view.name).collect();
    n.sort();
    n
}

#[sqlx::test(migrations = "./migrations")]
async fn a_personal_view_of_someone_else_is_not_found_everywhere(pool: PgPool) {
    let fx = setup(pool).await;
    let v = create(&fx, &bob(), cv("Bobs private", json!({}))).await;
    let id = v.view.id.to_string();
    assert_eq!(names(&fx, &bob()).await, vec!["Bobs private".to_owned()]);
    assert!(
        names(&fx, &alice()).await.is_empty(),
        "list does not show it"
    );
    for reference in [
        id.clone(),
        v.view.slug_id.clone(),
        "Bobs private".to_owned(),
    ] {
        assert_eq!(
            kind(fx.app.get_view(&alice(), &reference).await),
            ErrorKind::NotFound
        );
    }
    assert_eq!(
        kind(
            fx.app
                .update_view(
                    &alice(),
                    UpdateViewInput {
                        reference: id.clone(),
                        name: Some("hijacked".into()),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(fx.app.delete_view(&alice(), &id).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(fx.app.duplicate_view(&alice(), &id).await),
        ErrorKind::NotFound
    );
    // Auch als `customView`-Parameter einer Liste.
    assert_eq!(
        kind(
            fx.app
                .list_issues(
                    &alice(),
                    ListIssuesInput {
                        custom_view: Some(id),
                        limit: 10,
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
    // Der Owner sieht sie weiterhin.
    assert!(
        fx.app
            .get_view(&bob(), &v.view.slug_id)
            .await
            .unwrap()
            .can_edit
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn workspace_team_and_guest_views_follow_the_rights_table(pool: PgPool) {
    let fx = setup(pool).await;
    let d = fx.data;

    // Workspace-View: jeder Nicht-Gast legt an (docs/05 §4.4), Gäste nicht; alle Nicht-Gäste
    // sehen sie, ändern dürfen Owner und Admins (§4.2).
    let mut ws = cv("Company bugs", json!({}));
    ws.shared = true;
    assert_eq!(
        kind(fx.app.create_view(&carol(), ws.clone()).await),
        ErrorKind::Forbidden,
        "guests only get personal views"
    );
    let wsv = create(&fx, &alice(), ws).await;
    assert!(wsv.view.shared && wsv.view.team_id.is_none());
    assert_eq!(names(&fx, &bob()).await, vec!["Company bugs".to_owned()]);
    assert!(
        names(&fx, &carol()).await.is_empty(),
        "guests see no workspace views"
    );
    assert_eq!(
        kind(fx.app.get_view(&carol(), "Company bugs").await),
        ErrorKind::NotFound
    );
    // Bob sieht sie, darf aber nicht ändern/löschen; duplizieren darf er.
    let seen = fx.app.get_view(&bob(), "Company bugs").await.unwrap();
    assert!(!seen.can_edit);
    assert_eq!(
        kind(
            fx.app
                .update_view(
                    &bob(),
                    UpdateViewInput {
                        reference: "Company bugs".into(),
                        name: Some("mine now".into()),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::Forbidden
    );
    assert_eq!(
        kind(fx.app.delete_view(&bob(), "Company bugs").await),
        ErrorKind::Forbidden
    );
    let copy = fx.app.duplicate_view(&bob(), "Company bugs").await.unwrap();
    assert_eq!(copy.view.name, "Company bugs (copy)");
    assert!(!copy.view.shared && copy.view.owner_id == d.bob);

    // Team-View: Bob ist Mitglied von WEB und teilt sie mit dem Team.
    let mut tv = cv("Web board", json!({}));
    tv.shared = true;
    tv.team = Some("WEB".into());
    let web_view = create(&fx, &bob(), tv.clone()).await;
    assert_eq!(web_view.view.team_id, Some(d.web));
    // Carol (Gast, Mitglied von WEB) sieht sie; Alice (Admin, kein WEB-Mitglied) ebenfalls.
    assert!(names(&fx, &carol()).await.contains(&"Web board".to_owned()));
    let alice_sees = fx.app.get_view(&alice(), "Web board").await.unwrap();
    assert!(alice_sees.can_edit, "workspace admins may edit team views");
    // Alice ist kein WEB-Mitglied: sie darf selbst keine Team-View für WEB teilen.
    let mut tv2 = tv.clone();
    tv2.name = "Alice web".into();
    assert_eq!(
        kind(fx.app.create_view(&alice(), tv2).await),
        ErrorKind::Forbidden
    );
    // Gäste: nur persönliche Views.
    let mut gv = cv("Carol shared", json!({}));
    gv.shared = true;
    gv.team = Some("WEB".into());
    assert_eq!(
        kind(fx.app.create_view(&carol(), gv).await),
        ErrorKind::Forbidden
    );
    let carols = create(&fx, &carol(), cv("Carol personal", json!({}))).await;
    assert!(!carols.view.shared);

    // Team-View eines privaten Teams: für Alice ist DES nicht auffindbar.
    let mut dv = cv("Design", json!({}));
    dv.shared = true;
    dv.team = Some("DES".into());
    assert_eq!(
        kind(fx.app.create_view(&alice(), dv.clone()).await),
        ErrorKind::NotFound
    );
    let des_view = create(&fx, &bob(), dv).await;
    assert!(
        !names(&fx, &alice()).await.contains(&"Design".to_owned()),
        "a view of a private team is invisible to non-members"
    );
    assert_eq!(
        kind(
            fx.app
                .get_view(&alice(), &des_view.view.id.to_string())
                .await
        ),
        ErrorKind::NotFound
    );

    // Scope ändern („Save to“): Gäste dürfen ihre View nicht teilen, Mitglieder ins Team schon.
    assert_eq!(
        kind(
            fx.app
                .update_view(
                    &carol(),
                    UpdateViewInput {
                        reference: carols.view.id.to_string(),
                        shared: Some(true),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::Forbidden
    );
    let p = create(&fx, &bob(), cv("Promote me", json!({}))).await;
    let moved = fx
        .app
        .update_view(
            &bob(),
            UpdateViewInput {
                reference: p.view.id.to_string(),
                shared: Some(true),
                team: Some(Some("WEB".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(moved.view.shared && moved.view.team_id == Some(d.web));
    assert_eq!(moved.view.version, p.view.version + 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn view_plus_extra_filter_is_and_and_team_scope_is_implicit(pool: PgPool) {
    let fx = setup(pool).await;
    let d = fx.data;
    let a = fx.issue(d.eng, 101, "Eng bob prio1", 1).await;
    fx.set(a, &format!("assignee_id = '{}', priority = 1", d.bob.0))
        .await;
    let b = fx.issue(d.eng, 102, "Eng bob prio2", 1).await;
    fx.set(b, &format!("assignee_id = '{}', priority = 2", d.bob.0))
        .await;
    let c = fx.issue(d.eng, 103, "Eng alice prio1", 1).await;
    fx.set(c, &format!("assignee_id = '{}', priority = 1", d.alice.0))
        .await;
    let w = fx.issue(d.web, 101, "Web bob prio1", 1).await;
    fx.set(w, &format!("assignee_id = '{}', priority = 1", d.bob.0))
        .await;

    let bob_id = d.bob.to_string();
    let view = create(&fx, &alice(), {
        let mut v = cv("Bob's work", json!({"assignee": {"id": {"in": [bob_id]}}}));
        v.shared = true;
        v
    })
    .await;
    let r = |extra: ListIssuesInput| ListIssuesInput {
        custom_view: Some(view.view.slug_id.clone()),
        ..extra
    };
    // Nur die View.
    assert_eq!(
        view_titles(&fx, &alice(), r(Default::default())).await,
        set(&["Eng bob prio1", "Eng bob prio2", "Web bob prio1"])
    );
    // View UND flacher Parameter.
    assert_eq!(
        view_titles(
            &fx,
            &alice(),
            r(ListIssuesInput {
                priority: Some(1),
                ..Default::default()
            })
        )
        .await,
        set(&["Eng bob prio1", "Web bob prio1"])
    );
    // View UND Zusatzfilter (AST) UND flacher Parameter: nur Verengung.
    assert_eq!(
        view_titles(
            &fx,
            &alice(),
            r(ListIssuesInput {
                priority: Some(1),
                team: Some("ENG".into()),
                filter: Some(json!({"title": {"contains": "Eng"}})),
                ..Default::default()
            })
        )
        .await,
        set(&["Eng bob prio1"])
    );
    // Ein Zusatzfilter kann die View-Bedingung nie erweitern.
    assert_eq!(
        view_titles(
            &fx,
            &alice(),
            r(ListIssuesInput {
                filter: Some(json!({"or": [{"assignee": {"id": {"in": [d.alice.to_string()]}}}, {"priority": {"in": [1, 2]}}]})),
                ..Default::default()
            })
        )
        .await,
        set(&["Eng bob prio1", "Eng bob prio2", "Web bob prio1"])
    );

    // Team-View ohne Teamkriterium bleibt auf ihr Team begrenzt (ohne Sub-Team WEB).
    let mut tv = cv("ENG only", json!({}));
    tv.shared = true;
    tv.team = Some("ENG".into());
    let tv = create(&fx, &alice(), tv).await;
    assert_eq!(
        view_titles(
            &fx,
            &alice(),
            ListIssuesInput {
                custom_view: Some(tv.view.name.clone()),
                ..Default::default()
            }
        )
        .await,
        set(&["Eng bob prio1", "Eng bob prio2", "Eng alice prio1"])
    );
    // Mit `ancestors` im Filter schließt sie Sub-Teams ein (docs/05 §4.1).
    let eng = d.eng.to_string();
    let mut tv2 = cv(
        "ENG tree",
        json!({"team": {"or": [{"id": {"eq": eng}}, {"ancestors": {"some": {"id": {"eq": eng}}}}]}}),
    );
    tv2.shared = true;
    tv2.team = None;
    let tv2 = create(&fx, &alice(), tv2).await;
    assert_eq!(
        view_titles(
            &fx,
            &alice(),
            ListIssuesInput {
                custom_view: Some(tv2.view.id.to_string()),
                ..Default::default()
            }
        )
        .await
        .len(),
        4
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn custom_view_parameter_validation(pool: PgPool) {
    let fx = setup(pool).await;
    let issue_view = create(&fx, &bob(), cv("Issues", json!({}))).await;
    let mut pv = cv("Projects", json!({}));
    pv.model = ViewModel::Project;
    let project_view = create(&fx, &bob(), pv).await;

    // query + customView → invalid_request
    assert_eq!(
        kind(
            fx.app
                .list_issues(
                    &bob(),
                    ListIssuesInput {
                        custom_view: Some("Issues".into()),
                        query: Some("login".into()),
                        limit: 10,
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // falscher Typ → invalid_request
    assert_eq!(
        kind(
            fx.app
                .list_issues(
                    &bob(),
                    ListIssuesInput {
                        custom_view: Some(project_view.view.id.to_string()),
                        limit: 10,
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // unbekannt → not_found
    assert_eq!(
        kind(
            fx.app
                .list_issues(
                    &bob(),
                    ListIssuesInput {
                        custom_view: Some(Uuid::new_v4().to_string()),
                        limit: 10,
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
    // Referenz per URL (letztes Segment: Slug).
    let url = format!(
        "https://app.example/acme/view/issues-{}",
        issue_view.view.slug_id
    );
    assert!(
        fx.app
            .list_issues(
                &bob(),
                ListIssuesInput {
                    custom_view: Some(url),
                    limit: 10,
                    ..Default::default()
                }
            )
            .await
            .is_ok()
    );
    // doppelter Name → ambiguous_reference
    create(&fx, &bob(), cv("Issues", json!({"priority": {"in": [1]}}))).await;
    assert_eq!(
        kind(fx.app.get_view(&bob(), "issues").await),
        ErrorKind::AmbiguousReference
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn saving_validates_normalizes_and_checks_references(pool: PgPool) {
    let fx = setup(pool).await;
    let d = fx.data;
    // Normalisierung: eq → in mit einem Element, Schlüssel sortiert.
    let v = create(
        &fx,
        &bob(),
        cv(
            "Normalized",
            json!({"priority": {"eq": 1}, "assignee": {"id": {"eq": d.bob.to_string()}}}),
        ),
    )
    .await;
    assert_eq!(
        v.view.filter["assignee"]["id"]["in"],
        json!([d.bob.to_string()])
    );
    assert!(v.view.filter["assignee"]["id"].get("eq").is_none());
    // Schemaverstoß → invalid_request mit details.path
    let e = fx
        .app
        .create_view(&bob(), cv("Bad", json!({"priority": {"in": [9]}})))
        .await
        .unwrap_err();
    assert_eq!(e.kind, ErrorKind::InvalidRequest);
    assert!(e.details["path"].as_str().unwrap().starts_with("/priority"));
    // Referenzen: unbekannte ID und privates Team → not_found
    for f in [
        json!({"assignee": {"id": {"in": [Uuid::new_v4().to_string()]}}}),
        json!({"team": {"id": {"in": [d.des.to_string()]}}}),
    ] {
        assert_eq!(
            kind(fx.app.create_view(&alice(), cv("Ref", f)).await),
            ErrorKind::NotFound
        );
    }
    // Name/Icon/Farbe
    for bad in [
        CreateViewInput {
            name: " ".into(),
            ..cv("x", json!({}))
        },
        CreateViewInput {
            name: "n".repeat(81),
            ..cv("x", json!({}))
        },
        CreateViewInput {
            color: Some("red".into()),
            ..cv("x", json!({}))
        },
        CreateViewInput {
            icon: Some("<script>".into()),
            ..cv("x", json!({}))
        },
    ] {
        assert_eq!(
            kind(fx.app.create_view(&bob(), bad).await),
            ErrorKind::InvalidRequest
        );
    }
    // Optimistic Locking
    let upd = |version| UpdateViewInput {
        reference: v.view.id.to_string(),
        expected_version: Some(version),
        name: Some("Renamed".into()),
        ..Default::default()
    };
    let ok = fx
        .app
        .update_view(&bob(), upd(v.view.version))
        .await
        .unwrap();
    assert_eq!(ok.view.name, "Renamed");
    assert_eq!(
        kind(fx.app.update_view(&bob(), upd(v.view.version)).await),
        ErrorKind::Conflict
    );
    // Archivierte Views verschwinden aus der Liste und der Auflösung.
    fx.app
        .update_view(
            &bob(),
            UpdateViewInput {
                reference: v.view.id.to_string(),
                archived: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!names(&fx, &bob()).await.contains(&"Renamed".to_owned()));
}

#[sqlx::test(migrations = "./migrations")]
async fn mutations_write_outbox_events(pool: PgPool) {
    let fx = setup(pool).await;
    let v = create(&fx, &bob(), cv("Evented", json!({}))).await;
    fx.app
        .update_view(
            &bob(),
            UpdateViewInput {
                reference: v.view.id.to_string(),
                name: Some("Evented 2".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    fx.app
        .delete_view(&bob(), &v.view.id.to_string())
        .await
        .unwrap();
    let types: Vec<String> = sqlx::query_scalar(
        "SELECT type FROM outbox_event WHERE type LIKE 'CustomView%' ORDER BY sync_id",
    )
    .fetch_all(&fx.pool)
    .await
    .unwrap();
    assert_eq!(
        types,
        vec![
            "CustomViewCreated",
            "CustomViewUpdated",
            "CustomViewRemoved"
        ]
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn display_options_layers_defaults_and_permissions(pool: PgPool) {
    let fx = setup(pool).await;
    let mut ws = cv("Shared", json!({}));
    ws.shared = true;
    let v = create(&fx, &alice(), ws).await;
    let key = ViewKeyInput {
        custom_view: Some(v.view.slug_id.clone()),
        ..Default::default()
    };
    // System-Default.
    let eff = fx.app.view_preferences(&bob(), key.clone()).await.unwrap();
    assert_eq!(eff.effective["grouping"], "status");
    assert!(!eff.has_personal && !eff.has_default);
    // Jede Änderung ist persönlich: Bob ändert, Alice bleibt unberührt.
    let eff = fx
        .app
        .update_view_preferences(
            &bob(),
            key.clone(),
            json!({"grouping": "assignee", "layout": "board"}),
        )
        .await
        .unwrap();
    assert_eq!(eff.effective["grouping"], "assignee");
    assert!(eff.has_personal);
    let alices = fx
        .app
        .view_preferences(&alice(), key.clone())
        .await
        .unwrap();
    assert_eq!(alices.effective["grouping"], "status");
    // Ungültige Werte werden abgelehnt.
    assert_eq!(
        kind(
            fx.app
                .update_view_preferences(&bob(), key.clone(), json!({"grouping": "nonsense"}))
                .await
        ),
        ErrorKind::InvalidRequest
    );
    // „Set default for everyone“: Bob darf nicht (kein Owner/Admin), Alice schon.
    assert_eq!(
        kind(
            fx.app
                .set_view_preferences_default(&bob(), key.clone())
                .await
        ),
        ErrorKind::Forbidden
    );
    fx.app
        .update_view_preferences(
            &alice(),
            key.clone(),
            json!({"ordering": "updatedAt", "orderDirection": "asc"}),
        )
        .await
        .unwrap();
    let after = fx
        .app
        .set_view_preferences_default(&alice(), key.clone())
        .await
        .unwrap();
    assert!(
        after.has_default && !after.has_personal,
        "personal row is dropped"
    );
    // Bob behält seinen Override, übernimmt aber den neuen Default für alles andere.
    let bobs = fx.app.view_preferences(&bob(), key.clone()).await.unwrap();
    assert_eq!(bobs.effective["grouping"], "assignee");
    assert_eq!(bobs.effective["ordering"], "updatedAt");
    // Reset löscht nur die persönliche Zeile.
    let reset = fx
        .app
        .reset_view_preferences(&bob(), key.clone())
        .await
        .unwrap();
    assert_eq!(reset.effective["grouping"], "status");
    assert_eq!(reset.effective["ordering"], "updatedAt");
    // Vordefinierte Ansicht: Board-Default und My-Issues-Default.
    let team = ViewKeyInput {
        view_type: Some("teamActive".into()),
        team: Some("ENG".into()),
        ..Default::default()
    };
    let t = fx.app.view_preferences(&bob(), team).await.unwrap();
    assert_eq!(t.effective["layout"], "board");
    let my = ViewKeyInput {
        view_type: Some("myIssuesAssigned".into()),
        ..Default::default()
    };
    assert_eq!(
        fx.app.view_preferences(&bob(), my).await.unwrap().effective["showCompletedIssues"],
        "pastDay"
    );
    // Einstellungen einer unsichtbaren View → not_found.
    let hidden = create(&fx, &bob(), cv("Hidden", json!({}))).await;
    assert_eq!(
        kind(
            fx.app
                .view_preferences(
                    &alice(),
                    ViewKeyInput {
                        custom_view: Some(hidden.view.id.to_string()),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn favorites_are_personal_and_hide_invisible_targets(pool: PgPool) {
    let fx = setup(pool).await;
    let d = fx.data;
    let mut ws = cv("Shared fav", json!({}));
    ws.shared = true;
    let v = create(&fx, &alice(), ws).await;
    let fav = fx
        .app
        .create_favorite(
            &bob(),
            CreateFavoriteInput {
                kind: "customView".into(),
                target: Some(v.view.slug_id.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    // idempotent
    let again = fx
        .app
        .create_favorite(
            &bob(),
            CreateFavoriteInput {
                kind: "customView".into(),
                target: Some(v.view.id.to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(again.id, fav.id);
    assert_eq!(fx.app.list_favorites(&bob()).await.unwrap().len(), 1);
    assert!(
        fx.app.list_favorites(&alice()).await.unwrap().is_empty(),
        "favorites are personal"
    );
    // Fremder Favorit → not_found
    assert_eq!(
        kind(fx.app.delete_favorite(&alice(), fav.id).await),
        ErrorKind::NotFound
    );
    // Ordner und Zuordnung
    let folder = fx
        .app
        .create_favorite(
            &bob(),
            CreateFavoriteInput {
                kind: "folder".into(),
                folder_name: Some("Planning".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let team_fav = fx
        .app
        .create_favorite(
            &bob(),
            CreateFavoriteInput {
                kind: "team".into(),
                target: Some(d.web.to_string()),
                parent: Some(folder.id),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(team_fav.parent_id, Some(folder.id));
    assert!(
        folder.sort_order > fav.sort_order,
        "new favorites go to the end of their level"
    );
    // Privates Team: Alice kann DES nicht favorisieren (not_found), Bob schon.
    assert_eq!(
        kind(
            fx.app
                .create_favorite(
                    &alice(),
                    CreateFavoriteInput {
                        kind: "team".into(),
                        target: Some(d.des.to_string()),
                        ..Default::default()
                    }
                )
                .await
        ),
        ErrorKind::NotFound
    );
    // Entzieht der Owner die Sichtbarkeit (Workspace → persönlich), verschwindet der Favorit.
    fx.app
        .update_view(
            &alice(),
            UpdateViewInput {
                reference: v.view.id.to_string(),
                shared: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let left: Vec<String> = fx
        .app
        .list_favorites(&bob())
        .await
        .unwrap()
        .into_iter()
        .map(|f| f.kind)
        .collect();
    assert!(!left.contains(&"customView".to_owned()), "{left:?}");
    // Ordner löschen: Kinder wandern auf die oberste Ebene.
    fx.app.delete_favorite(&bob(), folder.id).await.unwrap();
    let rest = fx.app.list_favorites(&bob()).await.unwrap();
    assert!(
        rest.iter()
            .any(|f| f.id == team_fav.id && f.parent_id.is_none())
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn custom_views_filter_projects_and_initiatives(pool: PgPool) {
    use lineal_app::usecases::initiative::ListInitiativesInput;
    use lineal_app::usecases::project::ListProjectsInput;
    use lineal_testkit::seed::planning::seed_initiative_id;
    let fx = setup(pool).await;
    let mk = |name: &str, model: ViewModel, filter: Value| CreateViewInput {
        model,
        ..cv(name, filter)
    };
    create(
        &fx,
        &alice(),
        mk("P2", ViewModel::Project, json!({"priority": {"in": [2]}})),
    )
    .await;
    create(
        &fx,
        &alice(),
        mk("P1", ViewModel::Project, json!({"priority": {"in": [1]}})),
    )
    .await;
    create(
        &fx,
        &alice(),
        mk(
            "InI1",
            ViewModel::Project,
            json!({"initiatives": {"some": {"id": {"in": [seed_initiative_id().to_string()]}}}}),
        ),
    )
    .await;
    create(
        &fx,
        &alice(),
        mk(
            "Active",
            ViewModel::Initiative,
            json!({"status": {"eq": "Active"}}),
        ),
    )
    .await;
    create(
        &fx,
        &alice(),
        mk(
            "Done",
            ViewModel::Initiative,
            json!({"status": {"eq": "Completed"}}),
        ),
    )
    .await;
    let projects = |view: &str| {
        let fx = &fx;
        let view = view.to_owned();
        async move {
            fx.app
                .list_projects(
                    &alice(),
                    ListProjectsInput {
                        custom_view: Some(view),
                        page: PageRequest {
                            limit: 50,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                )
                .await
        }
    };
    assert_eq!(projects("P2").await.unwrap().0.items.len(), 1);
    assert_eq!(projects("P1").await.unwrap().0.items.len(), 0);
    assert_eq!(projects("InI1").await.unwrap().0.items.len(), 1);
    // Falscher Typ → invalid_request.
    assert_eq!(kind(projects("Active").await), ErrorKind::InvalidRequest);
    let initiatives = |view: &str| {
        let fx = &fx;
        let view = view.to_owned();
        async move {
            fx.app
                .list_initiatives(
                    &alice(),
                    ListInitiativesInput {
                        custom_view: Some(view),
                        page: PageRequest {
                            limit: 50,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                )
                .await
        }
    };
    assert_eq!(initiatives("Active").await.unwrap().0.items.len(), 1);
    assert_eq!(initiatives("Done").await.unwrap().0.items.len(), 0);
    assert_eq!(kind(initiatives("P2").await), ErrorKind::InvalidRequest);
    // query + customView → invalid_request; fremde persönliche View → not_found.
    let e = fx
        .app
        .list_projects(
            &alice(),
            ListProjectsInput {
                custom_view: Some("P2".into()),
                query: Some("x".into()),
                page: PageRequest {
                    limit: 5,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(e), ErrorKind::InvalidRequest);
    let e = fx
        .app
        .list_projects(
            &bob(),
            ListProjectsInput {
                custom_view: Some("P2".into()),
                page: PageRequest {
                    limit: 5,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await;
    assert_eq!(kind(e), ErrorKind::NotFound);
}

#[sqlx::test(migrations = "./migrations")]
async fn project_issue_tab_options_are_per_project_and_default_needs_manage_rights(pool: PgPool) {
    use lineal_testkit::seed::planning::seed_project_id;
    let fx = setup(pool).await;
    let key = ViewKeyInput {
        view_type: Some("projectIssues".into()),
        project: Some(seed_project_id().to_string()),
        ..Default::default()
    };
    // System-Default einer Issue-Liste; ohne Projekt ist der Schlüssel ungültig.
    let eff = fx.app.view_preferences(&bob(), key.clone()).await.unwrap();
    assert_eq!(eff.effective["grouping"], "status");
    assert_eq!(eff.effective["nestedSubIssues"], "hide");
    let no_project = ViewKeyInput {
        view_type: Some("projectIssues".into()),
        ..Default::default()
    };
    assert_eq!(
        kind(fx.app.view_preferences(&bob(), no_project).await),
        ErrorKind::InvalidRequest
    );
    // Persönliche Änderung gilt nur für Bob.
    fx.app
        .update_view_preferences(&bob(), key.clone(), json!({"grouping": "milestone"}))
        .await
        .unwrap();
    let mine = fx.app.view_preferences(&bob(), key.clone()).await.unwrap();
    assert_eq!(mine.effective["grouping"], "milestone");
    assert!(mine.has_personal);
    let other = fx
        .app
        .view_preferences(&alice(), key.clone())
        .await
        .unwrap();
    assert_eq!(other.effective["grouping"], "status");
    // „Set default for everyone“ braucht Verwaltungsrecht am Projekt; Gäste scheitern.
    assert!(
        fx.app
            .set_view_preferences_default(&carol(), key.clone())
            .await
            .is_err()
    );
    let set = fx
        .app
        .set_view_preferences_default(&bob(), key.clone())
        .await
        .unwrap();
    assert!(set.has_default && !set.has_personal);
    let again = fx.app.view_preferences(&alice(), key).await.unwrap();
    assert_eq!(again.effective["grouping"], "milestone");
}
