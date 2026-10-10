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

/// Alle `@handle`-Kandidaten eines Textes (klein geschrieben). Satzzeichen am Ende werden
/// zusätzlich abgeschnitten (`@alice.` → `alice`); ein `@` direkt hinter einem Wortzeichen
/// (E-Mail-Adresse) zählt nicht.
pub fn extract_handles(text: &str) -> BTreeSet<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = BTreeSet::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '@' && (i == 0 || !chars[i - 1].is_alphanumeric()) {
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

/// Löst Erwähnungen auf aktive User auf, die das Team sehen dürfen.
pub async fn resolve_mentions(
    tx: &mut dyn Tx,
    snap: &PolicySnapshot,
    team: TeamId,
    text: &str,
) -> AppResult<Vec<UserId>> {
    let mut out: Vec<UserId> = Vec::new();
    for handle in extract_handles(text) {
        for u in tx.users().find_by_name(&handle).await? {
            if !u.active || u.display_name != handle || out.contains(&u.id) {
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
    fn trailing_dot_is_trimmed() {
        let h = extract_handles("cc @bob.");
        assert!(h.contains("bob"));
    }
}
