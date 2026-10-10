//! Lesbare Kennungen (docs/01 §6.1): `ENG-123`, `P-ENG-7`, `I-4`.

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
        // `P` und `I` sind reserviert (docs/01 §6.1): sonst wären `P-ENG-7`/`I-4` mehrdeutig.
        let ok = !matches!(s, "P" | "I")
            && !bytes.is_empty()
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

/// Issue-Kennung `KEY-n` (docs/01 §6.1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IssueIdentifier {
    /// Team-Key.
    pub team_key: TeamKey,
    /// Laufnummer im Team (>= 1).
    pub number: i32,
}

impl fmt::Display for IssueIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.team_key, self.number)
    }
}

/// Projekt-Kennung `P-KEY-n` (Key des Lead-Teams).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProjectIdentifier {
    /// Key des Lead-Teams.
    pub lead_team_key: TeamKey,
    /// Laufnummer (>= 1).
    pub number: i32,
}

impl fmt::Display for ProjectIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "P-{}-{}", self.lead_team_key, self.number)
    }
}

/// Ergebnis von [`parse_identifier`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ParsedIdentifier {
    /// `ENG-123`.
    Issue(IssueIdentifier),
    /// `P-ENG-7`.
    Project(ProjectIdentifier),
    /// `I-4` (nur die Nummer).
    Initiative {
        /// Laufnummer im Workspace (>= 1).
        number: i32,
    },
}

impl fmt::Display for ParsedIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Issue(i) => i.fmt(f),
            Self::Project(p) => p.fmt(f),
            Self::Initiative { number } => write!(f, "I-{number}"),
        }
    }
}

fn parse_number(s: &str) -> Option<i32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<i32>().ok().filter(|n| *n > 0)
}

/// Zerlegt eine Kennung (Trim + Großschreibung, docs/01 §6.1). `None`, wenn
/// die Form nicht passt (kein Bindestrich, Nummer keine positive Zahl, Key ungültig).
pub fn parse_identifier(s: &str) -> Option<ParsedIdentifier> {
    let s = s.trim().to_uppercase();
    if let Some(rest) = s.strip_prefix("I-") {
        return parse_number(rest).map(|number| ParsedIdentifier::Initiative { number });
    }
    let pos = s.rfind('-')?;
    if pos < 1 {
        return None;
    }
    let number = parse_number(&s[pos + 1..])?;
    let prefix = &s[..pos];
    if let Some(key) = prefix.strip_prefix("P-") {
        return TeamKey::parse(key).ok().map(|lead_team_key| {
            ParsedIdentifier::Project(ProjectIdentifier {
                lead_team_key,
                number,
            })
        });
    }
    TeamKey::parse(prefix)
        .ok()
        .map(|team_key| ParsedIdentifier::Issue(IssueIdentifier { team_key, number }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(k: &str, n: i32) -> Option<ParsedIdentifier> {
        Some(ParsedIdentifier::Issue(IssueIdentifier {
            team_key: TeamKey::parse(k).unwrap(),
            number: n,
        }))
    }

    fn project(k: &str, n: i32) -> Option<ParsedIdentifier> {
        Some(ParsedIdentifier::Project(ProjectIdentifier {
            lead_team_key: TeamKey::parse(k).unwrap(),
            number: n,
        }))
    }

    #[test]
    fn parses_issue_project_initiative() {
        assert_eq!(parse_identifier("ENG-123"), issue("ENG", 123));
        assert_eq!(parse_identifier("  eng-7 "), issue("ENG", 7));
        assert_eq!(parse_identifier("W3-1"), issue("W3", 1));
        assert_eq!(parse_identifier("P-ENG-7"), project("ENG", 7));
        assert_eq!(parse_identifier("p-des-12"), project("DES", 12));
        assert_eq!(
            parse_identifier("I-4"),
            Some(ParsedIdentifier::Initiative { number: 4 })
        );
    }

    #[test]
    fn rejects_malformed_identifiers() {
        for s in [
            "",
            "-",
            "ENG",
            "ENG-",
            "-5",
            "ENG-0",
            "ENG--5",
            "ENG-+5",
            "ENG-1x",
            "ENG 5",
            "ENG-99999999999",
            "I-",
            "I-x",
            "I-0",
            "P-ENG",
            "P--7",
            "P-TOOLONGKEY-3",
            "EN-G-5",
            "ÄB-1",
            "ENG-１２",
        ] {
            assert_eq!(parse_identifier(s), None, "{s:?}");
        }
    }

    #[test]
    fn display_round_trips() {
        for s in ["ENG-123", "P-ENG-7", "I-4", "WEB-1"] {
            assert_eq!(parse_identifier(s).unwrap().to_string(), s);
        }
    }

    #[test]
    fn accepts_valid_keys() {
        for k in ["ENG", "WEB", "A", "A1", "ABCDEFG", "X9Y8Z7W"] {
            assert!(TeamKey::parse(k).is_ok(), "{k}");
        }
    }

    #[test]
    fn rejects_invalid_keys() {
        for k in [
            "", "eng", "1AB", "ABCDEFGH", "EN G", "EN-G", "ÄB", "Eng", "_AB",
        ] {
            assert!(TeamKey::parse(k).is_err(), "{k}");
        }
    }

    #[test]
    fn reserved_keys_p_and_i_are_rejected() {
        assert!(TeamKey::parse("P").is_err() && TeamKey::parse("I").is_err());
        assert!(TeamKey::parse("PI").is_ok() && TeamKey::parse("IP").is_ok());
    }

    #[test]
    fn serde_validates() {
        assert_eq!(
            serde_json::to_string(&TeamKey::parse("ENG").unwrap()).unwrap(),
            "\"ENG\""
        );
        assert!(serde_json::from_str::<TeamKey>("\"eng\"").is_err());
    }
}
