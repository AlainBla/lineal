//! Schema-Tests (WP S2): Migrationen laufen durch, Stichproben-Inserts und Constraint-Verletzungen.
//!
//! Alle IDs werden in SQL mit `gen_random_uuid()` erzeugt und als Text (`id::text`) zurueckgelesen,
//! damit der Test nicht vom `uuid`-Feature von sqlx abhaengt.

use sqlx::PgPool;

const PG_UNIQUE_VIOLATION: &str = "23505";
const PG_FOREIGN_KEY_VIOLATION: &str = "23503";
const PG_CHECK_VIOLATION: &str = "23514";

/// Liefert den SQLSTATE-Code eines Datenbankfehlers.
fn sqlstate(err: &sqlx::Error) -> Option<String> {
    err.as_database_error()
        .and_then(|e| e.code())
        .map(|c| c.into_owned())
}

fn assert_sqlstate(err: &sqlx::Error, expected: &str) {
    assert_eq!(
        sqlstate(err).as_deref(),
        Some(expected),
        "unerwarteter Fehler: {err}"
    );
}

struct Seed {
    org: String,
    user: String,
    team: String,
    state: String,
}

async fn scalar(pool: &PgPool, sql: &str, binds: &[&str]) -> Result<String, sqlx::Error> {
    let mut q = sqlx::query_scalar::<_, String>(sql);
    for b in binds {
        q = q.bind(*b);
    }
    q.fetch_one(pool).await
}

async fn insert_team(
    pool: &PgPool,
    org: &str,
    name: &str,
    key: &str,
) -> Result<String, sqlx::Error> {
    scalar(
        pool,
        "insert into team (organization_id, name, key) values ($1::uuid, $2, $3) returning id::text",
        &[org, name, key],
    )
    .await
}

async fn insert_state(
    pool: &PgPool,
    org: &str,
    team: &str,
    name: &str,
    ty: &str,
) -> Result<String, sqlx::Error> {
    scalar(
        pool,
        "insert into workflow_state (organization_id, team_id, name, type, color, position) \
         values ($1::uuid, $2::uuid, $3, $4, '#5E6AD2', 1) returning id::text",
        &[org, team, name, ty],
    )
    .await
}

async fn insert_user(
    pool: &PgPool,
    org: &str,
    display_name: &str,
) -> Result<String, sqlx::Error> {
    scalar(
        pool,
        "insert into \"user\" (organization_id, name, display_name, email) \
         values ($1::uuid, $2, $2, $2 || '@example.test') returning id::text",
        &[org, display_name],
    )
    .await
}

async fn insert_issue(
    pool: &PgPool,
    s: &Seed,
    number: i32,
    title: &str,
) -> Result<String, sqlx::Error> {
    sqlx::query_scalar::<_, String>(
        "insert into issue (organization_id, team_id, number, title, description, state_id, creator_id) \
         values ($1::uuid, $2::uuid, $3, $4, 'Weiterleitung nach SSO schlaegt fehl', $5::uuid, $6::uuid) \
         returning id::text",
    )
    .bind(&s.org)
    .bind(&s.team)
    .bind(number)
    .bind(title)
    .bind(&s.state)
    .bind(&s.user)
    .fetch_one(pool)
    .await
}

/// Org, User, Team ENG, Status "Todo" (unstarted); setzt `team.default_issue_state_id`.
async fn seed(pool: &PgPool) -> Result<Seed, sqlx::Error> {
    let org = scalar(
        pool,
        "insert into organization (name, url_key) values ('Acme Inc.', 'acme') returning id::text",
        &[],
    )
    .await?;
    let user = insert_user(pool, &org, "alice").await?;
    let team = insert_team(pool, &org, "Engineering", "ENG").await?;
    let state = insert_state(pool, &org, &team, "Todo", "unstarted").await?;
    sqlx::query(
        "update team set default_issue_state_id = $1::uuid where id = $2::uuid and organization_id = $3::uuid",
    )
    .bind(&state)
    .bind(&team)
    .bind(&org)
    .execute(pool)
    .await?;
    Ok(Seed {
        org,
        user,
        team,
        state,
    })
}

#[sqlx::test(migrations = "./migrations")]
async fn migrations_create_expected_tables(pool: PgPool) -> Result<(), sqlx::Error> {
    let expected = [
        "organization",
        "user",
        "user_session",
        "team",
        "team_key",
        "team_membership",
        "workflow_state",
        "time_schedule",
        "time_schedule_entry",
        "triage_responsibility",
        "api_key",
        "oauth_application",
        "oauth_token",
        "webhook",
        "invitation",
        "issue",
        "issue_identifier_alias",
        "issue_label",
        "issue_to_label",
        "issue_relation",
        "comment",
        "reaction",
        "attachment",
        "issue_history",
        "issue_subscription",
        "sla_rule",
        "draft",
        "cycle",
        "project",
        "project_team",
        "project_member",
        "project_status",
        "project_milestone",
        "project_label",
        "project_to_label",
        "project_update",
        "project_relation",
        "initiative",
        "initiative_parent",
        "initiative_to_project",
        "initiative_update",
        "initiative_label",
        "initiative_to_label",
        "entity_external_link",
        "custom_view",
        "view_preferences",
        "favorite",
        "notification",
        "notification_subscription",
        "notification_channel_preference",
        "document",
        "document_subscription",
        "template",
        "document_content",
        "document_content_history",
        "import_run",
        "import_mapping",
        "import_issue",
        "import_credential",
    ];
    for table in expected {
        let found: i64 = sqlx::query_scalar(
            "select count(*) from information_schema.tables \
             where table_schema = 'public' and table_name = $1",
        )
        .bind(table)
        .fetch_one(&pool)
        .await?;
        assert_eq!(found, 1, "Tabelle {table} fehlt");
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn smoke_inserts_org_user_team_state_issue_label_comment_project(
    pool: PgPool,
) -> Result<(), sqlx::Error> {
    let s = seed(&pool).await?;

    // Issue: Kennung und state_type werden per Trigger gepflegt, search_vector ist generiert.
    let issue = insert_issue(&pool, &s, 1, "Login-Fehler").await?;
    let (identifier, state_type, fts_hit): (String, String, bool) = sqlx::query_as(
        "select identifier, state_type, search_vector @@ to_tsquery('simple', 'sso') \
         from issue where id = $1::uuid",
    )
    .bind(&issue)
    .fetch_one(&pool)
    .await?;
    assert_eq!(identifier, "ENG-1");
    assert_eq!(state_type, "unstarted");
    assert!(fts_hit, "Volltext findet Begriff aus der Beschreibung");

    // Label + Zuordnung.
    let label = scalar(
        &pool,
        "insert into issue_label (organization_id, name, color) \
         values ($1::uuid, 'Bug', '#FF0000') returning id::text",
        &[&s.org],
    )
    .await?;
    sqlx::query(
        "insert into issue_to_label (organization_id, issue_id, label_id) \
         values ($1::uuid, $2::uuid, $3::uuid)",
    )
    .bind(&s.org)
    .bind(&issue)
    .bind(&label)
    .execute(&pool)
    .await?;

    // Kommentar.
    sqlx::query(
        "insert into comment (organization_id, body, user_id, issue_id) \
         values ($1::uuid, 'Reproduziert.', $2::uuid, $3::uuid)",
    )
    .bind(&s.org)
    .bind(&s.user)
    .bind(&issue)
    .execute(&pool)
    .await?;

    // Projekt: Projektstatus, Projekt und ProjectTeam in einer Transaktion; danach Issue zuordnen.
    let project_status = scalar(
        &pool,
        "insert into project_status (organization_id, name, type, color) \
         values ($1::uuid, 'Backlog', 'backlog', '#95A2B3') returning id::text",
        &[&s.org],
    )
    .await?;
    let mut tx = pool.begin().await?;
    let project: String = sqlx::query_scalar(
        "insert into project (organization_id, name, lead_team_id, number, status_id) \
         values ($1::uuid, 'Checkout-Relaunch', $2::uuid, 1, $3::uuid) returning id::text",
    )
    .bind(&s.org)
    .bind(&s.team)
    .bind(&project_status)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "insert into project_team (organization_id, project_id, team_id) \
         values ($1::uuid, $2::uuid, $3::uuid)",
    )
    .bind(&s.org)
    .bind(&project)
    .bind(&s.team)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    sqlx::query("update issue set project_id = $1::uuid where id = $2::uuid")
        .bind(&project)
        .bind(&issue)
        .execute(&pool)
        .await?;

    // Dokument mit genau einem Parent (Projekt) und ein Team-Dokument.
    sqlx::query("insert into document (organization_id, title, project_id) values ($1::uuid, 'Spec', $2::uuid)")
        .bind(&s.org)
        .bind(&project)
        .execute(&pool)
        .await?;
    sqlx::query("insert into document (organization_id, title, team_id) values ($1::uuid, 'Team-Seite', $2::uuid)")
        .bind(&s.org)
        .bind(&s.team)
        .execute(&pool)
        .await?;

    // Gespeicherte Spalte last_issue_number existiert mit Default 0.
    let counter: i32 = sqlx::query_scalar("select last_issue_number from team where id = $1::uuid")
        .bind(&s.team)
        .fetch_one(&pool)
        .await?;
    assert_eq!(counter, 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn duplicate_issue_number_in_team_is_rejected(pool: PgPool) -> Result<(), sqlx::Error> {
    let s = seed(&pool).await?;
    insert_issue(&pool, &s, 1, "Erstes").await?;
    let err = insert_issue(&pool, &s, 1, "Zweites mit gleicher Nummer")
        .await
        .expect_err("gleiche Nummer im selben Team muss scheitern");
    assert_sqlstate(&err, PG_UNIQUE_VIOLATION);

    // Gleiche Nummer in einem anderen Team ist erlaubt.
    let other_team = insert_team(&pool, &s.org, "Design", "DES").await?;
    let other_state = insert_state(&pool, &s.org, &other_team, "Todo", "unstarted").await?;
    let other = Seed {
        org: s.org.clone(),
        user: s.user.clone(),
        team: other_team,
        state: other_state,
    };
    insert_issue(&pool, &other, 1, "Erstes in DES").await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn invalid_state_type_is_rejected(pool: PgPool) -> Result<(), sqlx::Error> {
    let s = seed(&pool).await?;
    let err = insert_state(&pool, &s.org, &s.team, "Kaputt", "bogus")
        .await
        .expect_err("ungueltiger state type muss scheitern");
    assert_sqlstate(&err, PG_CHECK_VIOLATION);

    // Kategorie ist nach Anlage unveraenderlich.
    let err = sqlx::query("update workflow_state set type = 'started' where id = $1::uuid")
        .bind(&s.state)
        .execute(&pool)
        .await
        .expect_err("type ist unveraenderlich");
    assert_sqlstate(&err, PG_CHECK_VIOLATION);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn document_with_two_parents_is_rejected(pool: PgPool) -> Result<(), sqlx::Error> {
    let s = seed(&pool).await?;
    let issue = insert_issue(&pool, &s, 1, "Mit Dokument").await?;
    let project_status = scalar(
        &pool,
        "insert into project_status (organization_id, name, type, color) \
         values ($1::uuid, 'Backlog', 'backlog', '#95A2B3') returning id::text",
        &[&s.org],
    )
    .await?;
    let mut tx = pool.begin().await?;
    let project: String = sqlx::query_scalar(
        "insert into project (organization_id, name, lead_team_id, number, status_id) \
         values ($1::uuid, 'P', $2::uuid, 1, $3::uuid) returning id::text",
    )
    .bind(&s.org)
    .bind(&s.team)
    .bind(&project_status)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "insert into project_team (organization_id, project_id, team_id) \
         values ($1::uuid, $2::uuid, $3::uuid)",
    )
    .bind(&s.org)
    .bind(&project)
    .bind(&s.team)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    // Zwei Parents (Projekt + Issue) -> Check-Constraint.
    let err = sqlx::query(
        "insert into document (organization_id, title, project_id, issue_id) \
         values ($1::uuid, 'Zwei Eltern', $2::uuid, $3::uuid)",
    )
    .bind(&s.org)
    .bind(&project)
    .bind(&issue)
    .execute(&pool)
    .await
    .expect_err("zwei Parents muessen scheitern");
    assert_sqlstate(&err, PG_CHECK_VIOLATION);

    // Kein Parent -> ebenfalls abgelehnt.
    let err = sqlx::query("insert into document (organization_id, title) values ($1::uuid, 'Ohne Eltern')")
        .bind(&s.org)
        .execute(&pool)
        .await
        .expect_err("kein Parent muss scheitern");
    assert_sqlstate(&err, PG_CHECK_VIOLATION);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn team_key_with_hyphen_or_reserved_letter_is_rejected(
    pool: PgPool,
) -> Result<(), sqlx::Error> {
    let s = seed(&pool).await?;
    for (name, key) in [
        ("Bindestrich", "AB-C"),
        ("Reserviert P", "P"),
        ("Reserviert I", "I"),
        ("Kleinbuchstaben", "eng"),
        ("Zu lang", "ABCDEFGH"),
        ("Ziffer vorn", "1AB"),
    ] {
        let err = insert_team(&pool, &s.org, name, key)
            .await
            .expect_err("ungueltiger Team-Key muss scheitern");
        assert_sqlstate(&err, PG_CHECK_VIOLATION);
    }

    // Gleicher Key im selben Workspace -> Unique-Verletzung.
    let err = insert_team(&pool, &s.org, "Engineering 2", "ENG")
        .await
        .expect_err("doppelter Key muss scheitern");
    assert_sqlstate(&err, PG_UNIQUE_VIOLATION);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn composite_foreign_keys_enforce_tenant_and_team_consistency(
    pool: PgPool,
) -> Result<(), sqlx::Error> {
    let s = seed(&pool).await?;

    // Status eines anderen Teams darf nicht an ein Issue dieses Teams.
    let other_team = insert_team(&pool, &s.org, "Design", "DES").await?;
    let foreign_state = insert_state(&pool, &s.org, &other_team, "Todo", "unstarted").await?;
    let bad = Seed {
        org: s.org.clone(),
        user: s.user.clone(),
        team: s.team.clone(),
        state: foreign_state,
    };
    let err = insert_issue(&pool, &bad, 1, "Falscher Status")
        .await
        .expect_err("Status eines anderen Teams muss scheitern");
    assert_sqlstate(&err, PG_FOREIGN_KEY_VIOLATION);

    // User eines anderen Workspace darf nicht referenziert werden.
    let org2 = scalar(
        &pool,
        "insert into organization (name, url_key) values ('Other', 'other') returning id::text",
        &[],
    )
    .await?;
    let stranger = insert_user(&pool, &org2, "zed").await?;
    let err = sqlx::query(
        "insert into issue (organization_id, team_id, number, title, state_id, assignee_id) \
         values ($1::uuid, $2::uuid, 1, 'Fremder Assignee', $3::uuid, $4::uuid)",
    )
    .bind(&s.org)
    .bind(&s.team)
    .bind(&s.state)
    .bind(&stranger)
    .execute(&pool)
    .await
    .expect_err("Cross-Tenant-Referenz muss scheitern");
    assert_sqlstate(&err, PG_FOREIGN_KEY_VIOLATION);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn team_key_change_updates_issue_identifiers_and_reserves_old_key(
    pool: PgPool,
) -> Result<(), sqlx::Error> {
    let s = seed(&pool).await?;
    let issue = insert_issue(&pool, &s, 7, "Umbenennung").await?;

    // Core-Ablauf bei Key-Wechsel: alten Key in die Historie, neuen Key setzen.
    sqlx::query(
        "insert into team_key (organization_id, team_id, key) values ($1::uuid, $2::uuid, 'ENG')",
    )
    .bind(&s.org)
    .bind(&s.team)
    .execute(&pool)
    .await?;
    sqlx::query("update team set key = 'PLAT' where id = $1::uuid")
        .bind(&s.team)
        .execute(&pool)
        .await?;
    let identifier: String =
        sqlx::query_scalar("select identifier from issue where id = $1::uuid")
            .bind(&issue)
            .fetch_one(&pool)
            .await?;
    assert_eq!(identifier, "PLAT-7");

    // Der alte Key darf nicht an ein anderes Team vergeben werden.
    let err = insert_team(&pool, &s.org, "Neues Team", "ENG")
        .await
        .expect_err("frueherer Key ist reserviert");
    assert_sqlstate(&err, PG_UNIQUE_VIOLATION);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn symmetric_issue_relation_is_unique_per_unordered_pair(
    pool: PgPool,
) -> Result<(), sqlx::Error> {
    let s = seed(&pool).await?;
    let a = insert_issue(&pool, &s, 1, "A").await?;
    let b = insert_issue(&pool, &s, 2, "B").await?;
    let insert = |from: String, to: String, ty: &'static str| {
        let pool = pool.clone();
        let org = s.org.clone();
        async move {
            sqlx::query(
                "insert into issue_relation (organization_id, issue_id, related_issue_id, type) \
                 values ($1::uuid, $2::uuid, $3::uuid, $4)",
            )
            .bind(org)
            .bind(from)
            .bind(to)
            .bind(ty)
            .execute(&pool)
            .await
        }
    };
    insert(a.clone(), b.clone(), "related").await?;
    let err = insert(b.clone(), a.clone(), "related")
        .await
        .expect_err("related ist symmetrisch");
    assert_sqlstate(&err, PG_UNIQUE_VIOLATION);

    insert(a.clone(), b.clone(), "blocks").await?;
    let err = insert(b.clone(), a.clone(), "blocks")
        .await
        .expect_err("blocks nicht in beide Richtungen");
    assert_sqlstate(&err, PG_UNIQUE_VIOLATION);

    let err = insert(a.clone(), a.clone(), "related")
        .await
        .expect_err("keine Selbstrelation");
    assert_sqlstate(&err, PG_CHECK_VIOLATION);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn comment_requires_exactly_one_parent_and_history_is_append_only(
    pool: PgPool,
) -> Result<(), sqlx::Error> {
    let s = seed(&pool).await?;
    let issue = insert_issue(&pool, &s, 1, "Kommentare").await?;

    let err = sqlx::query("insert into comment (organization_id, body) values ($1::uuid, 'Ohne Eltern')")
        .bind(&s.org)
        .execute(&pool)
        .await
        .expect_err("Kommentar ohne Elternobjekt muss scheitern");
    assert_sqlstate(&err, PG_CHECK_VIOLATION);

    sqlx::query(
        "insert into issue_history (organization_id, issue_id, source, changes) \
         values ($1::uuid, $2::uuid, 'user', '{\"priority\":{\"from\":0,\"to\":2}}'::jsonb)",
    )
    .bind(&s.org)
    .bind(&issue)
    .execute(&pool)
    .await?;
    let err = sqlx::query("update issue_history set source = 'api' where issue_id = $1::uuid")
        .bind(&issue)
        .execute(&pool)
        .await
        .expect_err("IssueHistory ist append-only");
    assert_sqlstate(&err, PG_CHECK_VIOLATION);

    // Leerer Eintrag (keine Aenderung) ist ungueltig.
    let err = sqlx::query(
        "insert into issue_history (organization_id, issue_id, source) values ($1::uuid, $2::uuid, 'user')",
    )
    .bind(&s.org)
    .bind(&issue)
    .execute(&pool)
    .await
    .expect_err("History-Eintrag ohne Aenderung muss scheitern");
    assert_sqlstate(&err, PG_CHECK_VIOLATION);
    Ok(())
}
