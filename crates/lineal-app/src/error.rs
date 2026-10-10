//! Anwendungsfehler, 1:1 abbildbar auf das Fehlermodell aus docs/00 §7.

use lineal_domain::DomainError;
use serde_json::{Value, json};
use std::fmt;

/// Fehlerart = `error`-Code des Fehlermodells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    InvalidRequest,
    Unauthenticated,
    Forbidden,
    NotFound,
    AmbiguousReference,
    Conflict,
    LimitExceeded,
    RateLimited,
    Internal,
}

impl ErrorKind {
    /// Alle Arten (für Snapshot-Tests).
    pub const ALL: [ErrorKind; 9] = [
        Self::InvalidRequest,
        Self::Unauthenticated,
        Self::Forbidden,
        Self::NotFound,
        Self::AmbiguousReference,
        Self::Conflict,
        Self::LimitExceeded,
        Self::RateLimited,
        Self::Internal,
    ];

    /// Wert des Felds `error`.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::Unauthenticated => "unauthenticated",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::AmbiguousReference => "ambiguous_reference",
            Self::Conflict => "conflict",
            Self::LimitExceeded => "limit_exceeded",
            Self::RateLimited => "rate_limited",
            Self::Internal => "internal",
        }
    }

    /// HTTP-Status laut 00 §7.
    pub const fn http_status(&self) -> u16 {
        match self {
            Self::InvalidRequest => 400,
            Self::Unauthenticated => 401,
            Self::Forbidden => 403,
            Self::NotFound => 404,
            Self::AmbiguousReference | Self::Conflict => 409,
            Self::LimitExceeded => 422,
            Self::RateLimited => 429,
            Self::Internal => 500,
        }
    }
}

/// Fehler eines Use-Cases.
#[derive(Debug, Clone, PartialEq)]
pub struct AppError {
    pub kind: ErrorKind,
    pub message: String,
    pub details: Value,
}

/// Ergebnis eines Use-Cases.
pub type AppResult<T> = Result<T, AppError>;

macro_rules! ctor {
    ($($fn_name:ident => $kind:ident),+ $(,)?) => {$(
        #[doc = concat!("Fehler der Art `", stringify!($kind), "`.")]
        pub fn $fn_name(message: impl Into<String>) -> Self { Self::new(ErrorKind::$kind, message) }
    )+};
}

impl AppError {
    /// Erzeugt einen Fehler ohne Details.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), details: json!({}) }
    }

    ctor! {
        invalid_request => InvalidRequest,
        unauthenticated => Unauthenticated,
        forbidden => Forbidden,
        not_found => NotFound,
        ambiguous_reference => AmbiguousReference,
        conflict => Conflict,
        limit_exceeded => LimitExceeded,
        rate_limited => RateLimited,
        internal => Internal,
    }

    /// Setzt `details`.
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.code(), self.message)
    }
}

impl std::error::Error for AppError {}

impl From<DomainError> for AppError {
    fn from(e: DomainError) -> Self {
        match &e {
            DomainError::Validation { field, reason } => {
                Self::invalid_request(e.to_string()).with_details(json!({ "field": field, "reason": reason }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_statuses_follow_00_section_7() {
        let table: Vec<_> = ErrorKind::ALL.iter().map(|k| (k.code(), k.http_status())).collect();
        assert_eq!(
            table,
            [
                ("invalid_request", 400),
                ("unauthenticated", 401),
                ("forbidden", 403),
                ("not_found", 404),
                ("ambiguous_reference", 409),
                ("conflict", 409),
                ("limit_exceeded", 422),
                ("rate_limited", 429),
                ("internal", 500),
            ]
        );
    }

    #[test]
    fn domain_error_maps_to_invalid_request() {
        let e: AppError = DomainError::validation("key", "bad").into();
        assert_eq!(e.kind, ErrorKind::InvalidRequest);
        assert_eq!(e.details["field"], "key");
    }
}
