//! Lesbare Kennungen. WP01 ergänzt `IssueIdentifier`, `ProjectIdentifier`, `parse_identifier`.

use crate::error::DomainError;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Team-Key, validiert gegen `^[A-Z][A-Z0-9]{0,6}$`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TeamKey(String);

impl TeamKey {
    /// Maximale Länge eines Keys.
    pub const MAX_LEN: usize = 7;

    /// Validiert und erzeugt einen Key (exakt, keine Normalisierung).
    pub fn parse(s: &str) -> Result<Self, DomainError> {
        let bytes = s.as_bytes();
        let ok = !bytes.is_empty()
            && bytes.len() <= Self::MAX_LEN
            && bytes[0].is_ascii_uppercase()
            && bytes[1..]
                .iter()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        if ok {
            Ok(Self(s.to_owned()))
        } else {
            Err(DomainError::validation(
                "key",
                "must match ^[A-Z][A-Z0-9]{0,6}$",
            ))
        }
    }

    /// Der Key als `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TeamKey {
    type Error = DomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<TeamKey> for String {
    fn from(k: TeamKey) -> Self {
        k.0
    }
}

impl fmt::Display for TeamKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_keys() {
        for k in ["ENG", "WEB", "A", "A1", "ABCDEFG", "X9Y8Z7W"] {
            assert!(TeamKey::parse(k).is_ok(), "{k}");
        }
    }

    #[test]
    fn rejects_invalid_keys() {
        for k in ["", "eng", "1AB", "ABCDEFGH", "EN G", "EN-G", "ÄB", "Eng", "_AB"] {
            assert!(TeamKey::parse(k).is_err(), "{k}");
        }
    }

    #[test]
    fn serde_validates() {
        assert_eq!(serde_json::to_string(&TeamKey::parse("ENG").unwrap()).unwrap(), "\"ENG\"");
        assert!(serde_json::from_str::<TeamKey>("\"eng\"").is_err());
    }
}
