//! Tool-Handler der Gruppe `triage` (WP12): `get_triage_responsibility` (docs/07 §3.14).
//! Die Eingabe wurde bereits gegen das Snapshot-Schema validiert (`limit` 1..=50, Default 10).
//!
//! Alle Schlüssel sind immer vorhanden; bei deaktivierter Triage `triageEnabled: false` und die
//! übrigen `null`/`[]`. Sichtbarkeit, Rechte und Scopes prüft `App::get_triage_responsibility`
//! (privates Team → `not_found`). Es werden nie E-Mail-Adressen ausgegeben.

use crate::envelope::{team_ref, user_ref};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use lineal_app::usecases::triage::{ShiftView, TriageInfo};
use lineal_app::{App, AppError, AppResult, RequestCtx};
use serde_json::{Value, json};

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// ISO-8601-Dauer für gleich lange Schichten (ganze Wochen, Tage oder Stunden), sonst `null`.
fn shift_length(shifts: &[ShiftView]) -> Value {
    let Some(first) = shifts.first() else {
        return Value::Null;
    };
    let len: Duration = first.ends_at - first.starts_at;
    if shifts.iter().any(|s| s.ends_at - s.starts_at != len) {
        return Value::Null;
    }
    let secs = len.num_seconds();
    if secs <= 0 {
        return Value::Null;
    }
    let text = if secs % (7 * 86_400) == 0 {
        format!("P{}W", secs / (7 * 86_400))
    } else if secs % 86_400 == 0 {
        format!("P{}D", secs / 86_400)
    } else if secs % 3600 == 0 {
        format!("PT{}H", secs / 3600)
    } else {
        return Value::Null;
    };
    Value::String(text)
}

fn info_json(info: &TriageInfo) -> Value {
    let team = team_ref(info.team.id, &info.team.key, &info.team.name);
    if !info.triage_enabled {
        return json!({
            "team": team,
            "triageEnabled": false,
            "action": Value::Null,
            "currentUser": Value::Null,
            "rotation": Value::Null,
            "upcomingShifts": [],
        });
    }
    let rotation = info.schedule.as_ref().map_or(Value::Null, |s| {
        json!({
            "id": s.id.to_string(),
            "name": s.name,
            "shiftLength": shift_length(&info.upcoming_shifts),
            "handoffAt": Value::Null,
        })
    });
    let shifts: Vec<Value> = info
        .upcoming_shifts
        .iter()
        .map(|s| {
            json!({
                "user": user_ref(s.user.id, &s.user.name),
                "startsAt": ts(s.starts_at),
                "endsAt": ts(s.ends_at),
            })
        })
        .collect();
    json!({
        "team": team,
        "triageEnabled": true,
        "action": info.action.map(|a| a.as_str()),
        "currentUser": info.current_user.as_ref().map_or(Value::Null, |u| user_ref(u.id, &u.name)),
        "rotation": rotation,
        "upcomingShifts": shifts,
    })
}

/// Führt ein Tool dieser Gruppe aus.
pub async fn handle(app: &App, ctx: &RequestCtx, tool: &str, args: Value) -> AppResult<Value> {
    match tool {
        "get_triage_responsibility" => {
            let team = args
                .get("team")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::invalid_request("team is required"))?;
            let user = args.get("user").and_then(Value::as_str);
            let limit = args.get("limit").and_then(Value::as_i64).unwrap_or(10);
            let info = app
                .get_triage_responsibility(ctx, team, user, limit.clamp(1, 50))
                .await?;
            Ok(info_json(&info))
        }
        other => Err(AppError::internal(format!(
            "tool `{other}` is not handled by triage"
        ))),
    }
}

#[cfg(test)]
#[path = "triage_tests.rs"]
mod tests;
