//! Tool-Handler der Gruppe `documents` (WP10): `list_documents`, `get_document`, `save_document`
//! (docs/07 §3.15) und `search_documentation` (§3.18, eingebetteter Index über `docs/*.md`).

use super::comments::{arg_str, required_str};
use super::projects::parse_patch;
use super::teams_users::{parse_time_filter, ts};
use crate::envelope::{
    decode_cursor, encode_cursor, opt_text, page, parse_limit, query_fingerprint, team_ref,
    user_ref,
};
use crate::fields::{parse_fields, select_fields, truncate_content};
use chrono::{DateTime, SecondsFormat, Utc};
use lineal_app::ports::repos::user::{OrderBy, PageCursor};
use lineal_app::usecases::document::{
    DocumentView, ListDocumentsInput, ParentInfo, ParentRefs, SaveDocumentInput,
};
use lineal_app::{App, AppError, AppResult, RequestCtx};
use serde_json::{Map, Value, json};
use std::sync::OnceLock;

fn bad(path: &str, msg: impl Into<String>) -> AppError {
    AppError::invalid_request(msg).with_details(json!({ "path": format!("/{path}") }))
}

/// Dokument-Objekt nach docs/07 §3.15; genau eines der Parent-Felder ist gesetzt.
fn doc_json(v: &DocumentView, truncate: bool) -> Map<String, Value> {
    let d = &v.doc;
    let (mut project, mut initiative, mut team, mut issue, mut cycle) = (
        Value::Null,
        Value::Null,
        Value::Null,
        Value::Null,
        Value::Null,
    );
    match &v.parent {
        Some(ParentInfo::Project {
            id,
            identifier,
            name,
        }) => {
            project = json!({ "id": id.to_string(), "identifier": identifier, "name": name });
        }
        Some(ParentInfo::Initiative {
            id,
            identifier,
            name,
        }) => {
            initiative = json!({ "id": id.to_string(), "identifier": identifier, "name": name });
        }
        Some(ParentInfo::Team { id, key, name }) => team = team_ref(id, key, name),
        Some(ParentInfo::Issue {
            id,
            identifier,
            title,
        }) => {
            issue = json!({ "id": id.to_string(), "identifier": identifier, "title": title });
        }
        Some(ParentInfo::Cycle {
            id,
            number,
            team_id,
            team_key,
            team_name,
        }) => {
            cycle = json!({
                "id": id.to_string(), "number": number,
                "team": team_ref(team_id, team_key, team_name),
            });
        }
        None => {}
    }
    let content = if truncate {
        truncate_content(&d.content, "get_document")
    } else {
        d.content.clone()
    };
    let mut m = Map::new();
    m.insert("id".into(), json!(d.id.to_string()));
    m.insert("slugId".into(), json!(d.slug_id));
    m.insert("title".into(), json!(d.title));
    m.insert("content".into(), json!(content));
    m.insert("icon".into(), opt_text(d.icon.as_deref()));
    m.insert("color".into(), opt_text(d.color.as_deref()));
    m.insert("url".into(), json!(v.url));
    m.insert("createdAt".into(), json!(ts(d.created_at)));
    m.insert("updatedAt".into(), json!(ts(d.updated_at)));
    m.insert("archivedAt".into(), json!(d.archived_at.map(ts)));
    m.insert(
        "creator".into(),
        v.creator
            .as_ref()
            .map_or(Value::Null, |(id, name)| user_ref(id, name)),
    );
    m.insert(
        "updatedBy".into(),
        v.updated_by
            .as_ref()
            .map_or(Value::Null, |(id, name)| user_ref(id, name)),
    );
    m.insert("project".into(), project);
    m.insert("initiative".into(), initiative);
    m.insert("team".into(), team);
    m.insert("issue".into(), issue);
    m.insert("cycle".into(), cycle);
    m
}

fn list_input(args: &Value, now: DateTime<Utc>) -> AppResult<(ListDocumentsInput, String)> {
    let limit = parse_limit(args, 50, 250)?;
    let fingerprint = query_fingerprint(args);
    let order_by = match arg_str(args, "orderBy")? {
        Some("createdAt") => OrderBy::CreatedAt,
        _ => OrderBy::UpdatedAt,
    };
    let cursor = match arg_str(args, "cursor")? {
        Some(c) => {
            let pos = decode_cursor(c, &fingerprint)?;
            let bad_cursor = || AppError::invalid_request("Invalid cursor");
            let key = pos
                .key
                .as_str()
                .and_then(|k| DateTime::parse_from_rfc3339(k).ok())
                .ok_or_else(bad_cursor)?
                .with_timezone(&Utc);
            let id = pos.id.parse().map_err(|_| bad_cursor())?;
            Some(PageCursor { key, id })
        }
        None => None,
    };
    let time = |key: &str| -> AppResult<Option<DateTime<Utc>>> {
        arg_str(args, key)?
            .map(|s| parse_time_filter(key, s, now))
            .transpose()
    };
    let own = |k: &str| -> AppResult<Option<String>> { Ok(arg_str(args, k)?.map(str::to_owned)) };
    Ok((
        ListDocumentsInput {
            query: own("query")?,
            creator: own("creatorId")?,
            project: own("projectId")?,
            initiative: own("initiativeId")?,
            team: own("teamId")?,
            include_archived: args
                .get("includeArchived")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            created_after: time("createdAt")?,
            updated_after: time("updatedAt")?,
            order_by,
            limit: limit as i64,
            cursor,
            ..Default::default()
        },
        fingerprint,
    ))
}

fn save_input(args: &Value) -> AppResult<SaveDocumentInput> {
    let own = |k: &str| -> AppResult<Option<String>> { Ok(arg_str(args, k)?.map(str::to_owned)) };
    if args.get("content").is_some_and(|v| !v.is_null()) && args.get("patch").is_some() {
        return Err(bad("patch", "`patch` cannot be combined with `content`"));
    }
    Ok(SaveDocumentInput {
        id: own("id")?,
        title: own("title")?,
        content: own("content")?,
        patch: parse_patch(args)?,
        icon: own("icon")?,
        color: own("color")?,
        parent: ParentRefs {
            project: own("project")?,
            issue: own("issue")?,
            initiative: own("initiative")?,
            cycle: own("cycle")?,
            team: own("team")?,
        },
        ..Default::default()
    })
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    match tool {
        "list_documents" => {
            let (input, fingerprint) = list_input(&args, app.clock.now())?;
            let order_by = input.order_by;
            let res = app.list_documents(ctx, input).await?;
            let requested = parse_fields(&args);
            let items: Vec<Value> = res
                .items
                .iter()
                .map(|v| select_fields(&doc_json(v, true), &requested, &[]))
                .collect();
            let next = res.next.map(|c| {
                let _ = order_by;
                encode_cursor(
                    &json!(c.key.to_rfc3339_opts(SecondsFormat::Micros, true)),
                    c.id,
                    &fingerprint,
                )
            });
            Ok(page("documents", items, next))
        }
        "get_document" => {
            let v = app.get_document(ctx, required_str(&args, "id")?).await?;
            Ok(Value::Object(doc_json(&v, false)))
        }
        "save_document" => {
            let v = app.save_document(ctx, save_input(&args)?).await?;
            Ok(Value::Object(doc_json(&v, false)))
        }
        "search_documentation" => search_documentation(app, &args),
        other => Err(AppError::internal(format!(
            "tool `{other}` does not belong to the documents group"
        ))),
    }
}

// ---------------------------------------------------------------------------------------------
// search_documentation: statischer Index über die eingebetteten Lineal-Dokumente
// ---------------------------------------------------------------------------------------------

/// Die Lineal-Dokumentation (`docs/*.md`), zur Build-Zeit eingebettet.
const DOCS: [(&str, &str); 16] = [
    ("README", include_str!("../../../../docs/README.md")),
    (
        "00-konventionen",
        include_str!("../../../../docs/00-konventionen.md"),
    ),
    (
        "01-domaenenmodell",
        include_str!("../../../../docs/01-domaenenmodell.md"),
    ),
    (
        "02-teams-rechte",
        include_str!("../../../../docs/02-teams-rechte.md"),
    ),
    (
        "03-issues-workflow",
        include_str!("../../../../docs/03-issues-workflow.md"),
    ),
    ("04-planung", include_str!("../../../../docs/04-planung.md")),
    (
        "05-views-suche-inbox",
        include_str!("../../../../docs/05-views-suche-inbox.md"),
    ),
    (
        "06-dokumente-templates",
        include_str!("../../../../docs/06-dokumente-templates.md"),
    ),
    (
        "07-mcp-server",
        include_str!("../../../../docs/07-mcp-server.md"),
    ),
    (
        "08-userflows",
        include_str!("../../../../docs/08-userflows.md"),
    ),
    (
        "09-api-events",
        include_str!("../../../../docs/09-api-events.md"),
    ),
    (
        "10-architektur-umsetzung",
        include_str!("../../../../docs/10-architektur-umsetzung.md"),
    ),
    (
        "11-implementierungsarchitektur",
        include_str!("../../../../docs/11-implementierungsarchitektur.md"),
    ),
    (
        "12-arbeitspakete",
        include_str!("../../../../docs/12-arbeitspakete.md"),
    ),
    (
        "13-import-linear",
        include_str!("../../../../docs/13-import-linear.md"),
    ),
    (
        "14-firmenweite-sicht",
        include_str!("../../../../docs/14-firmenweite-sicht.md"),
    ),
];

/// Seitengröße von `search_documentation`.
const DOC_PAGE_SIZE: usize = 5;
/// Länge des Auszugs in Zeichen.
const EXCERPT_CHARS: usize = 240;

struct Section {
    title: String,
    slug: String,
    anchor: String,
    /// Klein geschriebener Volltext des Abschnitts (Titel + Text) für die Suche.
    haystack: String,
    text: String,
}

fn anchor_of(heading: &str) -> String {
    let mut out = String::new();
    for c in heading.to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

fn build_index() -> Vec<Section> {
    let mut sections = Vec::new();
    for (slug, body) in DOCS {
        let mut doc_title = slug.to_owned();
        let mut current: Option<(String, String)> = None; // (Überschrift, Text)
        let mut in_fence = false;
        let mut flush = |cur: &mut Option<(String, String)>, doc_title: &str| {
            if let Some((heading, text)) = cur.take() {
                let title = if heading.is_empty() || heading == doc_title {
                    doc_title.to_owned()
                } else {
                    format!("{doc_title} – {heading}")
                };
                let haystack = format!("{title}\n{text}").to_lowercase();
                sections.push(Section {
                    title,
                    slug: slug.to_owned(),
                    anchor: anchor_of(&heading),
                    haystack,
                    text,
                });
            }
        };
        for line in body.lines() {
            if line.trim_start().starts_with("```") {
                in_fence = !in_fence;
            }
            let heading = (!in_fence)
                .then(|| line.trim_start_matches('#'))
                .filter(|_| line.starts_with('#'))
                .filter(|rest| rest.starts_with(' '))
                .map(|rest| rest.trim().to_owned());
            match heading {
                Some(h) => {
                    if line.starts_with("# ") && doc_title == slug {
                        doc_title = h.clone();
                    }
                    flush(&mut current, &doc_title);
                    current = Some((h, String::new()));
                }
                None => {
                    let entry = current.get_or_insert_with(|| (String::new(), String::new()));
                    entry.1.push_str(line);
                    entry.1.push('\n');
                }
            }
        }
        flush(&mut current, &doc_title);
    }
    sections
}

fn index() -> &'static [Section] {
    static INDEX: OnceLock<Vec<Section>> = OnceLock::new();
    INDEX.get_or_init(build_index)
}

/// Auszug um den ersten Treffer, auf Zeichengrenzen und ohne Zeilenumbrüche.
fn excerpt(text: &str, terms: &[String]) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = flat.to_lowercase();
    let hit = terms
        .iter()
        .filter_map(|t| lower.find(t.as_str()))
        .min()
        .unwrap_or(0);
    // Byte-Positionen von `lower` und `flat` können bei Sonderfällen (ß, İ) abweichen.
    let hit_chars = lower
        .get(..hit)
        .map_or(0, |p| p.chars().count())
        .min(flat.chars().count());
    let start = hit_chars.saturating_sub(40);
    let piece: String = flat.chars().skip(start).take(EXCERPT_CHARS).collect();
    let mut out = String::new();
    if start > 0 {
        out.push_str("… ");
    }
    out.push_str(piece.trim());
    if flat.chars().count() > start + EXCERPT_CHARS {
        out.push_str(" …");
    }
    out
}

fn search_documentation(app: &App, args: &Value) -> AppResult<Value> {
    let query = required_str(args, "query")?;
    let page_no = match args.get("page") {
        None | Some(Value::Null) => 0usize,
        Some(v) => {
            let n = v.as_f64().unwrap_or(-1.0);
            if n < 0.0 || n.fract() != 0.0 || n > 10_000.0 {
                return Err(bad("page", "`page` must be a non-negative integer"));
            }
            n as usize
        }
    };
    let terms: Vec<String> = query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|t| !t.is_empty())
        .take(10)
        .collect();
    if terms.is_empty() {
        return Err(bad("query", "`query` must not be empty"));
    }
    let base = app.config.base_url.trim_end_matches('/');
    let mut scored: Vec<(usize, usize, &Section)> = index()
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let mut score = 0usize;
            for t in &terms {
                let hits = s.haystack.matches(t.as_str()).count();
                if hits == 0 {
                    return None; // alle Suchbegriffe müssen vorkommen
                }
                let in_title = s.title.to_lowercase().contains(t.as_str());
                score += hits.min(20) + if in_title { 10 } else { 0 };
            }
            Some((score, i, s))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let total = scored.len();
    let start = page_no.saturating_mul(DOC_PAGE_SIZE);
    let results: Vec<Value> = scored
        .iter()
        .skip(start)
        .take(DOC_PAGE_SIZE)
        .map(|(_, _, s)| {
            let url = if s.anchor.is_empty() {
                format!("{base}/docs/{}", s.slug)
            } else {
                format!("{base}/docs/{}#{}", s.slug, s.anchor)
            };
            json!({ "title": s.title, "url": url, "excerpt": excerpt(&s.text, &terms) })
        })
        .collect();
    Ok(json!({
        "results": results,
        "page": page_no,
        "hasNextPage": start.saturating_add(DOC_PAGE_SIZE) < total,
    }))
}

#[cfg(test)]
#[path = "documents_tests.rs"]
mod tests;
