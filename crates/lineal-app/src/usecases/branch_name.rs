//! Git-Branchname eines Issues: einzige Quelle für MCP (`gitBranchName`) und GraphQL
//! (`Issue.branchName`). Format: `<handle>/<kennung-klein>-<titel-slug>`.

use crate::app::App;
use crate::context::RequestCtx;

/// Handle, wenn der Aufrufer nicht aufgelöst werden kann.
const FALLBACK_HANDLE: &str = "lineal";

/// Titel-Slug: kleingeschrieben, Nicht-Alphanumerisches wird zu `-`, höchstens 50 Bytes.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 50 {
            break;
        }
    }
    out.trim_matches('-').to_owned()
}

/// `handle/eng-12-titel-als-slug`.
pub fn git_branch_name(handle: &str, identifier: &str, title: &str) -> String {
    format!("{handle}/{}-{}", identifier.to_lowercase(), slug(title))
}

impl App {
    /// Branch-Präfix des Aufrufers (sein Anzeigename); `lineal`, wenn er nicht auflösbar ist.
    pub async fn branch_handle(&self, ctx: &RequestCtx) -> String {
        match self.get_user(ctx, "me").await {
            Ok(u) => u.user.display_name,
            Err(_) => FALLBACK_HANDLE.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_prefixed_lowercase_name_with_unicode_slug() {
        assert_eq!(
            git_branch_name("alain.backs-it", "KOL-243", "Zugriff für Dirk!"),
            "alain.backs-it/kol-243-zugriff-für-dirk"
        );
    }

    #[test]
    fn slug_truncates_and_trims() {
        assert!(slug(&"a ".repeat(60)).len() <= 50);
        assert_eq!(slug("--x--"), "x");
    }
}
