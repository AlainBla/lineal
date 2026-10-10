//! Volltextsuche (WP11, docs/05 §5, docs/10 §5).
//!
//! Die Suche läuft über generierte `tsvector`-Spalten (Konfiguration `simple`, Migration
//! 0015/1100); es gibt keinen separaten Index, den der Outbox-Konsument pflegen müsste, und die
//! Berechtigung wird zur Abfragezeit als UND angehängt (docs/10 §5.3).

use lineal_domain::identifier::{ParsedIdentifier, parse_identifier};
use sqlx::{Postgres, QueryBuilder};

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
                let full = i.to_string();
                qb.push(format!(" AND ({alias}.identifier = "));
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
            // Projekt-/Initiativ-Kennungen sind keine Issue-Kennungen.
            _ => {
                qb.push(" AND FALSE");
            }
        }
        return;
    }
    match tsquery(text) {
        Some(q) => {
            qb.push(format!(" AND {alias}.search_vector @@ to_tsquery('simple', "));
            qb.push_bind(q);
            qb.push(")");
        }
        None => {
            qb.push(" AND FALSE");
        }
    }
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
            search_identifier("p-eng-7").map(|p| p.to_string()).as_deref(),
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
