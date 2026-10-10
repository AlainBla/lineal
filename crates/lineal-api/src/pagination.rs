//! Relay-Paginierungshelfer (docs/09 §1.3, 00 §5): opake Cursor, Limit-Grenzen, `PageInfo`
//! und die Umrechnung in die REST/MCP-Hülle `{ <plural>, hasNextPage, cursor }`.
//!
//! Ablauf im Repository: `limit + 1` Zeilen holen (`PageRequest::fetch_limit`), dann
//! `finish_page` aufrufen.

use async_graphql::{Enum, SimpleObject};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use lineal_app::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

pub const DEFAULT_PAGE_SIZE: u32 = 50;
pub const MAX_PAGE_SIZE: u32 = 250;
/// Projekte: Maximum 50 (00 §5).
pub const MAX_PROJECT_PAGE_SIZE: u32 = 50;

/// Sortierfeld, Default `updatedAt` absteigend (00 §5).
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[graphql(name = "PaginationOrderBy", rename_items = "camelCase")]
pub enum PaginationOrderBy {
    CreatedAt,
    #[default]
    UpdatedAt,
}

impl PaginationOrderBy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CreatedAt => "createdAt",
            Self::UpdatedAt => "updatedAt",
        }
    }
}

/// Relay-`PageInfo` (09 §1.3).
#[derive(SimpleObject, Debug, Clone, PartialEq, Eq, Default)]
pub struct PageInfo {
    pub has_next_page: bool,
    pub has_previous_page: bool,
    pub start_cursor: Option<String>,
    pub end_cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct CursorWire {
    o: String,
    k: String,
    i: Uuid,
}

/// Position in einer sortierten Liste: Sortierwert (z. B. ISO-Zeitpunkt) und ID als Tiebreaker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    pub sort_key: String,
    pub id: Uuid,
}

impl Cursor {
    /// base64url von `{order, sortKey, id}`; opak für Clients.
    pub fn encode(&self, order: PaginationOrderBy) -> String {
        let wire = CursorWire {
            o: order.as_str().to_owned(),
            k: self.sort_key.clone(),
            i: self.id,
        };
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&wire).unwrap_or_default())
    }

    /// Dekodiert; ein Cursor einer anderen Sortierung ist `invalid_request` (09 §1.3).
    pub fn decode(s: &str, order: PaginationOrderBy) -> AppResult<Self> {
        let bad = || AppError::invalid_request("Invalid cursor.");
        let bytes = URL_SAFE_NO_PAD.decode(s).map_err(|_| bad())?;
        let wire: CursorWire = serde_json::from_slice(&bytes).map_err(|_| bad())?;
        if wire.o != order.as_str() {
            return Err(AppError::invalid_request(
                "Cursor does not match the requested orderBy.",
            ));
        }
        Ok(Self {
            sort_key: wire.k,
            id: wire.i,
        })
    }
}

/// Validierte Paginierungsargumente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRequest {
    pub limit: u32,
    pub after: Option<Cursor>,
    pub order_by: PaginationOrderBy,
}

impl PageRequest {
    /// Zeilenzahl, die das Repository laden soll (eine mehr zur `hasNextPage`-Erkennung).
    pub const fn fetch_limit(&self) -> u32 {
        self.limit + 1
    }
}

/// Baut den `PageRequest` aus `first/after/last/before` (GraphQL) bzw. `limit/cursor`
/// (REST/MCP; dort `first = limit`, `after = cursor`). `max` ist 250 bzw. 50 für Projekte;
/// größere Werte werden auf `max` begrenzt. Rückwärts-Paginierung (`last`/`before`) wird
/// nicht unterstützt (`invalid_request`).
pub fn page_request(
    first: Option<i32>,
    after: Option<&str>,
    last: Option<i32>,
    before: Option<&str>,
    order_by: PaginationOrderBy,
    max: u32,
) -> AppResult<PageRequest> {
    if last.is_some() || before.is_some() {
        return Err(AppError::invalid_request(
            "Backward pagination (last/before) is not supported.",
        ));
    }
    let limit = match first {
        None => DEFAULT_PAGE_SIZE.min(max),
        Some(n) if n < 1 => {
            return Err(AppError::invalid_request("first/limit must be at least 1."));
        }
        Some(n) => u32::try_from(n).unwrap_or(max).min(max),
    };
    let after = after.map(|c| Cursor::decode(c, order_by)).transpose()?;
    Ok(PageRequest {
        limit,
        after,
        order_by,
    })
}

/// Eine Seite Ergebnisse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    pub nodes: Vec<T>,
    pub has_next_page: bool,
    pub start_cursor: Option<String>,
    pub end_cursor: Option<String>,
}

/// Schneidet die über-geladene Zeilenliste (`fetch_limit`) auf `limit` zu.
pub fn finish_page<T>(
    mut rows: Vec<T>,
    req: &PageRequest,
    cursor_of: impl Fn(&T) -> Cursor,
) -> Page<T> {
    let has_next_page = rows.len() > req.limit as usize;
    rows.truncate(req.limit as usize);
    let start_cursor = rows.first().map(|r| cursor_of(r).encode(req.order_by));
    let end_cursor = rows.last().map(|r| cursor_of(r).encode(req.order_by));
    Page {
        nodes: rows,
        has_next_page,
        start_cursor,
        end_cursor,
    }
}

impl<T> Page<T> {
    /// Relay-`PageInfo`.
    pub fn page_info(&self) -> PageInfo {
        PageInfo {
            has_next_page: self.has_next_page,
            has_previous_page: false,
            start_cursor: self.start_cursor.clone(),
            end_cursor: self.end_cursor.clone(),
        }
    }

    /// REST/MCP-Hülle nach 00 §5; `cursor` fehlt bei `hasNextPage = false`.
    pub fn envelope(&self, plural: &str, items: Vec<Value>) -> Value {
        let mut out = json!({ plural: items, "hasNextPage": self.has_next_page });
        if self.has_next_page
            && let Some(c) = &self.end_cursor
        {
            out["cursor"] = Value::String(c.clone());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cur(n: u128) -> Cursor {
        Cursor {
            sort_key: format!("2026-10-10T07:15:0{n}.000Z"),
            id: Uuid::from_u128(n),
        }
    }

    #[test]
    fn cursor_roundtrip_and_order_binding() {
        let c = cur(3);
        let enc = c.encode(PaginationOrderBy::UpdatedAt);
        assert_eq!(
            Cursor::decode(&enc, PaginationOrderBy::UpdatedAt).unwrap(),
            c
        );
        let err = Cursor::decode(&enc, PaginationOrderBy::CreatedAt).unwrap_err();
        assert_eq!(err.kind, lineal_app::ErrorKind::InvalidRequest);
        assert!(Cursor::decode("###", PaginationOrderBy::UpdatedAt).is_err());
    }

    #[test]
    fn limits_default_clamp_and_reject() {
        let o = PaginationOrderBy::default();
        let lim = |first, max| page_request(first, None, None, None, o, max).map(|r| r.limit);
        assert_eq!(lim(None, 250).unwrap(), 50);
        assert_eq!(lim(Some(999), 250).unwrap(), 250);
        assert_eq!(lim(Some(999), 50).unwrap(), 50);
        assert!(lim(Some(0), 250).is_err());
        assert!(page_request(None, None, Some(5), None, o, 250).is_err());
    }

    #[test]
    fn finish_page_detects_next_page_and_envelope_omits_cursor_on_last() {
        let o = PaginationOrderBy::UpdatedAt;
        let req = page_request(Some(2), None, None, None, o, 250).unwrap();
        assert_eq!(req.fetch_limit(), 3);
        let page = finish_page(vec![1u128, 2, 3], &req, |n| cur(*n));
        assert!(page.has_next_page);
        assert_eq!(page.nodes, vec![1, 2]);
        let env = page.envelope("issues", vec![json!(1), json!(2)]);
        assert_eq!(env["hasNextPage"], true);
        assert!(env["cursor"].is_string());

        let last = finish_page(vec![1u128], &req, |n| cur(*n));
        assert!(!last.has_next_page);
        assert!(last.envelope("issues", vec![]).get("cursor").is_none());
        let empty = finish_page(Vec::<u128>::new(), &req, |n| cur(*n));
        assert_eq!(empty.page_info().end_cursor, None);
    }
}
