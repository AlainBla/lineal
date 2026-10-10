//! Validierungsfehler der Domain (Feld und Grund).

/// Fehler einer reinen Domain-Regel.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    /// Ein Feld verletzt eine Regel.
    #[error("invalid value for `{field}`: {reason}")]
    Validation {
        /// Betroffenes Feld (camelCase wie in der API).
        field: String,
        /// Menschenlesbarer Grund.
        reason: String,
    },
}

impl DomainError {
    /// Kurzform für einen Validierungsfehler.
    pub fn validation(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Validation {
            field: field.into(),
            reason: reason.into(),
        }
    }
}
