//! Tool-Handler der Gruppe `teams_users` (WP02): `get_workspace`, `list_teams`, `get_team`,
//! `list_users`, `get_user` (docs/07 §3.1). Die Eingabe wurde bereits gegen das Snapshot-Schema
//! validiert; die Handler prüfen defensiv trotzdem.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, Utc};
use lineal_app::ports::repos::user::{OrderBy, Page, PageCursor, PageRequest, User};
use lineal_app::usecases::team::{ListTeamsInput, TeamView};
use lineal_app::usecases::user::ListUsersInput;
use lineal_app::{App, AppError, AppResult, RequestCtx};
use lineal_domain::ids::UserId;
use serde_json::{Map, Value, json};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

/// ISO-8601 UTC mit Millisekunden.
pub(crate) fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn arg_str<'a>(args: &'a Value, key: &str) -> AppResult<Option<&'a str>> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(
            AppError::invalid_request(format!("`{key}` must be a string"))
                .with_details(json!({ "path": format!("/{key}") })),
        ),
    }
}

fn required_str<'a>(args: &'a Value, key: &str) -> AppResult<&'a str> {
    arg_str(args, key)?
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            AppError::invalid_request(format!("`{key}` is required"))
                .with_details(json!({ "path": format!("/{key}") }))
        })
}

fn arg_bool(args: &Value, key: &str) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// `DateTimeOrDuration`: ISO-Zeitpunkt, Datum oder Dauer (`-P1D`, `-P2W`, `PT1H`).
pub(crate) fn parse_time_filter(
    field: &str,
    s: &str,
    now: DateTime<Utc>,
) -> AppResult<DateTime<Utc>> {
    let bad = || {
        AppError::invalid_request(format!(
            "`{field}` must be an ISO-8601 date, timestamp or duration"
        ))
        .with_details(json!({ "path": format!("/{field}") }))
    };
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Ok(t.with_timezone(&Utc));
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return d.and_hms_opt(0, 0, 0).map(|n| n.and_utc()).ok_or_else(bad);
    }
    let (negative, rest) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let rest = rest.strip_prefix('P').ok_or_else(bad)?;
    let (date_part, time_part) = rest.split_once('T').unwrap_or((rest, ""));
    let mut total = Duration::zero();
    let mut any = false;
    for (part, in_time) in [(date_part, false), (time_part, true)] {
        let mut num = String::new();
        for c in part.chars() {
            if c.is_ascii_digit() {
                num.push(c);
                continue;
            }
            let n: i64 = num.parse().map_err(|_| bad())?;
            num.clear();
            any = true;
            // Geprüfte Arithmetik: übergroße Angaben (z. B. `-P99999999999Y`) sind ein
            // Eingabefehler und dürfen keinen Überlauf/Panic auslösen.
            let part = match (c, in_time) {
                ('Y', false) => n.checked_mul(365).and_then(Duration::try_days),
                ('M', false) => n.checked_mul(30).and_then(Duration::try_days),
                ('W', false) => Duration::try_weeks(n),
                ('D', false) => Duration::try_days(n),
                ('H', true) => Duration::try_hours(n),
                ('M', true) => Duration::try_minutes(n),
                ('S', true) => Duration::try_seconds(n),
                _ => return Err(bad()),
            }
            .ok_or_else(bad)?;
            total = total.checked_add(&part).ok_or_else(bad)?;
        }
        if !num.is_empty() {
            return Err(bad());
        }
    }
    if !any {
        return Err(bad());
    }
    if negative {
        now.checked_sub_signed(total)
    } else {
        now.checked_add_signed(total)
    }
    .ok_or_else(bad)
}

fn filter_hash(tool: &str, args: &Value) -> String {
    let mut map: Vec<(String, String)> = args
        .as_object()
        .map(|o| {
            o.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "cursor" | "limit"))
                .map(|(k, v)| (k.clone(), v.to_string()))
                .collect()
        })
        .unwrap_or_default();
    map.sort();
    let mut h = DefaultHasher::new();
    tool.hash(&mut h);
    map.hash(&mut h);
    format!("{:x}", h.finish() & 0xffff_ffff)
}

fn encode_cursor(c: &PageCursor, hash: &str) -> String {
    URL_SAFE_NO_PAD.encode(json!({ "k": ts(c.key), "i": c.id.to_string(), "h": hash }).to_string())
}

fn decode_cursor(raw: &str, hash: &str) -> AppResult<PageCursor> {
    let bad = || {
        AppError::invalid_request("Cursor does not match the query")
            .with_details(json!({ "path": "/cursor" }))
    };
    let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| bad())?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    if v.get("h").and_then(Value::as_str) != Some(hash) {
        return Err(bad());
    }
    let key = v
        .get("k")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .ok_or_else(bad)?
        .with_timezone(&Utc);
    let id = v
        .get("i")
        .and_then(Value::as_str)
        .and_then(|s| UserId::from_str(s).ok())
        .ok_or_else(bad)?
        .0;
    Ok(PageCursor { key, id })
}

/// `limit`, `orderBy` und `cursor` der Listen-Tools.
pub(crate) fn page_request(tool: &str, args: &Value) -> AppResult<PageRequest> {
    let limit = match args.get("limit") {
        None | Some(Value::Null) => 50,
        Some(v) => {
            let n = v
                .as_f64()
                .ok_or_else(|| AppError::invalid_request("`limit` must be a number"))?;
            if !(1.0..=250.0).contains(&n) {
                return Err(
                    AppError::invalid_request("`limit` must be between 1 and 250")
                        .with_details(json!({ "path": "/limit" })),
                );
            }
            n as i64
        }
    };
    let order_by = match arg_str(args, "orderBy")? {
        None | Some("updatedAt") => OrderBy::UpdatedAt,
        Some("createdAt") => OrderBy::CreatedAt,
        Some(_) => {
            return Err(
                AppError::invalid_request("`orderBy` must be createdAt or updatedAt")
                    .with_details(json!({ "path": "/orderBy" })),
            );
        }
    };
    let cursor = match arg_str(args, "cursor")? {
        Some(c) if !c.is_empty() => Some(decode_cursor(c, &filter_hash(tool, args))?),
        _ => None,
    };
    Ok(PageRequest {
        limit,
        cursor,
        order_by,
    })
}

/// Listenhülle nach docs/00 §5: `<plural>`, `hasNextPage`, `cursor` nur bei weiteren Seiten.
pub(crate) fn envelope<T>(
    key: &str,
    tool: &str,
    args: &Value,
    page: &Page<T>,
    items: Vec<Value>,
) -> Value {
    let mut out = Map::new();
    out.insert(key.into(), Value::Array(items));
    out.insert("hasNextPage".into(), Value::Bool(page.has_next_page()));
    if let Some(c) = &page.next {
        out.insert(
            "cursor".into(),
            Value::String(encode_cursor(c, &filter_hash(tool, args))),
        );
    }
    Value::Object(out)
}

pub(crate) fn team_ref(team: &lineal_app::ports::repos::team::Team) -> Value {
    json!({ "id": team.id.to_string(), "key": team.key, "name": team.name })
}

fn team_json(v: &TeamView) -> Value {
    let t = &v.team;
    json!({
        "id": t.id.to_string(), "key": t.key, "name": t.name, "displayName": v.display_name,
        "parentId": t.parent_id.map(|p| p.to_string()), "icon": t.icon, "color": t.color,
        "visibility": t.visibility.as_str(), "createdAt": ts(t.created_at), "updatedAt": ts(t.updated_at),
    })
}

fn team_detail_json(v: &TeamView) -> Value {
    let mut j = team_json(v);
    if let Some(o) = j.as_object_mut() {
        o.insert("timezone".into(), json!(v.team.timezone));
        o.insert("cyclesEnabled".into(), json!(v.team.cycles_enabled));
        o.insert("triageEnabled".into(), json!(v.team.triage_enabled));
        o.insert(
            "issueEstimationType".into(),
            json!(v.team.issue_estimation_type.as_str()),
        );
        o.insert(
            "issueEstimationAllowZero".into(),
            json!(v.team.issue_estimation_allow_zero),
        );
    }
    j
}

fn user_json(u: &User) -> Value {
    let status = match u.last_seen {
        Some(t) => format!("Offline (last seen {})", ts(t)),
        None => "Offline".to_string(),
    };
    json!({
        "id": u.id.to_string(), "name": if u.name.is_empty() { u.email.clone() } else { u.name.clone() },
        "displayName": u.display_name, "email": u.email, "avatarUrl": u.avatar_url,
        "isAdmin": matches!(u.role, lineal_domain::enums::UserRole::Owner | lineal_domain::enums::UserRole::Admin),
        "isGuest": u.role == lineal_domain::enums::UserRole::Guest,
        "isActive": u.active, "status": status,
        "createdAt": ts(u.created_at), "updatedAt": ts(u.updated_at),
    })
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    let args = if args.is_null() { json!({}) } else { args };
    match tool {
        "get_workspace" => {
            let org = app.get_workspace(ctx).await?;
            Ok(
                json!({ "id": org.id.to_string(), "name": org.name, "urlKey": org.url_key, "url": app.workspace_url(&org) }),
            )
        }
        "list_teams" => {
            let now = app.clock.now();
            let time = |key: &str| -> AppResult<Option<DateTime<Utc>>> {
                arg_str(&args, key)?
                    .map(|s| parse_time_filter(key, s, now))
                    .transpose()
            };
            let input = ListTeamsInput {
                query: arg_str(&args, "query")?.map(str::to_string),
                parent_team: arg_str(&args, "parentTeam")?.map(str::to_string),
                ancestor_team: arg_str(&args, "ancestorTeam")?.map(str::to_string),
                created_after: time("createdAt")?,
                updated_after: time("updatedAt")?,
                include_archived: arg_bool(&args, "includeArchived"),
                page: page_request(tool, &args)?,
            };
            let page = app.list_teams(ctx, input).await?;
            let items = page.items.iter().map(team_json).collect();
            Ok(envelope("teams", tool, &args, &page, items))
        }
        "get_team" => {
            let v = app.get_team(ctx, required_str(&args, "query")?).await?;
            Ok(team_detail_json(&v))
        }
        "list_users" => {
            let input = ListUsersInput {
                query: arg_str(&args, "query")?.map(str::to_string),
                team: arg_str(&args, "team")?.map(str::to_string),
                page: page_request(tool, &args)?,
            };
            let page = app.list_users(ctx, input).await?;
            let items = page.items.iter().map(user_json).collect();
            Ok(envelope("users", tool, &args, &page, items))
        }
        "get_user" => {
            let v = app.get_user(ctx, required_str(&args, "query")?).await?;
            let mut j = user_json(&v.user);
            if let Some(o) = j.as_object_mut() {
                o.insert(
                    "teams".into(),
                    Value::Array(v.teams.iter().map(|t| team_ref(&t.team)).collect()),
                );
            }
            Ok(j)
        }
        other => Err(AppError::internal(format!(
            "tool `{other}` is not handled by teams_users"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_filters() {
        let now = DateTime::parse_from_rfc3339("2026-10-10T07:15:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            parse_time_filter("f", "-P1D", now).unwrap(),
            now - Duration::days(1)
        );
        assert_eq!(
            parse_time_filter("f", "-P2W", now).unwrap(),
            now - Duration::weeks(2)
        );
        assert_eq!(
            parse_time_filter("f", "-PT3H", now).unwrap(),
            now - Duration::hours(3)
        );
        assert!(parse_time_filter("f", "2026-10-01", now).is_ok());
        assert!(parse_time_filter("f", "2026-10-01T00:00:00.000Z", now).is_ok());
        for bad in ["", "yesterday", "P", "-P1X", "P1"] {
            assert!(parse_time_filter("f", bad, now).is_err(), "{bad}");
        }
    }

    #[test]
    fn oversized_durations_are_rejected_without_panic() {
        let now = Utc::now();
        for raw in [
            "-P99999999999Y",
            "P9223372036854775807D",
            "-PT9223372036854775807S",
            "P99999999999999W",
        ] {
            assert!(parse_time_filter("createdAt", raw, now).is_err(), "{raw}");
        }
    }

    #[test]
    fn cursor_roundtrip_and_filter_binding() {
        let c = PageCursor {
            key: Utc::now(),
            id: uuid_like().0,
        };
        let h = filter_hash("list_users", &json!({ "query": "a" }));
        let enc = encode_cursor(&c, &h);
        assert_eq!(decode_cursor(&enc, &h).unwrap().id, c.id);
        let other = filter_hash("list_users", &json!({ "query": "b" }));
        assert!(decode_cursor(&enc, &other).is_err());
        assert!(decode_cursor("!!!", &h).is_err());
    }

    fn uuid_like() -> lineal_domain::ids::UserId {
        lineal_domain::ids::UserId::new_v7()
    }
}

#[cfg(test)]
#[path = "teams_users_tests.rs"]
mod contract_tests;
