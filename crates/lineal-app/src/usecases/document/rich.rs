//! Rich-Text-Hilfen für Dokumente (WP34, docs/06 §1.11/§1.12): Erkennen und Verlinken von
//! Issue-Kennungen (`ENG-123`) sowie `@Name`-Erwähnungen in Dokumenten.
//!
//! Die Kennungen werden beim **Ausgeben** verlinkt (nicht gespeichert): so bleiben die Anker der
//! Patch-Operationen (`replace` mit genau einem Vorkommen) gegen den gespeicherten Text gültig.

use crate::app::App;
use crate::context::RequestCtx;
use crate::error::AppResult;
use crate::policy::PolicySnapshot;
use crate::ports::Tx;
use crate::ports::repos::document::{Document, ParentScope};
use crate::resolve;
use crate::resolve::user::actor_user_id;
use crate::usecases::issue::mentions::extract_handles;
use crate::usecases::notification::fanout::Fan;
use chrono::{DateTime, Utc};
use lineal_domain::enums::UserRole;
use lineal_domain::ids::{IssueId, UserId};
use std::collections::{BTreeMap, BTreeSet};

/// Höchstzahl verschiedener Kennungen, die je Dokument aufgelöst werden.
pub const MAX_LINKED_IDENTIFIERS: usize = 50;

/// Eine Kennung im Text (Byte-Bereich `start..end`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifierSpan {
    pub start: usize,
    pub end: usize,
    pub identifier: String,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_uppercase()
}

/// Findet Issue-Kennungen im Markdown, die nicht in Code, Links, Autolinks oder URLs stehen.
pub fn find_identifiers(md: &str) -> Vec<IdentifierSpan> {
    let mut out = Vec::new();
    let mut in_fence = false;
    let mut offset = 0;
    for line in md.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        } else if !in_fence {
            scan_line(line, offset, &mut out);
        }
        offset += line.len();
    }
    out
}

fn scan_line(line: &str, base: usize, out: &mut Vec<IdentifierSpan>) {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut in_code = false;
    let mut bracket_depth = 0usize; // innerhalb `[ ... ]` (Linktext)
    let mut in_target = false; // innerhalb `( ... )` direkt nach `]`
    let mut in_angle = false;
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];
        match c {
            '`' => in_code = !in_code,
            _ if in_code => {}
            '[' => bracket_depth += 1,
            ']' => {
                bracket_depth = bracket_depth.saturating_sub(1);
                if chars.get(i + 1).is_some_and(|(_, n)| *n == '(') {
                    in_target = true;
                    i += 1;
                }
            }
            ')' if in_target => in_target = false,
            '<' => in_angle = true,
            '>' => in_angle = false,
            _ if is_ident_start(c) && bracket_depth == 0 && !in_target && !in_angle => {
                let prev_ok = i == 0
                    || !chars[i - 1].1.is_alphanumeric()
                        && !matches!(chars[i - 1].1, '-' | '_' | '/' | '.' | '#' | '@' | ':');
                if prev_ok && let Some(len) = match_identifier(&chars[i..]) {
                    let end_idx = i + len;
                    let end = chars.get(end_idx).map_or(line.len(), |(p, _)| *p);
                    let next_ok = chars
                        .get(end_idx)
                        .is_none_or(|(_, n)| !n.is_alphanumeric() && !matches!(n, '-' | '_' | '/'));
                    if next_ok && !token_is_url(&chars, i) {
                        out.push(IdentifierSpan {
                            start: base + pos,
                            end: base + end,
                            identifier: line[pos..end].to_owned(),
                        });
                    }
                    i = end_idx;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
}

/// Länge (in Zeichen) einer Kennung `[A-Z][A-Z0-9]{0,6}-[0-9]{1,7}` am Anfang von `s`.
fn match_identifier(s: &[(usize, char)]) -> Option<usize> {
    let mut n = 1;
    while n < 8
        && s.get(n)
            .is_some_and(|(_, c)| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        n += 1;
    }
    if s.get(n).map(|(_, c)| *c) != Some('-') {
        return None;
    }
    let digits_start = n + 1;
    let mut m = digits_start;
    while m < digits_start + 7 && s.get(m).is_some_and(|(_, c)| c.is_ascii_digit()) {
        m += 1;
    }
    (m > digits_start).then_some(m)
}

/// Gehört die Stelle zu einem URL-artigen Token (`https://…/ENG-1`, `www.…`)?
fn token_is_url(chars: &[(usize, char)], at: usize) -> bool {
    let mut a = at;
    while a > 0 && !chars[a - 1].1.is_whitespace() {
        a -= 1;
    }
    let mut b = at;
    while b < chars.len() && !chars[b].1.is_whitespace() {
        b += 1;
    }
    let token: String = chars[a..b].iter().map(|(_, c)| *c).collect();
    token.contains("://") || token.starts_with("www.")
}

/// Ersetzt die Kennungen aus `links` (Kennung → URL) durch `[ENG-123](url)`.
pub fn apply_links(md: &str, spans: &[IdentifierSpan], links: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(md.len() + spans.len() * 32);
    let mut last = 0;
    for s in spans {
        let Some(url) = links.get(&s.identifier) else {
            continue;
        };
        out.push_str(&md[last..s.start]);
        out.push('[');
        out.push_str(&s.identifier);
        out.push_str("](");
        out.push_str(url);
        out.push(')');
        last = s.end;
    }
    out.push_str(&md[last..]);
    out
}

/// Verlinkt Kennungen sichtbarer Issues (`/<urlKey>/issue/<Kennung>`); unbekannte oder für den
/// Akteur unsichtbare Kennungen bleiben Klartext (kein Hinweis auf ihre Existenz).
pub async fn link_visible_issues(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    url_key: &str,
    md: &str,
) -> crate::error::AppResult<String> {
    let spans = find_identifiers(md);
    if spans.is_empty() {
        return Ok(md.to_owned());
    }
    let distinct: BTreeSet<&str> = spans.iter().map(|s| s.identifier.as_str()).collect();
    let visible = snap.visible_team_ids();
    let mut links = BTreeMap::new();
    for ident in distinct.into_iter().take(MAX_LINKED_IDENTIFIERS) {
        let id: IssueId = match resolve::issue::lookup(tx, ident).await {
            Ok(id) => id,
            Err(_) => continue,
        };
        if let Some(issue) = tx.issues().get(id, false).await?
            && !issue.trashed
            && visible.contains(&issue.team_id)
        {
            links.insert(
                ident.to_owned(),
                format!("/{url_key}/issue/{}", issue.identifier),
            );
        }
    }
    Ok(apply_links(md, &spans, &links))
}

impl App {
    /// Erwähnungen (`@Name`) in Dokumenten (docs/06 §1.12): neu hinzugekommene Namen (gegenüber
    /// `old`) werden aufgelöst, abonnieren das Dokument und erhalten eine Benachrichtigung
    /// `documentMention` über den vorhandenen Fan-out-Weg. Nur aktive User, die das Elternobjekt
    /// sehen (Gäste keine Initiative-Dokumente); der Akteur selbst bekommt nichts.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn notify_document_mentions(
        &self,
        tx: &mut dyn Tx,
        ctx: &RequestCtx,
        doc: &Document,
        scope: &ParentScope,
        old: &str,
        new: &str,
        now: DateTime<Utc>,
    ) -> AppResult<Vec<UserId>> {
        let before = extract_handles(old);
        let mut mentioned: Vec<UserId> = Vec::new();
        for handle in extract_handles(new).difference(&before) {
            for u in tx.users().find_by_name(handle).await? {
                if !u.active || u.display_name != *handle || mentioned.contains(&u.id) {
                    continue;
                }
                if scope.workspace && u.role == UserRole::Guest {
                    continue;
                }
                mentioned.push(u.id);
            }
        }
        if mentioned.is_empty() {
            return Ok(mentioned);
        }
        let actor = actor_user_id(ctx);
        let mut delivered = Vec::new();
        let mut fan = Fan::new(tx, now, actor).await?;
        fan.reset_for_document(doc.id.0, scope.teams.clone());
        fan.add_all(mentioned.iter().copied(), "documentMention");
        fan.deliver(
            self.ids.as_ref(),
            &format!("docmention:{}:{}", doc.id, now.timestamp_millis()),
        )
        .await?;
        drop(fan);
        for u in &mentioned {
            if Some(*u) == actor {
                continue;
            }
            tx.documents()
                .subscribe(self.ids.new_uuid(), doc.id, *u, "mention", now)
                .await?;
            delivered.push(*u);
        }
        Ok(delivered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idents(md: &str) -> Vec<String> {
        find_identifiers(md)
            .into_iter()
            .map(|s| s.identifier)
            .collect()
    }

    #[test]
    fn finds_plain_identifiers() {
        assert_eq!(idents("Siehe ENG-123 und WEB-7."), vec!["ENG-123", "WEB-7"]);
        assert_eq!(idents("(ENG-1)"), vec!["ENG-1"]);
        assert_eq!(idents("ENG-12\nDES-3"), vec!["ENG-12", "DES-3"]);
    }

    #[test]
    fn ignores_code_links_and_urls() {
        assert!(idents("`ENG-1`").is_empty());
        assert!(idents("```\nENG-1\n```").is_empty());
        assert!(idents("[ENG-1](https://x.example/ENG-2)").is_empty());
        assert!(idents("[siehe ENG-1](/acme/issue/ENG-1)").is_empty());
        assert!(idents("https://x.example/browse/ENG-1").is_empty());
        assert!(idents("<https://x.example/ENG-1>").is_empty());
        assert!(idents("xENG-1 und ENG-1x und eng-1 und ENG-").is_empty());
        assert!(idents("a-ENG-1 /ENG-1 ENG-1/2").is_empty());
    }

    #[test]
    fn apply_links_is_idempotent_on_its_output() {
        let md = "Bitte ENG-1 prüfen, nicht ENG-2.";
        let spans = find_identifiers(md);
        let mut links = BTreeMap::new();
        links.insert("ENG-1".to_owned(), "/acme/issue/ENG-1".to_owned());
        let once = apply_links(md, &spans, &links);
        assert_eq!(
            once,
            "Bitte [ENG-1](/acme/issue/ENG-1) prüfen, nicht ENG-2."
        );
        // Ein zweiter Durchlauf findet nur noch ENG-2.
        assert_eq!(idents(&once), vec!["ENG-2"]);
    }

    #[test]
    fn multibyte_text_keeps_offsets() {
        let md = "Müßig → ENG-5 äöü";
        let spans = find_identifiers(md);
        assert_eq!(spans.len(), 1);
        assert_eq!(&md[spans[0].start..spans[0].end], "ENG-5");
    }
}
