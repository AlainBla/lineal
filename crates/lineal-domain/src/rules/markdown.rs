//! Markdown-Sanitizing (docs/00 §6, docs/07 §2.8): Markdown ist das Austauschformat für alle
//! Rich-Text-Felder, Eingaben stammen von Nutzern, Agenten und Integrationen.
//!
//! * [`sanitize`] liefert wieder Markdown: rohes HTML wird nie ausgeführt, aber auch nie
//!   gelöscht. Jede Tag-artige Folge (`<token>`, `<uuid>`, `<script>`, `<img onerror=…>`) wird als
//!   wörtlicher Text maskiert (`\<`) und bleibt lesbar; Linear-Texte enthalten solche Platzhalter
//!   als normalen Text. Links/Bilder mit unsicherem Schema werden zu reinem Text. Alles andere bleibt Byte für Byte
//!   erhalten (Allowlist = Markdown-Syntax plus Schemata `http`, `https`, `mailto` und
//!   relative Ziele).
//! * [`render_html`] rendert Markdown zu HTML und filtert es zusätzlich mit der
//!   `ammonia`-Allowlist (z. B. für E-Mail-Benachrichtigungen).

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::collections::HashSet;
use std::ops::Range;

/// Erlaubte URL-Schemata für Links und Bilder (relative Ziele sind immer erlaubt).
const SAFE_SCHEMES: [&str; 3] = ["http", "https", "mailto"];

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

/// Ist das Link-/Bildziel unbedenklich? (Schema-Allowlist; Steuerzeichen und Leerraum werden
/// wie im Browser ignoriert, damit `java\tscript:` nicht durchrutscht.)
pub fn is_safe_url(url: &str) -> bool {
    let cleaned: String = url
        .chars()
        .filter(|c| !c.is_control() && !c.is_whitespace())
        .collect();
    let Some(colon) = cleaned.find(':') else {
        return true;
    };
    // Ein Doppelpunkt nach '/', '?' oder '#' gehört zu Pfad/Query/Fragment, nicht zu einem Schema.
    if cleaned[..colon].contains(['/', '?', '#']) {
        return true;
    }
    SAFE_SCHEMES.contains(&cleaned[..colon].to_ascii_lowercase().as_str())
}

/// Maskiert jedes `<`, das ein Tag, Ende-Tag, Kommentar oder eine Processing-Instruction
/// beginnen könnte, mit `\`. Der Text bleibt erhalten, wird aber nicht mehr als HTML geparst.
fn escape_html_as_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 8);
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<'
            && chars
                .peek()
                .is_some_and(|n| n.is_ascii_alphabetic() || matches!(n, '/' | '!' | '?'))
        {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;")
}

/// Maximale Zahl der Bereinigungsdurchläufe bis zum Fixpunkt.
const MAX_PASSES: usize = 8;

/// Bereinigt Markdown (siehe Moduldoku). Läuft bis zum Fixpunkt, weil das Entfernen von Tags
/// neue Konstrukte erzeugen kann (`<scr<script>ipt>` oder Markdown-Links in HTML-Blöcken);
/// dadurch ist das Ergebnis idempotent, harmloses Markdown bleibt unverändert.
pub fn sanitize(md: &str) -> String {
    let mut current = md.to_owned();
    for _ in 0..MAX_PASSES {
        match sanitize_pass(&current) {
            Some(next) => current = next,
            None => return current,
        }
    }
    // Kein Fixpunkt (pathologische Eingabe): alle spitzen Klammern neutralisieren.
    current.replace('<', "&lt;")
}

/// Wie [`sanitize`] für **einfachen Text** (Titel, Namen, Kurzbeschreibungen, Linkbezeichnungen):
/// unsichere Links verlieren ihr Ziel, Tag-artige Folgen (`<token>`) bleiben als Text erhalten,
/// aber ohne Markdown-Maskierung (`<` statt `\<`), weil das Feld nicht als Markdown dargestellt
/// wird. Die Anzeige muss einfachen Text immer als Text ausgeben (kein `innerHTML`).
pub fn sanitize_plain(text: &str) -> String {
    sanitize(text).replace("\\<", "<")
}

/// Ein Durchlauf; `None`, wenn nichts zu ändern war.
fn sanitize_pass(md: &str) -> Option<String> {
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    // Aktives unsicheres Link-/Bild-Element: (Gesamtbereich, Tiefe, gesammelter Text).
    let mut open: Option<(Range<usize>, usize, String)> = None;

    for (event, range) in Parser::new_ext(md, options()).into_offset_iter() {
        if let Some((_, depth, text)) = open.as_mut() {
            match event {
                Event::Start(Tag::Link { .. } | Tag::Image { .. }) => *depth += 1,
                Event::End(TagEnd::Link | TagEnd::Image) => {
                    *depth -= 1;
                    if *depth == 0 {
                        if let Some((full, _, text)) = open.take() {
                            edits.push((full, escape_text(&text)));
                        }
                    }
                }
                Event::Text(t) | Event::Code(t) => text.push_str(&t),
                Event::SoftBreak | Event::HardBreak => text.push(' '),
                _ => {}
            }
            continue;
        }
        match event {
            Event::Html(_) | Event::InlineHtml(_) => {
                let cleaned = escape_html_as_text(&md[range.clone()]);
                if cleaned != md[range.clone()] {
                    edits.push((range, cleaned));
                }
            }
            Event::Start(Tag::Link { ref dest_url, .. } | Tag::Image { ref dest_url, .. })
                if !is_safe_url(dest_url) =>
            {
                open = Some((range, 1, String::new()));
            }
            _ => {}
        }
    }

    if edits.is_empty() {
        return None;
    }
    let mut out = md.to_owned();
    for (range, replacement) in edits.into_iter().rev() {
        out.replace_range(range, &replacement);
    }
    Some(out)
}

/// Rendert Markdown zu sicherem HTML: rohes HTML wird zu Text, das Ergebnis läuft durch die
/// `ammonia`-Allowlist (entfernt Skripte, Event-Handler und unsichere URL-Schemata).
pub fn render_html(md: &str) -> String {
    let events = Parser::new_ext(md, options()).map(|e| match e {
        Event::Html(t) | Event::InlineHtml(t) => Event::Text(t),
        other => other,
    });
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, events);
    // Allowlist erst NACH dem Rendern auf das fertige HTML anwenden.
    ammonia::Builder::default()
        .url_schemes(HashSet::from(SAFE_SCHEMES))
        .link_rel(Some("noopener noreferrer"))
        .clean(&html)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harmless_markdown_is_byte_identical() {
        let doc = "# Titel\n\nText mit **fett**, _kursiv_, `code` und ~~weg~~.\n\n\
                   - [ ] offen\n- [x] erledigt\n\n1. eins\n2. zwei\n\n\
                   > Zitat von @alice zu ENG-123\n\n\
                   | a | b |\n|---|---|\n| 1 | 2 |\n\n\
                   ```rust\nlet x = \"<script>alert(1)</script>\";\n```\n\n\
                   [Doku](https://acme.example/docs) und <https://acme.example> und ![Bild](/files/a.png)\n\
                   Mail: [Bob](mailto:bob@acme.example), Pfad [x](/a:b), [y](#frag:1), [z](page.html?q=a:b)\n\
                   Entity: &lt;script&gt; &amp; fertig\n";
        assert_eq!(sanitize(doc), doc);
    }

    /// Gibt es ein `<`, das nicht mit `\` maskiert ist und ein Tag beginnen würde?
    fn has_raw_tag(s: &str) -> bool {
        let b: Vec<char> = s.chars().collect();
        b.iter().enumerate().any(|(i, c)| {
            *c == '<'
                && b.get(i + 1)
                    .is_some_and(|n| n.is_ascii_alphabetic() || matches!(n, '/' | '!' | '?'))
                && (i == 0 || b[i - 1] != '\\')
        })
    }

    #[test]
    fn raw_html_tags_become_literal_text() {
        assert_eq!(sanitize("a <b>fett</b> b"), "a \\<b>fett\\</b> b");
        assert_eq!(
            sanitize("x <img src=x onerror=alert(1)> y"),
            "x \\<img src=x onerror=alert(1)> y"
        );
        assert_eq!(sanitize("a <!-- geheim --> b"), "a \\<!-- geheim --> b");
        let out = sanitize("<div>\nhallo\n</div>\n\nrest\n");
        assert!(!has_raw_tag(&out), "{out:?}");
        assert!(out.contains("hallo") && out.contains("rest") && out.contains("div"));
    }

    #[test]
    fn plain_text_keeps_placeholders_without_markdown_escapes() {
        assert_eq!(
            sanitize_plain("Fix <uuid> in <script>x</script>"),
            "Fix <uuid> in <script>x</script>"
        );
        assert_eq!(sanitize_plain("[a](javascript:x) b"), "a b");
        // Der Wert aus einem früheren Speichern (mit `\<`) bleibt stabil.
        assert_eq!(sanitize_plain("T \\<x>"), "T <x>");
    }

    #[test]
    fn placeholders_survive_as_text() {
        let md = "Setze <token> und <uuid> in <datei> auf <host-ipv4>, mail an <e-mail> via <https> \
                  oder <script>.";
        let out = sanitize(md);
        assert!(!has_raw_tag(&out), "{out:?}");
        for p in [
            "token",
            "uuid",
            "datei",
            "host-ipv4",
            "e-mail",
            "https",
            "script",
        ] {
            assert!(out.contains(&format!("\\<{p}>")), "{p}: {out:?}");
        }
        // Kein Verlust beim Rendern: Text erscheint maskiert im HTML.
        let html = render_html(&out);
        assert!(
            html.contains("&lt;token&gt;") && html.contains("&lt;script&gt;"),
            "{html}"
        );
        assert!(
            !html.contains("<token") && !html.contains("<script"),
            "{html}"
        );
        // Nicht tag-artige spitze Klammern bleiben unberührt.
        assert_eq!(
            sanitize("a < b und 1<2 und x -> y"),
            "a < b und 1<2 und x -> y"
        );
    }

    #[test]
    fn script_blocks_cannot_survive() {
        for md in [
            "<script>alert(1)</script>",
            "text\n\n<script>\nalert(1)\n</script>\n",
            "<scr<script>ipt>alert(1)</scr</script>ipt>",
            "<iframe src=\"https://evil.example\"></iframe>",
            "<a href=\"javascript:alert(1)\">x</a>",
            "<svg onload=alert(1)>",
        ] {
            let out = sanitize(md);
            // `<scr<script>` ist kein Tag (nur Text); entscheidend ist, was gerendert wird.
            let html = render_html(&out).to_lowercase();
            assert!(
                !html.contains("<script")
                    && !html.contains("<iframe")
                    && !html.contains("<svg")
                    && !html.contains("<a href"),
                "{md:?} -> {html}"
            );
        }
    }

    #[test]
    fn unsafe_link_schemes_become_plain_text() {
        assert_eq!(sanitize("[klick](javascript:alert(1))"), "klick");
        assert_eq!(sanitize("a [klick](JaVaScRiPt:alert(1)) b"), "a klick b");
        // Kein gültiger Link (Tab im Ziel): bleibt Text und damit harmlos.
        assert!(!render_html(&sanitize("[x](java\tscript:alert(1))")).contains("href"));
        assert_eq!(sanitize("[x](&#106;avascript:alert(1))"), "x");
        assert_eq!(sanitize("[x](vbscript:msgbox)"), "x");
        assert_eq!(sanitize("![alt](data:image/svg+xml;base64,AAAA)"), "alt");
        assert_eq!(sanitize("[**fett** `c`](file:///etc/passwd)"), "fett c");
        assert_eq!(sanitize("<javascript:alert(1)>"), "javascript:alert(1)");
        // Verschachtelt: Bild im Link, beide unsicher.
        assert_eq!(sanitize("[![a](data:x)](javascript:y)"), "a");
        // Sicheres bleibt.
        assert_eq!(
            sanitize("[x](https://ok.example)"),
            "[x](https://ok.example)"
        );
    }

    #[test]
    fn replacement_text_cannot_reintroduce_html() {
        let out = sanitize("[&lt;script&gt;alert(1)&lt;/script&gt;](javascript:x)");
        assert!(!has_raw_tag(&out), "{out:?}");
    }

    #[test]
    fn sanitize_is_idempotent() {
        for md in [
            "a <b>x</b> [l](javascript:1) ![i](data:x) <script>1</script>",
            "<div>\n[a](javascript:b)\n</div>",
            "& < > &amp; &lt;",
            "",
        ] {
            let once = sanitize(md);
            assert_eq!(sanitize(&once), once, "{md:?}");
        }
    }

    #[test]
    fn code_spans_and_blocks_are_untouched() {
        let md = "`<b>x</b>` und\n\n    <script>alert(1)</script>\n";
        assert_eq!(sanitize(md), md);
    }

    #[test]
    fn url_safety() {
        for ok in [
            "https://a.example",
            "HTTP://a.example",
            "mailto:a@b.example",
            "/pfad",
            "page.html",
            "#anker",
            "?q=1",
            "./a:b",
            "",
        ] {
            assert!(is_safe_url(ok), "{ok:?}");
        }
        for bad in [
            "javascript:alert(1)",
            " JAVASCRIPT:alert(1)",
            "java\nscript:alert(1)",
            "data:text/html,<b>",
            "vbscript:x",
            "file:///x",
            "ftp://a",
            "tel:+4930123",
        ] {
            assert!(!is_safe_url(bad), "{bad:?}");
        }
    }

    #[test]
    fn render_html_is_allowlisted() {
        let h = render_html("# Hi\n\n**fett** [l](https://a.example) <script>alert(1)</script>");
        assert!(h.contains("<h1>Hi</h1>"), "{h}");
        assert!(h.contains("<strong>fett</strong>"), "{h}");
        assert!(h.contains("href=\"https://a.example\""), "{h}");
        assert!(h.contains("noopener"), "{h}");
        assert!(!h.contains("<script"), "{h}");

        let h = render_html(
            "[x](javascript:alert(1)) ![i](data:text/html;base64,AAAA) <img src=x onerror=alert(1)>",
        );
        assert!(!h.contains("javascript:"), "{h}");
        assert!(
            !h.contains("onerror=\"") && !h.contains("<img src=\"x\" onerror"),
            "{h}"
        );
        assert!(!h.contains("data:"), "{h}");
        let h = render_html("```\n<b>x</b>\n```");
        assert!(h.contains("&lt;b&gt;x&lt;/b&gt;"), "{h}");
    }

    #[test]
    fn render_html_bypass_attempts() {
        let attacks = [
            "[a](javascript:alert(1))",
            "[a](JaVaScRiPt:alert(1))",
            "[a](&#106;avascript:alert(1))",
            "[a](&#x6A;avascript:alert(1))",
            "[a](  javascript:alert(1))",
            "[a](vbscript:msgbox(1))",
            "[a](data:text/html;base64,PHNjcmlwdD4=)",
            "![i](javascript:alert(1))",
            "![i](data:image/svg+xml;base64,PHN2Zz4=)",
            "<javascript:alert(1)>",
            "[a][r]\n\n[r]: javascript:alert(1)",
            "[r]\n\n[r]: data:text/html,x",
            "<a href=\"javascript:alert(1)\">x</a>",
            "<a href=\" javascript:alert(1)\">x</a>",
            "<a href=\"jav&#x09;ascript:alert(1)\">x</a>",
            "<img src=x onerror=alert(1)>",
            "<p style=\"background:url(javascript:alert(1))\" onclick=alert(1)>x</p>",
            "<svg><script>alert(1)</script></svg>",
            "<svg onload=alert(1)>",
            "<math><mi xlink:href=\"javascript:alert(1)\">x</mi></math>",
            "<iframe src=\"https://evil.example\"></iframe>",
            "<form action=\"javascript:alert(1)\"><button>x</button></form>",
            "<object data=\"data:text/html,x\"></object>",
            "<script>alert(1)</script>",
            "<style>*{x:expression(alert(1))}</style>",
        ];
        for md in attacks {
            let h = render_html(md).to_lowercase();
            for bad in [
                "<a href=\"javascript:",
                "<a href=\"data:",
                "<img src=\"data:",
                "<img src=\"javascript:",
                "<a href=\"vbscript:",
                "<script",
                "<svg",
                "<math",
                "<iframe",
                "<style",
                "<form",
                "<object",
            ] {
                assert!(!h.contains(bad), "{md:?} -> {h} (contains {bad})");
            }
            // Keine Event-Handler/Styles in echten (unescapten) Tags.
            for tag in h.split('<').skip(1) {
                let tag = tag.split('>').next().unwrap_or("");
                assert!(
                    !tag.contains(" on") && !tag.contains("style="),
                    "{md:?} -> {h}"
                );
            }
            let clean = sanitize(md).to_lowercase();
            assert!(!has_raw_tag(&clean), "{md:?} -> {clean}");
        }
        // Erlaubt bleiben http, https, mailto und relative Ziele; tel nicht mehr.
        let h =
            render_html("[a](https://x.example) [b](mailto:a@b.example) [c](/rel) [d](tel:123)");
        assert!(
            h.contains("https://x.example")
                && h.contains("mailto:a@b.example")
                && h.contains("/rel")
        );
        assert!(!h.contains("tel:"), "{h}");
        assert!(h.contains("rel=\"noopener noreferrer\""), "{h}");
        assert!(!is_safe_url("tel:123"));
        // URL-Encoding des Schemas ist kein Schema und bleibt unwirksam.
        assert!(!is_safe_url("java%73cript:alert(1)"));
        assert!(!render_html("[a](java%73cript:alert(1))").contains("javascript"));
    }
}
