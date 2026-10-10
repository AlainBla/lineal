#![allow(clippy::unwrap_used, clippy::expect_used)]
//! WP11: Volltextsuche (docs/05 §5) gegen echte Datenbank: Ranking, Kennungen inkl. alter
//! Kennungen, Phrasen/Stoppwörter, Ausschnitte – und vor allem: kein Treffer aus privaten Teams.

mod wp11_common;

use lineal_app::RequestCtx;
use lineal_app::ports::repos::view::{SearchKind, SearchOrder};
use lineal_app::usecases::issue::ListIssuesInput;
use lineal_app::usecases::view::{SearchInput, SearchPage};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;
use wp11_common::*;

async fn search(fx: &Fx, ctx: &RequestCtx, term: &str) -> SearchPage {
    fx.app
        .search(
            ctx,
            SearchInput {
                term: term.into(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
}

fn titles(p: &SearchPage) -> Vec<String> {
    p.hits.iter().map(|h| h.title.clone()).collect()
}

async fn list_query(fx: &Fx, ctx: &RequestCtx, q: &str) -> Vec<String> {
    let page = fx
        .app
        .list_issues(
            ctx,
            ListIssuesInput {
                query: Some(q.into()),
                limit: 50,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    page.items
        .iter()
        .map(|v| v.issue.identifier.clone())
        .collect()
}

#[sqlx::test(migrations = "./migrations")]
async fn fulltext_phrases_stopwords_and_ranking(pool: PgPool) {
    let fx = setup(pool).await;
    let d = fx.data;
    let a = fx.issue(d.eng, 101, "Login fails for SSO users", 1).await;
    fx.set(
        a,
        "description = 'Users cannot sign in when the identity provider is slow'",
    )
    .await;
    let b = fx.issue(d.eng, 102, "Checkout page", 1).await;
    fx.set(b, "description = 'The login redirect loops forever'")
        .await;
    let c = fx.issue(d.eng, 103, "Unrelated", 1).await;
    fx.set(c, "description = 'nothing to see'").await;

    // Titel (A) schlägt Beschreibung (B).
    let r = search(&fx, &alice(), "login").await;
    assert_eq!(
        titles(&r)[..2],
        ["Login fails for SSO users", "Checkout page"]
    );
    // Präfix/Wortform: „fail“ findet „fails“.
    assert_eq!(
        titles(&search(&fx, &alice(), "fail").await),
        ["Login fails for SSO users"]
    );
    // UND-Verknüpfung, Stoppwörter außerhalb von Anführungszeichen entfallen.
    assert_eq!(
        titles(&search(&fx, &alice(), "the login fails").await),
        ["Login fails for SSO users"]
    );
    // Phrase: exakte Wortfolge.
    assert_eq!(
        titles(&search(&fx, &alice(), "\"identity provider\"").await),
        ["Login fails for SSO users"]
    );
    assert!(
        search(&fx, &alice(), "\"provider identity\"")
            .await
            .hits
            .is_empty()
    );
    // Leer / nur Stoppwörter → keine Treffer.
    assert!(search(&fx, &alice(), "   ").await.hits.is_empty());
    assert!(search(&fx, &alice(), "the of and").await.hits.is_empty());
    // Typ-Tab.
    let only_projects = fx
        .app
        .search(
            &alice(),
            SearchInput {
                term: "login".into(),
                kinds: vec![SearchKind::Project],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(only_projects.hits.is_empty());
    // Sortierung nach Aktualität.
    fx.set(b, "updated_at = '2026-10-09T12:00:00Z'").await;
    let by_update = fx
        .app
        .search(
            &alice(),
            SearchInput {
                term: "login".into(),
                order: SearchOrder::UpdatedAt,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(titles(&by_update)[0], "Checkout page");
}

#[sqlx::test(migrations = "./migrations")]
async fn identifiers_rank_first_and_old_identifiers_resolve(pool: PgPool) {
    let fx = setup(pool).await;
    let d = fx.data;
    let target = fx.issue(d.eng, 124, "Payment timeout", 1).await;
    let mention = fx.issue(d.eng, 125, "Follow up", 1).await;
    fx.set(
        mention,
        "description = 'See ENG-124 and eng124 for details'",
    )
    .await;
    for term in ["ENG-124", "eng-124", "eng124"] {
        let r = search(&fx, &alice(), term).await;
        assert_eq!(r.hits[0].id, target, "{term}: exact identifier first");
        assert!(r.hits[0].exact);
    }
    // Alias-Tabelle (Teamwechsel) und früherer Team-Key.
    sqlx::query(
        "INSERT INTO issue_identifier_alias (organization_id, identifier, entity_type, issue_id, team_key) \
         VALUES ($1,'OLD-7','issue',$2,'OLD')",
    )
    .bind(d.org.0)
    .bind(target)
    .execute(&fx.pool)
    .await
    .unwrap();
    assert_eq!(search(&fx, &alice(), "OLD-7").await.hits[0].id, target);
    sqlx::query("INSERT INTO team_key (organization_id, team_id, key) VALUES ($1,$2,'LEGACY')")
        .bind(d.org.0)
        .bind(d.eng.0)
        .execute(&fx.pool)
        .await
        .unwrap();
    assert_eq!(search(&fx, &alice(), "LEGACY-124").await.hits[0].id, target);
    // list_issues.query: Kennung trifft genau dieses Issue, auch über alte Kennungen.
    assert_eq!(list_query(&fx, &alice(), "ENG-124").await, ["ENG-124"]);
    assert_eq!(list_query(&fx, &alice(), "OLD-7").await, ["ENG-124"]);
    assert_eq!(list_query(&fx, &alice(), "LEGACY-124").await, ["ENG-124"]);
    // Projekt- und Initiativ-Kennungen.
    let r = search(&fx, &alice(), "P-ENG-1").await;
    assert_eq!(
        (r.hits[0].kind, r.hits[0].identifier.as_deref()),
        (SearchKind::Project, Some("P-ENG-1"))
    );
    let r = search(&fx, &alice(), "i-1").await;
    assert_eq!(
        (r.hits[0].kind, r.hits[0].identifier.as_deref()),
        (SearchKind::Initiative, Some("I-1"))
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn search_never_returns_objects_of_private_teams(pool: PgPool) {
    let fx = setup(pool).await;
    let d = fx.data;
    let secret = fx.issue(d.des, 101, "Zebra launch plan", 1).await;
    fx.set(secret, "description = 'zebra budget numbers'").await;
    sqlx::query("INSERT INTO comment (organization_id, body, user_id, issue_id) VALUES ($1,'zebra comment text',$2,$3)")
        .bind(d.org.0)
        .bind(d.bob.0)
        .bind(secret)
        .execute(&fx.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO issue_identifier_alias (organization_id, identifier, entity_type, issue_id, team_key) \
         VALUES ($1,'DESOLD-3','issue',$2,'DESOLD')",
    )
    .bind(d.org.0)
    .bind(secret)
    .execute(&fx.pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO team_key (organization_id, team_id, key) VALUES ($1,$2,'DESPAST')")
        .bind(d.org.0)
        .bind(d.des.0)
        .execute(&fx.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO document (organization_id, title, content, team_id) VALUES ($1,'Zebra handbook','zebra rules',$2)",
    )
    .bind(d.org.0)
    .bind(d.des.0)
    .execute(&fx.pool)
    .await
    .unwrap();
    // DES-only Projekt.
    let status: Uuid = sqlx::query_scalar(
        "SELECT id FROM project_status WHERE organization_id = $1 AND type = 'started'",
    )
    .bind(d.org.0)
    .fetch_one(&fx.pool)
    .await
    .unwrap();
    let project = Uuid::now_v7();
    sqlx::query("INSERT INTO project (id, organization_id, name, lead_team_id, number, status_id) VALUES ($1,$2,'Zebra rebrand',$3,1,$4)")
        .bind(project)
        .bind(d.org.0)
        .bind(d.des.0)
        .bind(status)
        .execute(&fx.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO project_team (organization_id, project_id, team_id) VALUES ($1,$2,$3)",
    )
    .bind(d.org.0)
    .bind(project)
    .bind(d.des.0)
    .execute(&fx.pool)
    .await
    .unwrap();

    let all = |ctx: RequestCtx, term: &'static str| {
        let fx = &fx;
        async move {
            fx.app
                .search(
                    &ctx,
                    SearchInput {
                        term: term.into(),
                        include_comments: true,
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
        }
    };
    for term in [
        "zebra",
        "\"zebra budget\"",
        "DES-101",
        "DESOLD-3",
        "DESPAST-101",
        "P-DES-1",
        "handbook",
    ] {
        let r = all(alice(), term).await;
        assert!(r.hits.is_empty(), "{term}: {:?}", titles(&r));
        assert!(r.issues.is_empty());
    }
    // Auch über list_issues.query (Titel, Kennung, alte Kennungen).
    for term in ["zebra", "DES-101", "DESOLD-3", "DESPAST-101"] {
        assert!(list_query(&fx, &alice(), term).await.is_empty(), "{term}");
    }
    // Bob ist Mitglied von DES und findet alles.
    let r = all(bob(), "zebra").await;
    let t = titles(&r);
    assert!(t.contains(&"Zebra launch plan".to_owned()));
    assert!(t.contains(&"Zebra handbook".to_owned()));
    assert!(t.contains(&"Zebra rebrand".to_owned()));
    for term in ["DES-101", "DESOLD-3", "DESPAST-101"] {
        assert_eq!(all(bob(), term).await.hits[0].id, secret, "{term}");
    }
    assert_eq!(list_query(&fx, &bob(), "DESOLD-3").await, ["DES-101"]);
    // Der Gast (nur WEB) sieht ebenfalls nichts.
    assert!(all(carol(), "zebra").await.hits.is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn archived_are_included_with_a_penalty_trashed_never_and_guests_skip_initiatives(
    pool: PgPool,
) {
    let fx = setup(pool).await;
    let d = fx.data;
    let live = fx.issue(d.eng, 101, "Rocket engine", 1).await;
    let archived = fx.issue(d.eng, 102, "Rocket engine", 1).await;
    fx.set(archived, "archived_at = '2026-10-01T00:00:00Z'")
        .await;
    let trashed = fx.issue(d.eng, 103, "Rocket engine", 1).await;
    fx.set(
        trashed,
        "trashed = true, trashed_at = now(), archived_at = now()",
    )
    .await;
    let r = search(&fx, &alice(), "rocket").await;
    let ids: Vec<Uuid> = r.hits.iter().map(|h| h.id).collect();
    assert_eq!(
        ids,
        vec![live, archived],
        "archived ranks below live, trashed never appears"
    );
    assert!(r.hits[1].archived);
    let r = fx
        .app
        .search(
            &alice(),
            SearchInput {
                term: "rocket".into(),
                include_archived: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(r.hits.len(), 1);
    // Gast: keine Initiativen, nur sichtbare Teams.
    assert!(search(&fx, &carol(), "Q4 Platform").await.hits.is_empty());
    assert_eq!(
        search(&fx, &alice(), "Q4 Platform").await.hits[0].kind,
        SearchKind::Initiative
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn highlights_are_escaped_and_paging_is_stable(pool: PgPool) {
    let fx = setup(pool).await;
    let d = fx.data;
    let a = fx.issue(d.eng, 101, "Widget", 1).await;
    fx.set(a, "description = 'gadget <script>x</script> & more'")
        .await;
    let r = search(&fx, &alice(), "gadget").await;
    let h = &r.hits[0].highlights;
    assert_eq!(h[0].0, "description");
    assert!(h[0].1.contains("<mark>gadget</mark>"), "{}", h[0].1);
    assert!(!h[0].1.contains("<script>"), "{}", h[0].1);
    assert!(h[0].1.contains("&amp;"), "{}", h[0].1);
    // Paginierung über next_offset.
    for n in 0..5 {
        fx.issue(d.eng, 110 + n, &format!("Pager item {n}"), 1)
            .await;
    }
    let p1 = fx
        .app
        .search(
            &alice(),
            SearchInput {
                term: "pager".into(),
                limit: 2,
                order: SearchOrder::CreatedAt,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(p1.hits.len(), 2);
    assert_eq!(p1.next_offset, Some(2));
    let p3 = fx
        .app
        .search(
            &alice(),
            SearchInput {
                term: "pager".into(),
                limit: 2,
                offset: 4,
                order: SearchOrder::CreatedAt,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!((p3.hits.len(), p3.next_offset), (1, None));
}

#[sqlx::test(migrations = "./migrations")]
async fn search_filter_is_anded_and_comments_are_opt_in(pool: PgPool) {
    let fx = setup(pool).await;
    let d = fx.data;
    let a = fx.issue(d.eng, 101, "Alpha task", 1).await;
    fx.set(a, &format!("assignee_id = '{}'", d.bob.0)).await;
    let b = fx.issue(d.eng, 102, "Alpha chore", 1).await;
    sqlx::query("INSERT INTO comment (organization_id, body, user_id, issue_id) VALUES ($1,'omega hidden in a comment',$2,$3)")
        .bind(d.org.0)
        .bind(d.alice.0)
        .bind(b)
        .execute(&fx.pool)
        .await
        .unwrap();
    let with_filter = fx
        .app
        .search(
            &alice(),
            SearchInput {
                term: "alpha".into(),
                filter: Some(json!({"assignee": {"id": {"in": [d.bob.to_string()]}}})),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(titles(&with_filter), ["Alpha task"]);
    assert!(
        search(&fx, &alice(), "omega").await.hits.is_empty(),
        "comments are opt-in"
    );
    let c = fx
        .app
        .search(
            &alice(),
            SearchInput {
                term: "omega".into(),
                include_comments: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(titles(&c), ["Alpha chore"]);
    assert_eq!(c.hits[0].highlights[0].0, "comment");
    // Referenz auf ein privates Team im Zusatzfilter → not_found.
    assert_eq!(
        kind(
            fx.app
                .search(
                    &alice(),
                    SearchInput {
                        term: "alpha".into(),
                        filter: Some(json!({"team": {"id": {"in": [d.des.to_string()]}}})),
                        ..Default::default()
                    }
                )
                .await
        ),
        lineal_app::ErrorKind::NotFound
    );
}
