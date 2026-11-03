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
/// Von der Bereinigung maskierte Tag-Anfänge (`\<name`, im Notfall `&lt;name`).
static MASKED_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"(?:\\<|&lt;)/?([A-Za-z][A-Za-z0-9-]*)"));
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
    /// Linear-Kennung (`ENG-12`) -> Lineal-Kennung, wo die Nummer beim Import neu vergeben wurde.
    pub number_map: HashMap<String, String>,
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
        if let Some(mapped) = self.number_map.get(&ident.trim().to_uppercase()) {
            return mapped.clone();
        }
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
        // Kennungen im Fließtext (`ENG-12`): Team-Key und neu vergebene Nummer. Kennungen in Links
        // und `<issue>`-Elementen übernehmen die Schritte darunter (je Vorkommen genau einmal).
        let md = if self.number_map.is_empty() && self.key_map.is_empty() {
            md
        } else {
            std::borrow::Cow::Owned(remap_identifiers(&md, true, |ident| {
                let mapped = self.map_identifier(ident);
                (mapped != ident).then_some(mapped)
            }))
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
        let cleaned = sanitize(&text);
        // Gemeldet wird nur, was die Bereinigung tatsächlich maskiert hat: Autolinks
        // (`<https://…>`), Code und schon vorher maskierte Folgen bleiben unverändert.
        let masked = |s: &str| {
            let mut n = std::collections::BTreeMap::<String, usize>::new();
            for c in MASKED_TAG.captures_iter(s) {
                *n.entry(c[1].to_lowercase()).or_default() += 1;
            }
            n
        };
        let before = masked(&text);
        let unknown_tags: BTreeSet<String> = masked(&cleaned)
            .into_iter()
            .filter(|(t, n)| *n > before.get(t).copied().unwrap_or(0))
            .map(|(t, _)| t)
            .filter(|t| !KNOWN_TAGS.contains(&t.as_str()))
            .collect();
        let (text, truncated) = truncate_chars(&cleaned, max_chars);
        Rewritten {
            text,
            unknown_tags,
            truncated,
        }
    }
}

static BARE_IDENT: LazyLock<Regex> = LazyLock::new(|| re(r"[A-Z][A-Z0-9]*-[0-9]+"));

/// Ersetzt Issue-Kennungen (`ENG-12`) außerhalb von Code (Inline-Code und Codeblöcke) in einem
/// Durchgang: jedes Vorkommen wird höchstens einmal abgebildet. `in_prose_only` lässt Kennungen
/// in URL-Pfaden (`…/issue/ENG-12`) und direkt hinter `>` (Inhalt von `<issue>`-Elementen) aus.
pub fn remap_identifiers(
    text: &str,
    in_prose_only: bool,
    map: impl Fn(&str) -> Option<String>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fence: Option<String> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start_matches(' ');
        let marker: String = trimmed
            .chars()
            .take_while(|c| *c == '`' || *c == '~')
            .collect();
        if line.len() - trimmed.len() <= 3
            && marker.len() >= 3
            && marker
                .chars()
                .all(|c| c == marker.chars().next().unwrap_or('`'))
        {
            match &fence {
                Some(open) if marker.starts_with(open.as_str()) => fence = None,
                None => fence = Some(marker),
                _ => {}
            }
            out.push_str(line);
            continue;
        }
        if fence.is_some() {
            out.push_str(line);
            continue;
        }
        remap_line(line, in_prose_only, &map, &mut out);
    }
    out
}

/// Eine Zeile außerhalb eines Codeblocks: Inline-Code (Backtick-Läufe gleicher Länge) bleibt.
fn remap_line(
    line: &str,
    in_prose_only: bool,
    map: &impl Fn(&str) -> Option<String>,
    out: &mut String,
) {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut plain_start = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let run_start = i;
        while i < bytes.len() && bytes[i] == b'`' {
            i += 1;
        }
        let run = &line[run_start..i];
        // Schließender Lauf gleicher Länge?
        let mut j = i;
        let mut close = None;
        while j < bytes.len() {
            if bytes[j] == b'`' {
                let s = j;
                while j < bytes.len() && bytes[j] == b'`' {
                    j += 1;
                }
                if j - s == run.len() {
                    close = Some(j);
                    break;
                }
            } else {
                j += 1;
            }
        }
        if let Some(end) = close {
            remap_plain(&line[plain_start..run_start], in_prose_only, map, out);
            out.push_str(&line[run_start..end]);
            i = end;
            plain_start = end;
        }
    }
    remap_plain(&line[plain_start..], in_prose_only, map, out);
}

fn remap_plain(
    s: &str,
    in_prose_only: bool,
    map: &impl Fn(&str) -> Option<String>,
    out: &mut String,
) {
    let mut last = 0;
    for m in BARE_IDENT.find_iter(s) {
        let before = s[..m.start()].chars().next_back();
        let after = s[m.end()..].chars().next();
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let blocked_before = before
            .is_some_and(|c| word(c) || c == '-' || (in_prose_only && (c == '/' || c == '>')));
        if blocked_before || after.is_some_and(word) {
            continue;
        }
        if let Some(mapped) = map(m.as_str()) {
            out.push_str(&s[last..m.start()]);
            out.push_str(&mapped);
            last = m.end();
        }
    }
    out.push_str(&s[last..]);
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
            number_map: HashMap::new(),
        }
    }

    #[test]
    fn renumbered_identifiers_are_mapped_once_in_text_links_and_tags() {
        let mut c = ctx();
        // ENG-1 wurde zu ENGX-12, ENG-12 zu ENGX-40: keine Kette (ENG-1 darf nie ENGX-40 werden).
        c.number_map = HashMap::from([
            ("ENG-1".to_owned(), "ENGX-12".to_owned()),
            ("ENG-12".to_owned(), "ENGX-40".to_owned()),
        ]);
        let out = c.rewrite(
            "Siehe ENG-1 und [ENG-12](https://linear.app/acme/issue/ENG-12/slug), \
             <issue id=\"a\" href=\"https://linear.app/acme/issue/ENG-1/x\">ENG-1</issue>, ENG-2, \
             `ENG-1` und ENG-1a.\n```\nENG-1\n```\nENG-1-Retry",
            10_000,
        );
        assert_eq!(
            out.text,
            "Siehe ENGX-12 und [ENGX-40](https://lineal.example.com/acme/issue/ENGX-40), \
             ENGX-12, ENGX-2, `ENG-1` und ENG-1a.\n```\nENG-1\n```\nENGX-12-Retry"
        );
    }

    #[test]
    fn remap_identifiers_maps_every_occurrence_once_outside_code() {
        let map = HashMap::from([
            ("KOL-1".to_owned(), "KOL-7".to_owned()),
            ("KOL-7".to_owned(), "KOL-9".to_owned()),
        ]);
        let out = remap_identifiers(
            "KOL-1 [KOL-1](https://x/ws/issue/KOL-1) KOL-7 ``KOL-1`` XKOL-1 RUSTSEC-2026-0373",
            false,
            |i| map.get(i).cloned(),
        );
        assert_eq!(
            out,
            "KOL-7 [KOL-7](https://x/ws/issue/KOL-7) KOL-9 ``KOL-1`` XKOL-1 RUSTSEC-2026-0373"
        );
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
    fn unchanged_markup_is_not_reported() {
        // Autolinks, Code und bereits maskierte Folgen bleiben unverändert: kein Berichtseintrag.
        let src = "Siehe <https://example.com/a?b=1>, <http://x.de> und <mailto:a@b.de>. \
                   `<token>` inline\n\n```\n<config>\n<script>x</script>\n```\n\n\\<schon> maskiert";
        let out = ctx().rewrite(src, 10_000);
        assert!(
            out.text.contains("<https://example.com/a?b=1>"),
            "{}",
            out.text
        );
        assert!(out.text.contains("<mailto:a@b.de>"), "{}", out.text);
        assert!(out.unknown_tags.is_empty(), "{:?}", out.unknown_tags);
        // Im selben Text zusätzlich ein echter Platzhalter: nur dieser wird gemeldet.
        let out = ctx().rewrite(&format!("{src} und <kunde>"), 10_000);
        assert_eq!(out.unknown_tags.into_iter().collect::<Vec<_>>(), ["kunde"]);
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
