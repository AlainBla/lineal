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
        Self {
            kind,
            message: message.into(),
            details: json!({}),
        }
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

impl AppError {
    /// Fehler eines Transaktionskonflikts (Deadlock 40P01, Serialisierung 40001), nach erschöpften
    /// Wiederholungen als `conflict` mit `details.retryable = true` gemeldet: der Client darf den
    /// Aufruf unverändert wiederholen.
    pub fn retryable_conflict(sqlstate: &str) -> Self {
        Self::conflict("The operation conflicted with a concurrent change; please retry.")
            .with_details(json!({ "retryable": true, "sqlstate": sqlstate }))
    }

    /// `true`, wenn der Fehler ein wiederholbarer Transaktionskonflikt ist.
    pub fn is_retryable(&self) -> bool {
        self.kind == ErrorKind::Conflict && self.details["retryable"] == json!(true)
    }
}

/// Maximale Anzahl Versuche von [`with_retry`] (ein Versuch plus drei Wiederholungen).
pub const MAX_TX_ATTEMPTS: u32 = 4;

/// Führt `op` (eine komplette Transaktion: begin, Änderungen, commit) aus und wiederholt sie bei
/// einem wiederholbaren Transaktionskonflikt (SQLSTATE 40P01/40001, siehe
/// [`AppError::is_retryable`]) mit kurzem, zufällig gestreutem Backoff. Nach [`MAX_TX_ATTEMPTS`]
/// Versuchen wird der letzte Fehler (ein `conflict` mit `retryable: true`) zurückgegeben.
/// `op` darf keine Nebenwirkungen außerhalb der Transaktion haben.
pub async fn with_retry<T, F, Fut>(mut op: F) -> AppResult<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = AppResult<T>>,
{
    let mut attempt = 1;
    loop {
        match op().await {
            Err(e) if e.is_retryable() && attempt < MAX_TX_ATTEMPTS => {
                let jitter = (uuid::Uuid::new_v4().as_u128() % 20) as u64;
                let ms = 10 * u64::from(attempt) + jitter;
                tracing::warn!(attempt, "transaction conflict, retrying");
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                attempt += 1;
            }
            other => return other,
        }
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
            DomainError::Validation { field, reason } => Self::invalid_request(e.to_string())
                .with_details(json!({ "field": field, "reason": reason })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_statuses_follow_00_section_7() {
        let table: Vec<_> = ErrorKind::ALL
            .iter()
            .map(|k| (k.code(), k.http_status()))
            .collect();
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

    #[tokio::test]
    async fn with_retry_repeats_simulated_deadlock_then_succeeds() {
        let calls = std::sync::atomic::AtomicU32::new(0);
        let r = with_retry(|| async {
            if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) < 2 {
                Err(AppError::retryable_conflict("40P01"))
            } else {
                Ok(7)
            }
        })
        .await;
        assert_eq!(r.unwrap(), 7);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn with_retry_gives_up_as_retryable_conflict() {
        let calls = std::sync::atomic::AtomicU32::new(0);
        let r: AppResult<()> = with_retry(|| async {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(AppError::retryable_conflict("40P01"))
        })
        .await;
        let e = r.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Conflict);
        assert!(e.is_retryable());
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            MAX_TX_ATTEMPTS
        );
    }

    #[tokio::test]
    async fn with_retry_does_not_repeat_other_errors() {
        let calls = std::sync::atomic::AtomicU32::new(0);
        let r: AppResult<()> = with_retry(|| async {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(AppError::conflict("plain"))
        })
        .await;
        assert!(!r.unwrap_err().is_retryable());
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn domain_error_maps_to_invalid_request() {
        let e: AppError = DomainError::validation("key", "bad").into();
        assert_eq!(e.kind, ErrorKind::InvalidRequest);
        assert_eq!(e.details["field"], "key");
    }
}
