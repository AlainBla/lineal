//! Volltextsuche (WP11, docs/05 §5, docs/10 §5).
//!
//! Die Suche läuft über generierte `tsvector`-Spalten (Konfiguration je Workspace
//! `organization.fulltext_config`, Migration 3100; Trigger pflegen sie); es gibt keinen separaten Index, den der Outbox-Konsument pflegen müsste, und die
//! Berechtigung wird zur Abfragezeit als UND angehängt (docs/10 §5.3).

use lineal_app::ports::repos::view::{SearchHitRow, SearchKind, SearchOrder, SearchQuery};
use lineal_app::{AppError, AppResult};
use lineal_domain::identifier::{ParsedIdentifier, parse_identifier};
use sqlx::{PgConnection, Postgres, QueryBuilder, Row};
use uuid::Uuid;

/// Englische Stoppwörter außerhalb von Anführungszeichen (docs/05 §5.3 Punkt 2).
const STOPWORDS: [&str; 33] = [
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "if", "in", "into", "is", "it",
    "no", "not", "of", "on", "or", "such", "that", "the", "their", "then", "there", "these",
    "they", "this", "to", "was", "will", "with",
];

/// Maximale Zahl von Suchbegriffen und Länge eines Begriffs (Schutz vor Riesenabfragen).
const MAX_TERMS: usize = 24;
const MAX_TERM_LEN: usize = 64;

fn tokens(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.chars()
                .take(MAX_TERM_LEN)
                .collect::<String>()
                .to_lowercase()
        })
}

/// Baut aus dem Suchtext einen `to_tsquery('simple', …)`-String (docs/05 §5.3):
/// unquotierte Begriffe werden mit UND verknüpft (Präfixsuche, damit „fail“ „failing“ findet),
/// Stoppwörter entfallen, `"Phrasen"` werden exakt als Wortfolge gesucht. Es gelangen nur
/// alphanumerische Zeichen in den Ausdruck – Syntaxzeichen des Benutzers können ihn nicht
/// verändern. `None`: leer oder nur Stoppwörter ⇒ keine Volltextsuche.
pub fn tsquery(text: &str) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for (i, segment) in text.split('"').enumerate() {
        let quoted = i % 2 == 1;
        if quoted {
            let words: Vec<String> = tokens(segment).collect();
            if !words.is_empty() {
                let phrase = words
                    .iter()
                    .map(|w| format!("'{w}'"))
                    .collect::<Vec<_>>()
                    .join(" <-> ");
                parts.push(format!("({phrase})"));
            }
        } else {
            for w in tokens(segment) {
                if !STOPWORDS.contains(&w.as_str()) {
                    parts.push(format!("'{w}':*"));
                }
            }
        }
        if parts.len() >= MAX_TERMS {
            parts.truncate(MAX_TERMS);
            break;
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" & "))
    }
}

/// Erkennt eine Kennung im Suchtext (`ENG-124`, `eng-124`, `eng124`, `P-ENG-7`, `I-4`;
/// docs/05 §5.3 Punkt 4). Leerzeichen im Text → keine Kennung.
pub fn search_identifier(text: &str) -> Option<ParsedIdentifier> {
    let t = text.trim();
    if t.is_empty() || t.chars().any(char::is_whitespace) {
        return None;
    }
    if let Some(p) = parse_identifier(t) {
        return Some(p);
    }
    // Kurzform ohne Bindestrich: Buchstaben/Ziffern, dann eine Zifferngruppe am Ende (`eng124`).
    let digits = t.chars().rev().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits == t.chars().count() {
        return None;
    }
    let split = t.len() - digits;
    if !t.is_char_boundary(split) {
        return None;
    }
    let (key, num) = t.split_at(split);
    let key = key.trim_end_matches('-');
    if !key.chars().last().is_some_and(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    parse_identifier(&format!("{key}-{num}"))
}

/// Hängt die Suchbedingung für `list_issues.query` an (` AND …`): eine Kennung trifft genau
/// dieses Issue – auch über frühere Kennungen (Alias-Tabelle und frühere Team-Keys); sonst
/// Volltext über Titel und Beschreibung (`search_vector`). Leere Anfrage ⇒ keine Treffer.
/// Die Berechtigung (`team_id IN sichtbar`) steht **außerhalb** und gilt damit auch für alte
/// Kennungen.
pub fn push_issue_search<'q>(qb: &mut QueryBuilder<'q, Postgres>, alias: &str, text: &str) {
    if let Some(ident) = search_identifier(text) {
        match ident {
            ParsedIdentifier::Issue(i) => {
                qb.push(" AND ");
                push_issue_exact(qb, alias, &i);
            }
            // Projekt-/Initiativ-Kennungen sind keine Issue-Kennungen.
            _ => {
                qb.push(" AND FALSE");
            }
        }
        return;
    }
    match tsquery(text) {
        Some(q) => {
            qb.push(format!(
                " AND {alias}.search_vector @@ to_tsquery(lineal_ts_config({alias}.organization_id), "
            ));
            qb.push_bind(q);
            qb.push(")");
        }
        None => {
            qb.push(" AND FALSE");
        }
    }
}

/// Boolescher Ausdruck „Issue `alias` hat diese Kennung“: aktuelle Kennung, Alias-Tabelle oder
/// früherer Team-Key mit der Nummer des Issues (docs/05 §5.2). Die Sichtbarkeit steht außerhalb.
fn push_issue_exact<'q>(
    qb: &mut QueryBuilder<'q, Postgres>,
    alias: &str,
    i: &lineal_domain::identifier::IssueIdentifier,
) {
    let full = i.to_string();
    qb.push(format!("({alias}.identifier = "));
    qb.push_bind(full.clone());
    qb.push(format!(
        " OR {alias}.id IN (SELECT a.issue_id FROM issue_identifier_alias a \
         WHERE a.organization_id = {alias}.organization_id AND a.entity_type = 'issue' \
         AND a.archived_at IS NULL AND a.identifier = "
    ));
    qb.push_bind(full);
    qb.push(format!(") OR ({alias}.number = "));
    qb.push_bind(i.number);
    qb.push(format!(
        " AND EXISTS (SELECT 1 FROM team_key tk WHERE tk.team_id = {alias}.team_id \
         AND tk.organization_id = {alias}.organization_id AND tk.key = "
    ));
    qb.push_bind(i.team_key.as_str().to_owned());
    qb.push(")))");
}

/// Dokument `alias` ist sichtbar, wenn das Elternobjekt sichtbar ist (docs/05 §5.7).
fn push_document_visible<'q>(
    qb: &mut QueryBuilder<'q, Postgres>,
    alias: &str,
    vis: &[Uuid],
    guest: bool,
) {
    qb.push(format!("(NOT {alias}.trashed AND ({alias}.team_id = ANY("));
    qb.push_bind(vis.to_vec());
    qb.push(format!(
        ") OR EXISTS (SELECT 1 FROM issue di WHERE di.id = {alias}.issue_id AND NOT di.trashed AND di.team_id = ANY("
    ));
    qb.push_bind(vis.to_vec());
    qb.push(format!(
        ")) OR EXISTS (SELECT 1 FROM project_team dpt WHERE dpt.project_id = {alias}.project_id AND dpt.team_id = ANY("
    ));
    qb.push_bind(vis.to_vec());
    qb.push("))");
    if !guest {
        qb.push(format!(
            " OR EXISTS (SELECT 1 FROM initiative dn WHERE dn.id = {alias}.initiative_id \
             AND (dn.lead_team_id IS NULL OR dn.lead_team_id = ANY("
        ));
        qb.push_bind(vis.to_vec());
        qb.push("))) ");
    }
    qb.push("))");
}

const MARK_OPEN: char = '\u{1}';
const MARK_CLOSE: char = '\u{2}';
const HEADLINE_OPTS: &str =
    "StartSel=\u{1}, StopSel=\u{2}, MaxFragments=1, MaxWords=18, MinWords=6";

/// Escaped den Ausschnitt und setzt die Markierungen (nur unsere Marker werden zu `<mark>`).
fn render_snippet(raw: &str) -> Option<String> {
    if !raw.contains(MARK_OPEN) {
        return None;
    }
    let mut out = String::with_capacity(raw.len() + 16);
    for c in raw.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            MARK_OPEN => out.push_str("<mark>"),
            MARK_CLOSE => out.push_str("</mark>"),
            c => out.push(c),
        }
    }
    Some(out)
}

fn status_boost_sql(col: &str, kind: SearchKind) -> String {
    match kind {
        SearchKind::Issue => format!(
            "(CASE {col} WHEN 'unstarted' THEN 4 WHEN 'started' THEN 4 WHEN 'backlog' THEN 3 \
             WHEN 'triage' THEN 3 WHEN 'completed' THEN 2 ELSE 1 END)"
        ),
        SearchKind::Project => format!(
            "(CASE {col} WHEN 'started' THEN 4 WHEN 'completed' THEN 2 WHEN 'canceled' THEN 1 ELSE 3 END)"
        ),
        SearchKind::Initiative => format!(
            "(CASE {col} WHEN 'Active' THEN 4 WHEN 'Completed' THEN 2 WHEN 'Canceled' THEN 1 ELSE 3 END)"
        ),
        SearchKind::Document => "3".to_owned(),
    }
}

fn begin_query<'q>(
    qb: &mut QueryBuilder<'q, Postgres>,
    org: Uuid,
    q: &SearchQuery,
    tsq: &Option<String>,
) {
    match &q.eval {
        Some(ev) => {
            crate::filter_sql::push_cte_prefix(qb, ev);
            qb.push(", ");
        }
        None => {
            qb.push("WITH ");
        }
    }
    qb.push("sq AS (SELECT to_tsquery(lineal_ts_config(");
    qb.push_bind(org);
    qb.push("), ");
    qb.push_bind(tsq.clone());
    qb.push("::text) AS q) ");
}

fn finish_query<'q>(qb: &mut QueryBuilder<'q, Postgres>, q: &SearchQuery) {
    qb.push(") s ORDER BY ");
    qb.push(match q.order {
        SearchOrder::Relevance => {
            "s.exact DESC, (s.rank * CASE WHEN s.archived THEN 0.5 ELSE 1 END) DESC, \
             s.boost DESC, s.updated_at DESC, s.id DESC"
        }
        SearchOrder::UpdatedAt => "s.updated_at DESC, s.id DESC",
        SearchOrder::CreatedAt => "s.created_at DESC, s.id DESC",
    });
    qb.push(" LIMIT ");
    qb.push_bind(q.limit.clamp(1, 500));
}

fn hit_from_row(kind: SearchKind, r: &sqlx::postgres::PgRow) -> AppResult<SearchHitRow> {
    let e = |_| AppError::internal("search row");
    Ok(SearchHitRow {
        kind,
        id: r.try_get("id").map_err(e)?,
        identifier: r.try_get("identifier").map_err(e)?,
        title: r.try_get("title").map_err(e)?,
        team_key: r.try_get("team_key").map_err(e)?,
        status: r.try_get("status").map_err(e)?,
        created_at: r.try_get("created_at").map_err(e)?,
        updated_at: r.try_get("updated_at").map_err(e)?,
        archived: r.try_get("archived").map_err(e)?,
        exact: r.try_get("exact").map_err(e)?,
        rank: r.try_get("rank").map_err(e)?,
        boost: r.try_get("boost").map_err(e)?,
        highlights: Vec::new(),
    })
}

async fn run(
    conn: &mut PgConnection,
    mut qb: QueryBuilder<'_, Postgres>,
    kind: SearchKind,
) -> AppResult<Vec<SearchHitRow>> {
    let rows = qb
        .build()
        .fetch_all(&mut *conn)
        .await
        .map_err(crate::db_err)?;
    rows.iter().map(|r| hit_from_row(kind, r)).collect()
}

/// Volltextsuche über alle gewünschten Arten (docs/05 §5). Jede Teilabfrage enthält die
/// Berechtigung; Trefferzahlen und Ausschnitte stammen nur aus sichtbaren Zeilen.
pub(crate) async fn search_all(
    conn: &mut PgConnection,
    org: Uuid,
    q: &SearchQuery,
) -> AppResult<Vec<SearchHitRow>> {
    let ident = search_identifier(&q.term);
    let tsq = tsquery(&q.term);
    if ident.is_none() && tsq.is_none() {
        return Ok(Vec::new());
    }
    let vis: Vec<Uuid> = q.access.visible_teams.iter().map(|t| t.0).collect();
    let scope: Option<Vec<Uuid>> = q
        .team_scope
        .as_ref()
        .map(|v| v.iter().map(|t| t.0).collect());
    let mut hits: Vec<SearchHitRow> = Vec::new();

    for kind in &q.kinds {
        let mut qb: QueryBuilder<'_, Postgres> = QueryBuilder::new("");
        begin_query(&mut qb, org, q, &tsq);
        match kind {
            SearchKind::Issue => {
                qb.push(
                    "SELECT * FROM (SELECT i.id, i.identifier, i.title, t.key AS team_key, \
                     i.state_type AS status, i.created_at, i.updated_at, \
                     (i.archived_at IS NOT NULL) AS archived, ",
                );
                match &ident {
                    Some(ParsedIdentifier::Issue(id)) => push_issue_exact(&mut qb, "i", id),
                    _ => {
                        qb.push("FALSE");
                    }
                }
                qb.push(" AS exact, COALESCE(ts_rank(i.search_vector, sq.q), 0)::float8 AS rank, ");
                qb.push(status_boost_sql("i.state_type", SearchKind::Issue));
                qb.push(
                    " AS boost FROM issue i JOIN team t ON t.id = i.team_id CROSS JOIN sq \
                     WHERE i.organization_id = ",
                );
                qb.push_bind(org);
                qb.push(" AND NOT i.trashed AND (i.team_id = ANY(");
                qb.push_bind(vis.clone());
                qb.push(
                    ") OR i.id IN (SELECT s.issue_id FROM issue_share s WHERE s.organization_id = i.organization_id AND s.user_id = ",
                );
                qb.push_bind(q.access.user_id.0);
                qb.push("))");
                if !q.include_archived {
                    qb.push(" AND i.archived_at IS NULL");
                }
                if let Some(sc) = &scope {
                    qb.push(" AND i.team_id = ANY(");
                    qb.push_bind(sc.clone());
                    qb.push(")");
                }
                if let Some(ast) = &q.ast {
                    let ev = q
                        .eval
                        .as_ref()
                        .ok_or_else(|| AppError::internal("filter without evaluation context"))?;
                    let cx = crate::filter_sql::Cx::new(ev);
                    qb.push(" AND ");
                    crate::filter_sql::push_issue_filter(&mut qb, &cx, "i", ast)?;
                }
                qb.push(" AND (");
                if let Some(ParsedIdentifier::Issue(id)) = &ident {
                    push_issue_exact(&mut qb, "i", id);
                    qb.push(" OR ");
                }
                qb.push("i.search_vector @@ sq.q");
                if q.include_comments {
                    qb.push(
                        " OR EXISTS (SELECT 1 FROM comment c WHERE c.issue_id = i.id \
                         AND c.organization_id = i.organization_id AND c.archived_at IS NULL \
                         AND c.search_vector @@ sq.q)",
                    );
                }
                qb.push(")");
            }
            SearchKind::Project => {
                qb.push(
                    "SELECT * FROM (SELECT p.id, 'P-' || lt.key || '-' || p.number AS identifier, \
                     p.name AS title, lt.key AS team_key, ps.type AS status, p.created_at, p.updated_at, \
                     (p.archived_at IS NOT NULL) AS archived, ",
                );
                match &ident {
                    Some(ParsedIdentifier::Project(pi)) => {
                        let full = pi.to_string();
                        qb.push("(lt.key = ");
                        qb.push_bind(pi.lead_team_key.as_str().to_owned());
                        qb.push(" AND p.number = ");
                        qb.push_bind(pi.number);
                        qb.push(
                            " OR EXISTS (SELECT 1 FROM issue_identifier_alias a WHERE a.organization_id = p.organization_id \
                             AND a.entity_type = 'project' AND a.project_id = p.id AND a.identifier = ",
                        );
                        qb.push_bind(full);
                        qb.push("))");
                    }
                    _ => {
                        qb.push("FALSE");
                    }
                }
                qb.push(" AS exact, COALESCE(ts_rank(p.search_vector, sq.q), 0)::float8 AS rank, ");
                qb.push(status_boost_sql("ps.type", SearchKind::Project));
                qb.push(
                    " AS boost FROM project p JOIN team lt ON lt.id = p.lead_team_id \
                     JOIN project_status ps ON ps.id = p.status_id CROSS JOIN sq WHERE p.organization_id = ",
                );
                qb.push_bind(org);
                qb.push(
                    " AND NOT p.trashed AND EXISTS (SELECT 1 FROM project_team pt WHERE pt.project_id = p.id AND pt.team_id = ANY(",
                );
                qb.push_bind(vis.clone());
                qb.push("))");
                if !q.include_archived {
                    qb.push(" AND p.archived_at IS NULL");
                }
                if let Some(sc) = &scope {
                    qb.push(
                        " AND EXISTS (SELECT 1 FROM project_team ps2 WHERE ps2.project_id = p.id AND ps2.team_id = ANY(",
                    );
                    qb.push_bind(sc.clone());
                    qb.push("))");
                }
                qb.push(" AND (");
                if let Some(ParsedIdentifier::Project(pi)) = &ident {
                    qb.push("(lt.key = ");
                    qb.push_bind(pi.lead_team_key.as_str().to_owned());
                    qb.push(" AND p.number = ");
                    qb.push_bind(pi.number);
                    qb.push(
                        ") OR EXISTS (SELECT 1 FROM issue_identifier_alias a WHERE a.organization_id = p.organization_id \
                         AND a.entity_type = 'project' AND a.project_id = p.id AND a.identifier = ",
                    );
                    qb.push_bind(pi.to_string());
                    qb.push(") OR ");
                }
                qb.push("p.search_vector @@ sq.q)");
            }
            SearchKind::Initiative => {
                if q.access.is_guest {
                    continue;
                }
                qb.push(
                    "SELECT * FROM (SELECT n.id, 'I-' || n.number AS identifier, n.name AS title, \
                     NULL::text AS team_key, n.status AS status, n.created_at, n.updated_at, \
                     (n.archived_at IS NOT NULL) AS archived, ",
                );
                match &ident {
                    Some(ParsedIdentifier::Initiative { number }) => {
                        qb.push("n.number = ");
                        qb.push_bind(*number);
                    }
                    _ => {
                        qb.push("FALSE");
                    }
                }
                qb.push(" AS exact, COALESCE(ts_rank(n.search_vector, sq.q), 0)::float8 AS rank, ");
                qb.push(status_boost_sql("n.status", SearchKind::Initiative));
                qb.push(" AS boost FROM initiative n CROSS JOIN sq WHERE n.organization_id = ");
                qb.push_bind(org);
                qb.push(" AND NOT n.trashed AND (n.lead_team_id IS NULL OR n.lead_team_id = ANY(");
                qb.push_bind(vis.clone());
                qb.push("))");
                if !q.include_archived {
                    qb.push(" AND n.archived_at IS NULL");
                }
                qb.push(" AND (");
                if let Some(ParsedIdentifier::Initiative { number }) = &ident {
                    qb.push("n.number = ");
                    qb.push_bind(*number);
                    qb.push(" OR ");
                }
                qb.push("n.search_vector @@ sq.q)");
            }
            SearchKind::Document => {
                qb.push(
                    "SELECT * FROM (SELECT d.id, NULL::text AS identifier, d.title AS title, \
                     NULL::text AS team_key, NULL::text AS status, d.created_at, d.updated_at, \
                     (d.archived_at IS NOT NULL) AS archived, FALSE AS exact, \
                     COALESCE(ts_rank(d.search_vector, sq.q), 0)::float8 AS rank, 3 AS boost \
                     FROM document d CROSS JOIN sq WHERE d.organization_id = ",
                );
                qb.push_bind(org);
                qb.push(" AND ");
                push_document_visible(&mut qb, "d", &vis, q.access.is_guest);
                if !q.include_archived {
                    qb.push(" AND d.archived_at IS NULL");
                }
                qb.push(" AND d.search_vector @@ sq.q");
            }
        }
        finish_query(&mut qb, q);
        hits.extend(run(conn, qb, *kind).await?);
    }

    // Mischen („All“): exakte Kennung, Relevanz (archiviert mit Abschlag), Status-Boost, Aktualität.
    match q.order {
        SearchOrder::Relevance => hits.sort_by(|a, b| {
            let ra = a.rank * if a.archived { 0.5 } else { 1.0 };
            let rb = b.rank * if b.archived { 0.5 } else { 1.0 };
            b.exact
                .cmp(&a.exact)
                .then(rb.partial_cmp(&ra).unwrap_or(std::cmp::Ordering::Equal))
                .then(b.boost.cmp(&a.boost))
                .then(b.updated_at.cmp(&a.updated_at))
                .then(b.id.cmp(&a.id))
        }),
        SearchOrder::UpdatedAt => {
            hits.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(b.id.cmp(&a.id)));
        }
        SearchOrder::CreatedAt => {
            hits.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        }
    }
    hits.truncate(q.limit.clamp(1, 500) as usize);
    add_highlights(conn, org, q, &tsq, &mut hits).await?;
    Ok(hits)
}

/// Trefferausschnitte für die endgültige Trefferliste (nur Zeilen, die die Sichtbarkeit
/// bereits bestanden haben).
async fn add_highlights(
    conn: &mut PgConnection,
    org: Uuid,
    q: &SearchQuery,
    tsq: &Option<String>,
    hits: &mut [SearchHitRow],
) -> AppResult<()> {
    let Some(tsq) = tsq else {
        return Ok(());
    };
    type Fields<'a> = &'a [(&'a str, &'a str)];
    let specs: [(SearchKind, &str, Fields<'_>); 4] = [
        (
            SearchKind::Issue,
            "issue",
            &[
                ("title", "title"),
                ("description", "left(description, 20000)"),
            ],
        ),
        (
            SearchKind::Project,
            "project",
            &[
                ("name", "name"),
                ("description", "description"),
                ("content", "left(content, 20000)"),
            ],
        ),
        (
            SearchKind::Initiative,
            "initiative",
            &[
                ("name", "name"),
                ("description", "description"),
                ("content", "left(content, 20000)"),
            ],
        ),
        (
            SearchKind::Document,
            "document",
            &[("title", "title"), ("content", "left(content, 20000)")],
        ),
    ];
    for (kind, table, fields) in specs {
        let ids: Vec<Uuid> = hits
            .iter()
            .filter(|h| h.kind == kind)
            .map(|h| h.id)
            .collect();
        if ids.is_empty() {
            continue;
        }
        for (label, expr) in fields {
            let sql = format!(
                "SELECT id, ts_headline(lineal_ts_config($1), coalesce({expr}, ''), to_tsquery(lineal_ts_config($1), $3), $4) \
                 FROM {table} WHERE organization_id = $1 AND id = ANY($2)"
            );
            let rows: Vec<(Uuid, String)> = sqlx::query_as(&sql)
                .bind(org)
                .bind(&ids)
                .bind(tsq)
                .bind(HEADLINE_OPTS)
                .fetch_all(&mut *conn)
                .await
                .map_err(crate::db_err)?;
            for (id, raw) in rows {
                if let Some(snippet) = render_snippet(&raw)
                    && let Some(h) = hits.iter_mut().find(|h| h.id == id)
                {
                    h.highlights.push(((*label).to_owned(), snippet));
                }
            }
        }
    }
    if q.include_comments {
        let ids: Vec<Uuid> = hits
            .iter()
            .filter(|h| h.kind == SearchKind::Issue)
            .map(|h| h.id)
            .collect();
        if !ids.is_empty() {
            let rows: Vec<(Uuid, String)> = sqlx::query_as(
                "SELECT DISTINCT ON (c.issue_id) c.issue_id, \
                 ts_headline(lineal_ts_config($1), left(c.body, 20000), to_tsquery(lineal_ts_config($1), $3), $4) \
                 FROM comment c WHERE c.organization_id = $1 AND c.issue_id = ANY($2) \
                 AND c.archived_at IS NULL AND c.search_vector @@ to_tsquery(lineal_ts_config($1), $3) \
                 ORDER BY c.issue_id, c.created_at",
            )
            .bind(org)
            .bind(&ids)
            .bind(tsq)
            .bind(HEADLINE_OPTS)
            .fetch_all(&mut *conn)
            .await
            .map_err(crate::db_err)?;
            for (id, raw) in rows {
                if let Some(snippet) = render_snippet(&raw)
                    && let Some(h) = hits.iter_mut().find(|h| h.id == id)
                {
                    h.highlights.push(("comment".to_owned(), snippet));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_and_of_prefix_terms_without_stopwords() {
        assert_eq!(
            tsquery("the login fails").as_deref(),
            Some("'login':* & 'fails':*")
        );
    }

    #[test]
    fn phrases_keep_stopwords_and_order() {
        assert_eq!(
            tsquery(r#""not in stock" retry"#).as_deref(),
            Some("('not' <-> 'in' <-> 'stock') & 'retry':*")
        );
    }

    #[test]
    fn empty_and_stopword_only_queries_yield_none() {
        assert_eq!(tsquery("  "), None);
        assert_eq!(tsquery("the of and"), None);
        assert_eq!(tsquery("'; DROP"), Some("'drop':*".into()));
    }

    #[test]
    fn identifiers_in_all_spellings() {
        for t in ["ENG-124", "eng-124", "eng124", " Eng124 "] {
            assert_eq!(
                search_identifier(t).map(|p| p.to_string()).as_deref(),
                Some("ENG-124"),
                "{t}"
            );
        }
        assert_eq!(
            search_identifier("p-eng-7")
                .map(|p| p.to_string())
                .as_deref(),
            Some("P-ENG-7")
        );
        assert_eq!(
            search_identifier("i-4").map(|p| p.to_string()).as_deref(),
            Some("I-4")
        );
        assert!(search_identifier("login fails").is_none());
        assert!(search_identifier("124").is_none());
        assert!(search_identifier("fix-login").is_none());
    }

    #[test]
    fn syntax_characters_are_stripped() {
        assert_eq!(tsquery("a:* | !b & (c)").as_deref(), Some("'b':* & 'c':*"));
    }
}
