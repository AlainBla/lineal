//! Umschreiben von Linear-Markdown nach Lineal (docs/13 §3.3) und Bereinigung.
//!
//! Importierte Inhalte gelten als nicht vertrauenswürdig: Nach dem Umschreiben läuft immer
//! `lineal_domain::rules::markdown::sanitize`, anschließend wird die Länge begrenzt.

use lineal_domain::rules::markdown::sanitize;
use regex::{Captures, Regex};
use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

fn re(p: &str) -> Regex {
    Regex::new(p).expect("static regex is valid")
}

static ISSUE_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"<issue\b[^>]*>([^<]*)</issue>"));
static TAG: LazyLock<Regex> = LazyLock::new(|| re(r"</?([A-Za-z][A-Za-z0-9-]*)"));
static IDENT: LazyLock<Regex> = LazyLock::new(|| re(r"^([A-Za-z][A-Za-z0-9]*)-(\d+)$"));

/// HTML-Elemente, die Lineal nach der Bereinigung behält/versteht; alles andere wird gemeldet.
const KNOWN_TAGS: &[&str] = &[
    "a",
    "b",
    "i",
    "u",
    "em",
    "strong",
    "code",
    "pre",
    "p",
    "br",
    "ul",
    "ol",
    "li",
    "blockquote",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "img",
    "table",
    "thead",
    "tbody",
    "tr",
    "td",
    "th",
    "del",
    "s",
    "sub",
    "sup",
    "details",
    "summary",
    "span",
    "div",
];

/// Elemente, die im Browser etwas ausführen/einbetten würden (Bericht: „entschärft“).
pub fn is_active_tag(t: &str) -> bool {
    matches!(
        t,
        "script"
            | "style"
            | "iframe"
            | "object"
            | "embed"
            | "svg"
            | "math"
            | "form"
            | "link"
            | "meta"
            | "base"
    )
}

/// Kontext des Umschreibens.
#[derive(Debug, Clone, Default)]
pub struct RewriteCtx {
    /// URL-Key des Linear-Workspace (`linear.app/<ws>/…`).
    pub source_ws: String,
    /// Basis-URL von Lineal (ohne `/` am Ende).
    pub target_base: String,
    /// URL-Key des Lineal-Workspace.
    pub target_ws: String,
    /// Linear-Team-Key -> Lineal-Team-Key (nur abweichende).
    pub key_map: HashMap<String, String>,
    /// Präfix der Lineal-Asset-URLs (`{base}/assets/`); importierter Text darf sie nie enthalten.
    pub asset_prefix: String,
    /// Linear-Benutzername (klein) -> Lineal-`display_name`.
    pub users: HashMap<String, String>,
}

/// Ergebnis des Umschreibens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rewritten {
    pub text: String,
    /// Unbekannte HTML-Elemente (für den Bericht).
    pub unknown_tags: BTreeSet<String>,
    /// Wurde wegen des Längenlimits gekürzt?
    pub truncated: bool,
}

impl RewriteCtx {
    fn map_identifier(&self, ident: &str) -> String {
        match IDENT.captures(ident.trim()) {
            Some(c) => {
                let key = c[1].to_uppercase();
                let key = self.key_map.get(&key).cloned().unwrap_or(key);
                format!("{key}-{}", &c[2])
            }
            None => ident.trim().to_owned(),
        }
    }

    /// Schreibt Linear-Elemente und -Links um, bereinigt und kürzt auf `max_chars`.
    pub fn rewrite(&self, md: &str, max_chars: usize) -> Rewritten {
        // Importierter Text darf keine Lineal-Asset-Schlüssel (und damit keine Blobs anderer
        // Objekte/Workspaces) einschleusen: solche Pfade werden entschärft.
        let md = if self.asset_prefix.is_empty() {
            std::borrow::Cow::Borrowed(md)
        } else {
            std::borrow::Cow::Owned(md.replace(self.asset_prefix.as_str(), "removed-asset-link:"))
        };
        let mut text = ISSUE_TAG
            .replace_all(&md, |c: &Captures| self.map_identifier(&c[1]))
            .into_owned();
        if !self.source_ws.is_empty() {
            let ws = regex::escape(&self.source_ws);
            // Mentions: [@Name](https://linear.app/<ws>/profiles/<username>)
            let mention = re(&format!(
                r"\[@?([^\]]*)\]\(https://linear\.app/{ws}/profiles/([^)\s]+)\)"
            ));
            text = mention
                .replace_all(&text, |c: &Captures| {
                    let username = c[2].trim_end_matches('/').to_lowercase();
                    match self.users.get(&username) {
                        Some(display) => format!("@{display}"),
                        None => format!("@{}", c[1].trim().to_lowercase().replace(' ', ".")),
                    }
                })
                .into_owned();
            // Issue-Links.
            let issue = re(&format!(
                r#"https://linear\.app/{ws}/issue/([A-Za-z][A-Za-z0-9]*-\d+)(?:/[^\s)\]">]*)?"#
            ));
            text = issue
                .replace_all(&text, |c: &Captures| {
                    format!(
                        "{}/{}/issue/{}",
                        self.target_base,
                        self.target_ws,
                        self.map_identifier(&c[1])
                    )
                })
                .into_owned();
            // Projekte, Dokumente, Initiativen, Views: Slug (12 Hex-Zeichen) bleibt erhalten.
            let other = re(&format!(
                r#"https://linear\.app/{ws}/(project|document|initiative|view)/([^\s)\]">/]+)(?:/[^\s)\]">]*)?"#
            ));
            text = other
                .replace_all(&text, |c: &Captures| {
                    let slug = c[2].rsplit('-').next().unwrap_or(&c[2]);
                    let slug = if slug.len() == 12 && slug.chars().all(|ch| ch.is_ascii_hexdigit())
                    {
                        slug.to_lowercase()
                    } else {
                        c[2].to_owned()
                    };
                    format!("{}/{}/{}/{slug}", self.target_base, self.target_ws, &c[1])
                })
                .into_owned();
        }
        let unknown_tags: BTreeSet<String> = TAG
            .captures_iter(&text)
            .map(|c| c[1].to_lowercase())
            .filter(|t| !KNOWN_TAGS.contains(&t.as_str()))
            .collect();
        let cleaned = sanitize(&text);
        let (text, truncated) = truncate_chars(&cleaned, max_chars);
        Rewritten {
            text,
            unknown_tags,
            truncated,
        }
    }
}

/// Kürzt auf `max` Zeichen (nicht Bytes).
pub fn truncate_chars(s: &str, max: usize) -> (String, bool) {
    if s.chars().count() <= max {
        return (s.to_owned(), false);
    }
    (s.chars().take(max).collect(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> RewriteCtx {
        RewriteCtx {
            asset_prefix: String::new(),
            source_ws: "acme".into(),
            target_base: "https://lineal.example.com".into(),
            target_ws: "acme".into(),
            key_map: HashMap::from([("ENG".to_owned(), "ENGX".to_owned())]),
            users: HashMap::from([("bob".to_owned(), "bob.b".to_owned())]),
        }
    }

    #[test]
    fn issue_tags_and_links_are_rewritten() {
        let out = ctx().rewrite(
            "See <issue id=\"abc\" href=\"https://linear.app/acme/issue/ENG-12/x\">ENG-12</issue> and \
             https://linear.app/acme/issue/WEB-3/some-slug and https://linear.app/other/issue/ENG-1/x",
            1000,
        );
        assert!(out.text.contains("See ENGX-12 and"), "{}", out.text);
        assert!(
            out.text
                .contains("https://lineal.example.com/acme/issue/WEB-3")
        );
        assert!(
            out.text.contains("https://linear.app/other/issue/ENG-1/x"),
            "foreign workspaces stay untouched"
        );
    }

    #[test]
    fn mentions_projects_and_documents() {
        let out = ctx().rewrite(
            "[@Bob](https://linear.app/acme/profiles/bob) see https://linear.app/acme/project/roadmap-0123456789ab \
             and https://linear.app/acme/document/spec-fedcba987654/overview",
            1000,
        );
        assert!(out.text.starts_with("@bob.b see"), "{}", out.text);
        assert!(out.text.contains("/acme/project/0123456789ab"));
        assert!(out.text.contains("/acme/document/fedcba987654"));
    }

    #[test]
    fn dangerous_content_is_sanitized_and_unknown_tags_reported() {
        let out = ctx().rewrite(
            "x <script>alert(1)</script> <linear-image>{\"a\":1}</linear-image> [t](javascript:alert(1)) <img src=x onerror=alert(1)>",
            1000,
        );
        // Kein ausführbares Markup mehr: jedes Tag ist mit `\` maskiert und bleibt als Text sichtbar.
        assert!(!out.text.contains(" <script") && out.text.contains("\\<script>alert(1)"));
        assert!(!out.text.contains("](javascript:"), "{}", out.text);
        assert!(
            out.text.contains("\\<img src=x onerror=alert(1)>"),
            "{}",
            out.text
        );
        assert!(out.unknown_tags.contains("linear-image"));
        assert!(out.unknown_tags.contains("script"));
    }

    #[test]
    fn many_placeholders_are_kept_verbatim() {
        let src = "Nutze <token> mit <uuid>, Datei <datei> auf <host-ipv4> (<e-mail>), Schema <https>://x und <script> als Wort. `<code-tag>` bleibt.";
        let out = ctx().rewrite(src, 10_000);
        for p in [
            "token",
            "uuid",
            "datei",
            "host-ipv4",
            "e-mail",
            "https",
            "script",
        ] {
            assert!(out.text.contains(&format!("\\<{p}>")), "{p}: {}", out.text);
            assert!(out.unknown_tags.contains(p), "{p}");
        }
        assert!(out.text.contains("`<code-tag>`"));
    }

    #[test]
    fn injected_lineal_asset_paths_are_neutralized() {
        let mut c = ctx();
        c.asset_prefix = "https://lineal.example.com/assets/".into();
        let out = c.rewrite(
            "![x](https://lineal.example.com/assets/other-org/aaa/bbb) <img src=\"https://lineal.example.com/assets/k\">",
            1000,
        );
        assert!(
            !out.text.contains("lineal.example.com/assets/"),
            "{}",
            out.text
        );
    }

    #[test]
    fn length_is_limited_by_characters() {
        let out = ctx().rewrite(&"ä".repeat(50), 10);
        assert_eq!(out.text.chars().count(), 10);
        assert!(out.truncated);
    }
}
