//! Markdown-Sanitizing (docs/00 §6, docs/07 §2.8): Markdown ist das Austauschformat für alle
//! Rich-Text-Felder, Eingaben stammen von Nutzern, Agenten und Integrationen.
//!
//! * [`sanitize`] liefert wieder Markdown: rohes HTML wird nie ausgeführt, aber auch nie
//!   gelöscht. Jede Tag-artige Folge (`<token>`, `<uuid>`, `<script>`, `<img onerror=…>`) wird als
//!   wörtlicher Text maskiert (`\<`) und bleibt lesbar; Linear-Texte enthalten solche Platzhalter
//!   als normalen Text. Links/Bilder mit unsicherem Schema werden zu reinem Text. Alles andere bleibt Byte für Byte
//!   erhalten (Allowlist = Markdown-Syntax plus Schemata `http`, `https`, `mailto` und
//!   relative Ziele).
//!   Zusätzlich parserunabhängig (FUZZ-01/INJECT-01): `javascript:`/`vbscript:`/`data:` direkt nach
//!   `(`, `<`, `=` oder `[x]:` erhalten `%3A` statt `:` (auch nach Entity-/Steuerzeichen-
//!   Normalisierung), und `<` + Buchstabe/`/`/`!`/`?` in Text, den pulldown-cmark nicht als HTML
//!   liest, wird maskiert. Code-Spans/-Blöcke bleiben unverändert, soweit sie das Muster nicht
//!   selbst enthalten.
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
    let mut in_code_block = false;

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
            // Was pulldown-cmark als Text liest, kann ein HTML5-Tokenizer oder ein anderer
            // Markdown-Parser als Tag lesen (`<img/src=x/onerror=…>`, unterminierte Tags, INJECT-01).
            Event::Start(Tag::CodeBlock(_)) => in_code_block = true,
            Event::End(TagEnd::CodeBlock) => in_code_block = false,
            Event::Text(_) if !in_code_block => mask_stray_tag_starts(md, range, &mut edits),
            _ => {}
        }
    }

    let mut out = md.to_owned();
    for (range, replacement) in edits.into_iter().rev() {
        out.replace_range(range, &replacement);
    }
    // Parserunabhängige Nachbehandlung (FUZZ-01): was andere CommonMark-Renderer als Link lesen.
    let out = neutralize_unsafe_schemes(&out);
    (out != md).then_some(out)
}

/// Maskiert `<` mit folgendem Buchstaben, `/`, `!` oder `?` in Textstücken, die pulldown-cmark
/// nicht als HTML erkannt hat, soweit das `<` nicht schon per Backslash maskiert ist.
fn mask_stray_tag_starts(md: &str, range: Range<usize>, edits: &mut Vec<(Range<usize>, String)>) {
    let bytes = md.as_bytes();
    for pos in range.clone() {
        if bytes[pos] != b'<' {
            continue;
        }
        let tag_like = bytes
            .get(pos + 1)
            .is_some_and(|n| n.is_ascii_alphabetic() || matches!(n, b'/' | b'!' | b'?'));
        let backslashes = bytes[..pos]
            .iter()
            .rev()
            .take_while(|b| **b == b'\\')
            .count();
        if tag_like && backslashes % 2 == 0 {
            edits.push((pos..pos, "\\".to_owned()));
        }
    }
}

/// Endet `before` auf einen URL-Attributnamen (`href`, `src`, …)? Ein beliebiges `=` (etwa
/// `?q=data:1` in einer URL) löst die Entschärfung nicht aus.
fn url_attribute_before(before: &str) -> bool {
    let name: String = before
        .trim_end()
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-'))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    matches!(
        name.to_ascii_lowercase().as_str(),
        "href" | "xlink:href" | "src" | "action" | "formaction" | "data" | "poster" | "background"
    )
}

/// Unsichere Schemata, die in keinem Link-Ziel überleben dürfen.
const BLOCKED_SCHEMES: [&str; 3] = ["javascript", "vbscript", "data"];

/// Liest ab `i` ein Zeichen so, wie ein Browser es nach dem Entity-Dekodieren eines Attributs
/// sieht: numerische Entities sowie `&Tab;`, `&NewLine;`, `&colon;`. Liefert Zeichen und Länge.
fn norm_char(s: &str, i: usize) -> Option<(char, usize)> {
    let rest = &s[i..];
    let c = rest.chars().next()?;
    if c != '&' {
        return Some((c, c.len_utf8()));
    }
    for (name, ch) in [("&Tab;", '\t'), ("&NewLine;", '\n'), ("&colon;", ':')] {
        if rest.len() >= name.len()
            && rest.as_bytes()[..name.len()].eq_ignore_ascii_case(name.as_bytes())
        {
            return Some((ch, name.len()));
        }
    }
    if let Some(num) = rest.strip_prefix("&#") {
        let (digits, radix) = match num.strip_prefix(['x', 'X']) {
            Some(h) => (h, 16),
            None => (num, 10),
        };
        let end = digits
            .find(|c: char| !c.is_digit(radix))
            .unwrap_or(digits.len());
        if (1..=8).contains(&end) {
            let value = u32::from_str_radix(&digits[..end], radix).ok()?;
            let used = 2 + (num.len() - digits.len()) + end;
            let used = used + usize::from(digits[end..].starts_with(';'));
            return Some((char::from_u32(value).unwrap_or('\u{fffd}'), used));
        }
    }
    Some(('&', 1))
}

/// Zeichen, die Browser in URLs ignorieren (Tab, Zeilenumbrüche, übrige Steuerzeichen).
fn url_ignorable(c: char) -> bool {
    c.is_control()
}

/// Versucht ab `start` ein unsicheres Schema samt `:` zu lesen (ignoriert Steuerzeichen und
/// Entities dazwischen). Liefert das Schema und das Ende des gelesenen Bereichs.
fn match_blocked_scheme(s: &str, start: usize) -> Option<(&'static str, usize)> {
    'scheme: for name in BLOCKED_SCHEMES {
        let mut i = start;
        for expected in name.chars() {
            loop {
                let (c, len) = norm_char(s, i).filter(|_| i < s.len())?;
                i += len;
                if url_ignorable(c) {
                    continue;
                }
                if c.to_ascii_lowercase() == expected {
                    break;
                }
                continue 'scheme;
            }
        }
        loop {
            let Some((c, len)) = norm_char(s, i).filter(|_| i < s.len()) else {
                continue 'scheme;
            };
            i += len;
            if url_ignorable(c) {
                continue;
            }
            if c == ':' {
                return Some((name, i));
            }
            continue 'scheme;
        }
    }
    None
}

/// Entschärft `javascript:`, `vbscript:` und `data:` an Stellen, an denen ein CommonMark-Renderer
/// ein Link-Ziel lesen könnte: nach `(`, `<`, `=` (`href=`) und nach der Referenzdefinition `[x]:`. Der
/// Doppelpunkt wird zu `%3A` (aus `javascript:x` wird das relative Ziel `javascript%3Ax`; ein
/// Entity wie `&#58;` würde der Browser im Attribut wieder zum Doppelpunkt dekodieren).
/// Unabhängig davon, ob pulldown-cmark die Stelle als Link, Text, Code oder Kommentar liest.
fn neutralize_unsafe_schemes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let c = s[i..].chars().next().unwrap_or('\0');
        out.push(c);
        i += c.len_utf8();
        let trigger = match c {
            // `=` fängt `href=javascript:` in Tags, die ein Renderer nach einem verschluckten `\\` liest.
            '(' | '<' => true,
            '=' => url_attribute_before(&s[..i - 1]),
            ':' => s[..i - 1]
                .trim_end_matches(|c: char| c.is_whitespace() || c.is_control())
                .ends_with(']'),
            _ => false,
        };
        if !trigger {
            continue;
        }
        // Leerraum, Steuerzeichen und Entity-Leerraum zwischen Auslöser und Schema bleiben stehen.
        let mut j = i;
        if c == '=' && s[j..].starts_with(['"', '\'']) {
            j += 1;
        }
        while j < s.len() {
            match norm_char(s, j) {
                Some((ch, len)) if ch.is_whitespace() || url_ignorable(ch) => j += len,
                _ => break,
            }
        }
        if let Some((name, end)) = match_blocked_scheme(s, j) {
            out.push_str(&s[i..j]);
            out.push_str(name);
            out.push_str("%3A");
            i = end;
        }
    }
    out
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
        // Tab im Ziel: Browser ignorieren ihn, daher wird das Schema entschärft (relatives Ziel).
        let html = render_html(&sanitize("[x](java\tscript:alert(1))")).replace("%3A", "");
        assert!(!html.contains("javascript:"), "{html}");
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

    /// Kein Link-Ziel mit unsicherem Schema in der Ausgabe (nach Entfernen von Leerraum/Steuerzeichen
    /// und Dekodieren der üblichen Entities), egal wie ein Renderer sie lesen würde.
    fn has_active_scheme(out: &str) -> bool {
        let flat: String = out
            .replace("&#58;", ":")
            .replace("&#106;", "j")
            .chars()
            .filter(|c| !c.is_whitespace() && !c.is_control())
            .collect::<String>()
            .to_ascii_lowercase();
        ["javascript:", "vbscript:", "data:"].iter().any(|s| {
            ["(", "<", "]:", "(<"]
                .iter()
                .any(|p| flat.contains(&format!("{p}{s}")))
        })
    }

    #[test]
    fn parser_differential_examples_are_neutralized() {
        // FUZZ-01 (marked liest diese als Link, pulldown-cmark nicht) und INJECT-01-Referenzen.
        for md in [
            "[](javascript:()",
            "a\r<javascript:>",
            "?\n```<javascript:>",
            "[r]:\x0c<javascript:>",
            "[r]: javascript:alert(8)\n\n[e][r]",
            "http://a\\<vbscript:>",
            "[data:](<javascript:alert(1)>[r]: ",
            "[x](  java\tscript:alert(1)",
            "[x](&#106;avascript&#58;alert(1)",
            "[x](&#x6A;avascript&colon;alert(1)",
            "[x](\n&Tab;JAVASCRIPT:alert(1)",
            "[x]:\n  vbscript:msgbox",
            "[x](data:text/html;base64,AAAA",
            "![i](<data:image/svg+xml;base64,AAAA",
            "[a]( ( javascript:alert(1) ",
            "https://a.b\\<a href=JavaScript:alert(1)>x",
            "<a href='vbscript:x'>",
        ] {
            let out = sanitize(md);
            assert!(!has_active_scheme(&out), "{md:?} -> {out:?}");
            assert_eq!(sanitize(&out), out, "nicht idempotent: {md:?}");
        }
        assert_eq!(sanitize("[](javascript:()"), "[](javascript%3A()");
        assert_eq!(sanitize("[r]: javascript:x"), "[r]: javascript%3Ax");
    }

    #[test]
    fn html5_only_tags_are_masked() {
        // INJECT-01: pulldown-cmark liest diese als Text, ein HTML5-Tokenizer als Tag.
        for md in [
            "<img/src=x/onerror=alert(1)>",
            "<svg/onload=alert(1)>",
            "<img src=\"x\"onerror=alert(1)>",
            "<img src=`x` onerror=alert(1)>",
            "<img src=x onerror=alert(1) \n/ >",
            "text <img src=x onerror=alert(1) \n\n> more",
            "<img src=x onerror=alert(1)//",
            "<body/onload=alert(1)>",
            "<iframe/src=//evil>",
            "<input/autofocus/onfocus=alert(1)>",
            "<math/href=javascript:1>",
        ] {
            let out = sanitize(md);
            assert!(!has_raw_tag(&out), "{md:?} -> {out:?}");
            assert_eq!(sanitize(&out), out, "nicht idempotent: {md:?}");
        }
        // Schon maskiertes und Codebereiche bleiben, ebenso nicht tag-artige Klammern.
        for stable in [
            "\\<img/src=x>",
            "`<img/src=x>`",
            "```\n<img/src=x onerror=1>\n```",
            "a < b, 1<2, x<=y",
        ] {
            assert_eq!(sanitize(stable), stable);
        }
        // Doppelter Backslash maskiert nur sich selbst: das `<` bleibt gefährlich und wird maskiert.
        assert_eq!(sanitize("\\\\<img/src=x>"), "\\\\\\<img/src=x>");
    }

    #[test]
    fn harmless_schemes_in_prose_and_code_stay() {
        for md in [
            "Das Schema data ist ok, javascript auch: kein Link",
            "`javascript:void(0)` im Code",
            "[a](https://x.example/?q=data:1) und (data) sowie (data kein Ziel)",
            "Datei: data.csv, Metadaten: (javascript)",
        ] {
            assert_eq!(sanitize(md), md);
        }
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
