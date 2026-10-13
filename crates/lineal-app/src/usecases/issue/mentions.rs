//! `@displayName`-Erwähnungen in Beschreibungen (docs/03 §2.9).

use crate::context::Actor;
use crate::error::AppResult;
use crate::policy::{ActorView, PolicySnapshot, compute_visible_team_ids};
use crate::ports::Tx;
use lineal_domain::ids::{TeamId, UserId};
use std::collections::BTreeSet;

fn is_handle_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')
}

/// Entfernt Code (Fenced-Blöcke und Inline-Code): `@Name` darin ist keine Erwähnung (B6.15).
/// Ein nicht geschlossener Block reicht bis zum Textende.
pub fn strip_code(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find('`') {
        out.push_str(&rest[..pos]);
        let ticks = rest[pos..].chars().take_while(|c| *c == '`').count();
        let fence = "`".repeat(ticks);
        let after = &rest[pos + ticks..];
        match after.find(&fence) {
            Some(end) => rest = &after[end + ticks..],
            None => {
                rest = "";
                break;
            }
        }
        out.push(' ');
    }
    out.push_str(rest);
    out
}

/// Zeichen, hinter denen ein `@` keine Erwähnung einleitet (E-Mail, URL-Pfad, `@@`).
fn blocks_mention(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '@' | '\\' | ':')
}

/// Alle `@handle`-Kandidaten eines Textes (klein geschrieben), Code ausgenommen. Satzzeichen am
/// Ende werden zusätzlich abgeschnitten (`@alice.` → `alice`); ein `@` direkt hinter einem
/// Wortzeichen (E-Mail-Adresse) oder Pfadzeichen zählt nicht.
pub fn extract_handles(text: &str) -> BTreeSet<String> {
    let text = strip_code(text);
    let chars: Vec<char> = text.chars().collect();
    let mut out = BTreeSet::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' && (i == 0 || !blocks_mention(chars[i - 1])) {
            let mut j = i + 1;
            let mut handle = String::new();
            while j < chars.len() && handle.len() < 40 {
                let c = chars[j].to_ascii_lowercase();
                if chars[j].is_ascii() && is_handle_char(c) {
                    handle.push(c);
                    j += 1;
                } else {
                    break;
                }
            }
            if !handle.is_empty() {
                let trimmed = handle.trim_end_matches(['.', '-', '_']).to_owned();
                if !trimmed.is_empty() && trimmed != handle {
                    out.insert(trimmed);
                }
                out.insert(handle);
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

/// Alle Issue-Kennungen (`ENG-123`) eines Textes, großgeschrieben; höchstens 20 je Text.
/// Zeichen direkt davor oder dahinter dürfen keine Buchstaben/Ziffern/Bindestriche sein.
pub fn extract_identifiers(text: &str) -> BTreeSet<String> {
    const MAX: usize = 20;
    let chars: Vec<char> = text.chars().collect();
    let mut out = BTreeSet::new();
    let mut i = 0;
    while i < chars.len() && out.len() < MAX {
        let boundary = i == 0 || !(chars[i - 1].is_ascii_alphanumeric() || chars[i - 1] == '-');
        if boundary && chars[i].is_ascii_alphabetic() {
            let mut j = i;
            while j < chars.len() && chars[j].is_ascii_alphanumeric() {
                j += 1;
            }
            let key_len = j - i;
            if (1..=10).contains(&key_len) && j < chars.len() && chars[j] == '-' {
                let mut k = j + 1;
                while k < chars.len() && chars[k].is_ascii_digit() {
                    k += 1;
                }
                let digits = k - (j + 1);
                let after_ok =
                    k >= chars.len() || !(chars[k].is_ascii_alphanumeric() || chars[k] == '-');
                if (1..=9).contains(&digits) && after_ok {
                    let key: String = chars[i..j].iter().collect();
                    let num: String = chars[j + 1..k].iter().collect();
                    out.insert(format!("{}-{}", key.to_ascii_uppercase(), num));
                }
                i = k.max(i + 1);
                continue;
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

/// Legt für neu erwähnte Issue-Kennungen `related`-Relationen zu `issue` an (nur sichtbare,
/// beschreibbare Gegenstücke, keine Duplikate; docs/03 §4.2). `old_text` = bisheriger Text:
/// nur Kennungen, die dort noch nicht standen, zählen.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn link_issue_mentions(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    ctx: &crate::context::RequestCtx,
    ids: &dyn crate::ports::IdGen,
    now: chrono::DateTime<chrono::Utc>,
    source: crate::ports::repos::issue::HistorySource,
    issue: &crate::ports::repos::issue::Issue,
    new_text: &str,
    old_text: Option<&str>,
) -> AppResult<()> {
    // Importierte Texte verweisen auf Kennungen des Quellsystems, nicht auf lokale Issues.
    if source == crate::ports::repos::issue::HistorySource::Import {
        return Ok(());
    }
    let before = old_text.map(extract_identifiers).unwrap_or_default();
    let env = super::relations::Env {
        ctx,
        snap,
        ids,
        now,
        source,
    };
    for ident in extract_identifiers(new_text) {
        if before.contains(&ident) || ident.eq_ignore_ascii_case(&issue.identifier) {
            continue;
        }
        super::relations::link_mention(tx, &env, issue, &ident).await?;
    }
    Ok(())
}

/// Wählt die Treffer für einen `@handle`: zuerst der Anzeigename (`display`, ohne Beachtung der
/// Groß-/Kleinschreibung), sonst der Name (`name`), aber nur wenn er eindeutig ist (B6.19).
/// Alle Erwähnungs-Pfade (Issue, Kommentar, Dokument, Projekt, Initiative) nutzen diese Funktion.
pub fn pick_mention_matches<'a, T>(
    candidates: &'a [T],
    handle: &str,
    display: impl Fn(&T) -> &str,
    name: impl Fn(&T) -> &str,
) -> Vec<&'a T> {
    let exact: Vec<&T> = candidates
        .iter()
        .filter(|c| display(c).eq_ignore_ascii_case(handle))
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    let by_name: Vec<&T> = candidates
        .iter()
        .filter(|c| name(c).eq_ignore_ascii_case(handle))
        .collect();
    if by_name.len() == 1 {
        by_name
    } else {
        Vec::new()
    }
}

/// Aktive User, die ein `@handle` trifft (Anzeigename, sonst eindeutiger Name).
pub async fn users_for_handle(
    tx: &mut dyn Tx,
    handle: &str,
) -> AppResult<Vec<crate::ports::repos::user::User>> {
    let found: Vec<_> = tx
        .users()
        .find_by_name(handle)
        .await?
        .into_iter()
        .filter(|u| u.active)
        .collect();
    Ok(
        pick_mention_matches(&found, handle, |u| &u.display_name, |u| &u.name)
            .into_iter()
            .cloned()
            .collect(),
    )
}

/// Löst Erwähnungen eines Textes auf aktive User auf (Anzeigename, sonst eindeutiger Name),
/// ohne Sichtbarkeitsprüfung: Sie liegt beim Empfänger-Check des Fan-outs.
pub async fn resolve_handles(tx: &mut dyn Tx, text: &str) -> AppResult<Vec<UserId>> {
    let mut out: Vec<UserId> = Vec::new();
    for handle in extract_handles(text) {
        for u in users_for_handle(tx, &handle).await? {
            if !out.contains(&u.id) {
                out.push(u.id);
            }
        }
    }
    Ok(out)
}

/// Löst Erwähnungen auf aktive User auf, die das Team sehen dürfen.
pub async fn resolve_mentions(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    team: TeamId,
    text: &str,
) -> AppResult<Vec<UserId>> {
    let mut out: Vec<UserId> = Vec::new();
    for handle in extract_handles(text) {
        for u in users_for_handle(tx, &handle).await? {
            if out.contains(&u.id) {
                continue;
            }
            let memberships = tx.teams().list_memberships_of_user(u.id).await?;
            let view = ActorView::new(
                Actor::User {
                    id: u.id,
                    role: u.role,
                },
                &memberships,
            );
            if compute_visible_team_ids(&view, &snap.tree).contains(&team) {
                out.push(u.id);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_handles_and_ignores_emails() {
        let h = extract_handles("Hi @alice, bitte mit @bob.k prüfen. mail me@acme.example @Carol.");
        assert!(h.contains("alice"));
        assert!(h.contains("bob.k"));
        assert!(h.contains("carol"));
        assert!(!h.contains("acme.example"));
    }

    #[test]
    fn finds_issue_identifiers() {
        let t = "Siehe ENG-123, web-7. Nicht: XENG-1x, FOO--3, a_ENG-5 und ENG-12345678901.";
        let ids = extract_identifiers(t);
        assert!(ids.contains("ENG-123"));
        assert!(ids.contains("WEB-7"));
        assert!(
            !ids.iter()
                .any(|i| i.contains("FOO") || i.contains("12345678901"))
        );
        let url = extract_identifiers("https://x.example/acme/issue/DES-4/title");
        assert!(url.contains("DES-4"));
    }

    #[test]
    fn code_and_paths_are_no_mentions() {
        let h = extract_handles(
            "`@bob` und ```\n@carol\n``` aber @dave, https://x.example/@erin a.@frank",
        );
        assert!(h.contains("dave"));
        assert!(!h.contains("bob") && !h.contains("carol"));
        assert!(!h.contains("erin") && !h.contains("frank"));
    }

    #[test]
    fn handle_prefers_display_name_and_needs_unique_name() {
        let users = [("dana", "Dana"), ("w3c-dana", "Dana"), ("erin", "Erin")];
        let pick = |h: &str| {
            pick_mention_matches(&users, h, |u| u.0, |u| u.1)
                .iter()
                .map(|u| u.0)
                .collect::<Vec<_>>()
        };
        assert_eq!(pick("dana"), ["dana"], "Anzeigename gewinnt");
        assert_eq!(pick("erin"), ["erin"]);
        assert_eq!(pick("DANA").len(), 1);
        let only_names = [("a", "Frank"), ("b", "Gina"), ("c", "Gina")];
        let p = |h: &str| pick_mention_matches(&only_names, h, |u| u.0, |u| u.1).len();
        assert_eq!(p("frank"), 1, "eindeutiger Name");
        assert_eq!(p("gina"), 0, "mehrdeutiger Name löst nichts auf");
    }

    #[test]
    fn trailing_dot_is_trimmed() {
        let h = extract_handles("cc @bob.");
        assert!(h.contains("bob"));
    }
}
