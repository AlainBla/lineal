//! Brücke zwischen Markdown und dem ProseMirror-Baum, den TipTap (`y-prosemirror`) in einem
//! `Y.XmlFragment` ablegt. Der Server muss den Baum selbst lesen (abgeleitetes Markdown) und
//! schreiben (Erstbefüllung aus Markdown, API-/MCP-Änderungen), ohne dass ein Client „seeden“ muss.
//!
//! Das Schema entspricht dem TipTap-`StarterKit` des Web-Editors: Knoten `paragraph`,
//! `heading{level}`, `bulletList`, `orderedList{start}`, `listItem`, `blockquote`,
//! `codeBlock{language}`, `horizontalRule`, `hardBreak`, `mention{id,label}`; Marken `bold`,
//! `italic`, `strike`, `code`, `link{href}`. Unbekannte Knoten (z. B. von einem manipulierten
//! Client) werden beim Lesen auf ihren Text reduziert, unsichere Link-Ziele fallen weg.
//!
//! Das Lesen ist gegen feindliche Bäume gesichert (Tiefen- und Knotenlimit).

use lineal_domain::rules::markdown::is_safe_url;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::collections::HashMap;
use std::sync::Arc;
use yrs::types::text::YChange;
use yrs::types::xml::XmlOut;
use yrs::{
    Any, Out, ReadTxn, Text, TransactionMut, Xml, XmlElementPrelim, XmlFragment, XmlTextPrelim,
};

/// Name des Fragments, an das TipTap `Collaboration` standardmäßig bindet.
pub const FRAGMENT: &str = "default";
/// Tiefenlimit beim Lesen (ProseMirror-Bäume sind flach; tiefere gelten als Angriff/Defekt).
const MAX_DEPTH: usize = 32;
/// Knotenlimit beim Lesen.
const MAX_NODES: usize = 400_000;

/// Textmarke (Reihenfolge = Verschachtelung von außen nach innen).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mark {
    Link(String),
    Bold,
    Italic,
    Strike,
    Code,
}

impl Mark {
    fn name(&self) -> &'static str {
        match self {
            Self::Link(_) => "link",
            Self::Bold => "bold",
            Self::Italic => "italic",
            Self::Strike => "strike",
            Self::Code => "code",
        }
    }

    fn attrs(&self) -> Any {
        let mut m: HashMap<String, Any> = HashMap::new();
        if let Self::Link(href) = self {
            m.insert("href".into(), Any::String(href.as_str().into()));
        }
        Any::Map(Arc::new(m))
    }
}

/// Ein Knoten des ProseMirror-Baums.
#[derive(Debug, Clone, PartialEq)]
pub enum Pm {
    Elem {
        tag: String,
        attrs: Vec<(String, Any)>,
        content: Vec<Pm>,
    },
    Text {
        text: String,
        marks: Vec<Mark>,
    },
}

impl Pm {
    fn elem(tag: &str, content: Vec<Pm>) -> Self {
        Self::Elem {
            tag: tag.to_owned(),
            attrs: vec![],
            content,
        }
    }

    fn attr(&self, key: &str) -> Option<&Any> {
        match self {
            Self::Elem { attrs, .. } => attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            Self::Text { .. } => None,
        }
    }

    fn attr_int(&self, key: &str) -> Option<i64> {
        match self.attr(key)? {
            Any::Number(n) if n.is_finite() => Some(*n as i64),
            Any::BigInt(n) => Some(*n),
            Any::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    fn attr_str(&self, key: &str) -> Option<&str> {
        match self.attr(key)? {
            Any::String(s) => Some(s),
            _ => None,
        }
    }
}

// --- Markdown -> Pm -----------------------------------------------------------------------

struct Frame {
    tag: &'static str,
    attrs: Vec<(String, Any)>,
    content: Vec<Pm>,
    implicit: bool,
}

struct Builder {
    stack: Vec<Frame>,
    marks: Vec<Mark>,
}

impl Builder {
    fn new() -> Self {
        Self {
            stack: vec![Frame {
                tag: "doc",
                attrs: vec![],
                content: vec![],
                implicit: false,
            }],
            marks: vec![],
        }
    }

    fn top(&mut self) -> &mut Frame {
        self.stack.last_mut().expect("root frame")
    }

    fn open(&mut self, tag: &'static str, attrs: Vec<(String, Any)>, implicit: bool) {
        self.stack.push(Frame {
            tag,
            attrs,
            content: vec![],
            implicit,
        });
    }

    fn close(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let f = self.stack.pop().expect("frame");
        let node = Pm::Elem {
            tag: f.tag.to_owned(),
            attrs: f.attrs,
            content: f.content,
        };
        self.top().content.push(node);
    }

    fn close_implicit(&mut self) {
        while self.stack.len() > 1 && self.top().implicit {
            self.close();
        }
    }

    fn in_textblock(&self) -> bool {
        self.stack
            .last()
            .is_some_and(|f| matches!(f.tag, "paragraph" | "heading" | "codeBlock"))
    }

    fn ensure_textblock(&mut self) {
        if !self.in_textblock() {
            self.open("paragraph", vec![], true);
        }
    }

    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.ensure_textblock();
        let marks = if self.top().tag == "codeBlock" {
            vec![]
        } else {
            let mut m = self.marks.clone();
            m.sort();
            m.dedup();
            m
        };
        let content = &mut self.top().content;
        if let Some(Pm::Text { text: t, marks: m }) = content.last_mut()
            && *m == marks
        {
            t.push_str(text);
            return;
        }
        content.push(Pm::Text {
            text: text.to_owned(),
            marks,
        });
    }

    fn push_inline(&mut self, node: Pm) {
        self.ensure_textblock();
        self.top().content.push(node);
    }
}

fn heading_level(l: HeadingLevel) -> f64 {
    match l {
        HeadingLevel::H1 => 1.0,
        HeadingLevel::H2 => 2.0,
        HeadingLevel::H3 => 3.0,
        HeadingLevel::H4 => 4.0,
        HeadingLevel::H5 => 5.0,
        HeadingLevel::H6 => 6.0,
    }
}

/// Parst Markdown in die Blöcke der Dokumentwurzel.
pub fn parse_markdown(md: &str) -> Vec<Pm> {
    let mut b = Builder::new();
    let parser = Parser::new_ext(md, Options::ENABLE_STRIKETHROUGH);
    for ev in parser {
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    b.close_implicit();
                    b.open("paragraph", vec![], false);
                }
                Tag::Heading { level, .. } => {
                    b.close_implicit();
                    b.open(
                        "heading",
                        vec![("level".into(), Any::Number(heading_level(level)))],
                        false,
                    );
                }
                Tag::BlockQuote(_) => {
                    b.close_implicit();
                    b.open("blockquote", vec![], false);
                }
                Tag::CodeBlock(kind) => {
                    b.close_implicit();
                    let mut attrs = vec![];
                    if let CodeBlockKind::Fenced(lang) = kind {
                        let lang = lang.split_whitespace().next().unwrap_or("");
                        if !lang.is_empty() && lang.len() <= 64 {
                            attrs.push(("language".into(), Any::String(lang.into())));
                        }
                    }
                    b.open("codeBlock", attrs, false);
                }
                Tag::List(start) => {
                    b.close_implicit();
                    match start {
                        Some(n) => {
                            let attrs = if n == 1 {
                                vec![]
                            } else {
                                vec![("start".into(), Any::Number(n.min(1_000_000_000) as f64))]
                            };
                            b.open("orderedList", attrs, false);
                        }
                        None => b.open("bulletList", vec![], false),
                    }
                }
                Tag::Item => {
                    b.close_implicit();
                    b.open("listItem", vec![], false);
                }
                Tag::Emphasis => b.marks.push(Mark::Italic),
                Tag::Strong => b.marks.push(Mark::Bold),
                Tag::Strikethrough => b.marks.push(Mark::Strike),
                // Bilder: kein Bildknoten im Schema, die Adresse bleibt als Link erhalten.
                // Unsichere/leere Ziele: leerer Link = Platzhalter (wird zu reinem Text).
                Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                    let ok = is_safe_url(&dest_url) && !dest_url.is_empty();
                    b.marks.push(Mark::Link(if ok {
                        dest_url.to_string()
                    } else {
                        String::new()
                    }));
                }
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::CodeBlock
                | TagEnd::List(_)
                | TagEnd::Item => {
                    b.close_implicit();
                    b.close();
                }
                TagEnd::Emphasis
                | TagEnd::Strong
                | TagEnd::Strikethrough
                | TagEnd::Link
                | TagEnd::Image => {
                    b.marks.pop();
                }
                _ => {}
            },
            Event::Text(t) => b.push_text(&t),
            Event::Code(t) => {
                b.marks.push(Mark::Code);
                b.push_text(&t);
                b.marks.pop();
            }
            Event::SoftBreak => b.push_text(" "),
            Event::HardBreak => b.push_inline(Pm::elem("hardBreak", vec![])),
            Event::Rule => {
                b.close_implicit();
                b.top().content.push(Pm::elem("horizontalRule", vec![]));
            }
            // Rohes HTML, Fußnoten, Aufgabenmarker u. a.: nicht im Schema.
            _ => {}
        }
    }
    b.close_implicit();
    while b.stack.len() > 1 {
        b.close();
    }
    let mut out = b.stack.pop().expect("root").content;
    clean_marks(&mut out);
    out
}

fn clean_marks(nodes: &mut [Pm]) {
    for n in nodes {
        match n {
            Pm::Text { marks, .. } => {
                marks.retain(|m| !matches!(m, Mark::Link(h) if h.is_empty()));
            }
            Pm::Elem { content, .. } => clean_marks(content),
        }
    }
}

// --- Pm -> Y ------------------------------------------------------------------------------

/// Fügt `nodes` ab Index `at` in `parent` ein; liefert die Zahl der eingefügten Kindknoten.
pub fn insert_nodes<F: XmlFragment>(
    parent: &F,
    txn: &mut TransactionMut,
    mut at: u32,
    nodes: &[Pm],
) -> u32 {
    let start = at;
    let mut i = 0;
    while i < nodes.len() {
        match &nodes[i] {
            Pm::Text { .. } => {
                let t = parent.insert(txn, at, XmlTextPrelim::new(""));
                while let Some(Pm::Text { text, marks }) = nodes.get(i) {
                    let mut attrs: HashMap<Arc<str>, Any> = HashMap::new();
                    for m in marks {
                        attrs.insert(m.name().into(), m.attrs());
                    }
                    let len = t.len(&*txn);
                    t.insert_with_attributes(txn, len, text, attrs);
                    i += 1;
                }
                at += 1;
            }
            Pm::Elem {
                tag,
                attrs,
                content,
            } => {
                let el = parent.insert(txn, at, XmlElementPrelim::empty(tag.as_str()));
                for (k, v) in attrs {
                    el.insert_attribute(txn, k.as_str(), v.clone());
                }
                insert_nodes(&el, txn, 0, content);
                at += 1;
                i += 1;
            }
        }
    }
    at - start
}

// --- Y -> Pm ------------------------------------------------------------------------------

fn read_marks(attrs: &HashMap<Arc<str>, Any>) -> Vec<Mark> {
    let mut marks = vec![];
    for (name, value) in attrs {
        // `null` = Marke entfernt (Yjs-Formatierung zurückgenommen).
        if matches!(value, Any::Null | Any::Undefined | Any::Bool(false)) {
            continue;
        }
        match &**name {
            "bold" | "strong" => marks.push(Mark::Bold),
            "italic" | "em" => marks.push(Mark::Italic),
            "strike" => marks.push(Mark::Strike),
            "code" => marks.push(Mark::Code),
            "link" => {
                if let Any::Map(m) = value
                    && let Some(Any::String(href)) = m.get("href")
                    && is_safe_url(href)
                    && !href.is_empty()
                {
                    marks.push(Mark::Link(href.to_string()));
                }
            }
            _ => {}
        }
    }
    marks.sort();
    marks.dedup();
    marks
}

struct Reader {
    nodes: usize,
}

impl Reader {
    fn read<T: ReadTxn, F: XmlFragment>(&mut self, parent: &F, txn: &T, depth: usize) -> Vec<Pm> {
        let mut out = vec![];
        if depth > MAX_DEPTH {
            return out;
        }
        for child in parent.children(txn) {
            self.nodes += 1;
            if self.nodes > MAX_NODES {
                break;
            }
            match child {
                XmlOut::Element(el) => {
                    let tag = el.tag().to_string();
                    let attrs: Vec<(String, Any)> = el
                        .attributes(txn)
                        .filter_map(|(k, v)| match v {
                            Out::Any(a) => Some((k.to_string(), a)),
                            _ => None,
                        })
                        .collect();
                    let content = self.read(&el, txn, depth + 1);
                    out.push(normalize_elem(tag, attrs, content));
                }
                XmlOut::Text(t) => {
                    for d in t.diff(txn, |y: YChange| y) {
                        let Out::Any(Any::String(s)) = &d.insert else {
                            continue;
                        };
                        let marks = d.attributes.as_deref().map(read_marks).unwrap_or_default();
                        out.push(Pm::Text {
                            text: s.to_string(),
                            marks,
                        });
                    }
                }
                XmlOut::Fragment(_) => {}
            }
        }
        out
    }
}

const KNOWN_NODES: [&str; 10] = [
    "paragraph",
    "heading",
    "bulletList",
    "orderedList",
    "listItem",
    "blockquote",
    "codeBlock",
    "horizontalRule",
    "hardBreak",
    "mention",
];

/// Unbekannte Elemente werden zu einem Absatz mit ihrem Text.
fn normalize_elem(tag: String, attrs: Vec<(String, Any)>, content: Vec<Pm>) -> Pm {
    if KNOWN_NODES.contains(&tag.as_str()) {
        return Pm::Elem {
            tag,
            attrs,
            content,
        };
    }
    let mut text = Vec::new();
    collect_text(&content, &mut text);
    Pm::Elem {
        tag: "paragraph".into(),
        attrs: vec![],
        content: text,
    }
}

fn collect_text(nodes: &[Pm], out: &mut Vec<Pm>) {
    for n in nodes {
        match n {
            Pm::Text { .. } => out.push(n.clone()),
            Pm::Elem { content, .. } => collect_text(content, out),
        }
    }
}

/// Liest die Blöcke des Fragments.
pub fn read_fragment<T: ReadTxn, F: XmlFragment>(frag: &F, txn: &T) -> Vec<Pm> {
    Reader { nodes: 0 }.read(frag, txn, 0)
}

// --- Pm -> Markdown -----------------------------------------------------------------------

/// Maskiert Markdown-Syntax in Fließtext. `line_start`: Anfang einer Zeile (Listen-/Überschrift-
/// Marker würden sonst Blöcke erzeugen).
fn escape_text(s: &str, line_start: bool) -> String {
    let chars: Vec<char> = s.chars().collect();
    let task_prefix = s.starts_with("[ ] ") || s.starts_with("[x] ") || s.starts_with("[X] ");
    // Index des Punkts/der Klammer einer Zeilenanfangs-Nummerierung („1. “).
    let digits = chars.iter().take_while(|c| c.is_ascii_digit()).count();
    let ordered_marker = (line_start
        && digits > 0
        && matches!(chars.get(digits), Some('.' | ')'))
        && chars.get(digits + 1).is_none_or(|n| *n == ' '))
    .then_some(digits);
    let mut out = String::with_capacity(s.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).and_then(|j| chars.get(j)).copied();
        let next = chars.get(i + 1).copied();
        let alnum = |o: Option<char>| o.is_some_and(char::is_alphanumeric);
        let escape = match c {
            '\\' | '*' | '`' | '<' => true,
            '_' => !(alnum(prev) && alnum(next)),
            '[' | ']' => !(task_prefix && i < 3),
            '~' => next == Some('~') || prev == Some('~'),
            '#' | '>' => line_start && i == 0,
            '-' | '+' => line_start && i == 0 && next.is_none_or(|n| n == ' '),
            '.' | ')' => ordered_marker == Some(i),
            _ => false,
        };
        if escape {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn fence_for(text: &str, ch: char, min: usize) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        if c == ch {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    ch.to_string().repeat((longest + 1).max(min))
}

fn link_dest(href: &str) -> String {
    let mut out = String::new();
    for c in href.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            '\n' | '\r' => {}
            c => out.push(c),
        }
    }
    out
}

fn wrap_marks(text: &str, marks: &[Mark], line_start: bool) -> String {
    if marks.is_empty() {
        return escape_text(text, line_start);
    }
    // Leerraum an den Rändern bleibt außerhalb der Auszeichnung.
    let core = text.trim();
    if core.is_empty() {
        return text.to_owned();
    }
    let lead = &text[..text.len() - text.trim_start().len()];
    let trail = &text[text.trim_end().len()..];
    let mut s = if marks.contains(&Mark::Code) {
        let f = fence_for(core, '`', 1);
        let pad = if core.starts_with('`') || core.ends_with('`') {
            " "
        } else {
            ""
        };
        format!("{f}{pad}{core}{pad}{f}")
    } else {
        escape_text(core, line_start && lead.is_empty())
    };
    for m in marks.iter().rev() {
        s = match m {
            Mark::Code => s,
            Mark::Strike => format!("~~{s}~~"),
            Mark::Italic => format!("*{s}*"),
            Mark::Bold => format!("**{s}**"),
            Mark::Link(href) => format!("[{s}]({})", link_dest(href)),
        };
    }
    format!("{lead}{s}{trail}")
}

fn inline_md(content: &[Pm]) -> String {
    let mut out = String::new();
    let mut at_start = true;
    for n in content {
        match n {
            Pm::Text { text, marks } => {
                let t = text.replace(['\n', '\r'], " ");
                out.push_str(&wrap_marks(&t, marks, at_start));
                if !t.is_empty() {
                    at_start = false;
                }
            }
            Pm::Elem { tag, .. } if tag == "hardBreak" => {
                out.push_str("\\\n");
                at_start = true;
            }
            Pm::Elem { tag, .. } if tag == "mention" => {
                let label = n
                    .attr_str("label")
                    .or_else(|| n.attr_str("id"))
                    .unwrap_or("");
                out.push('@');
                out.push_str(&escape_text(label, false));
                at_start = false;
            }
            Pm::Elem { content, .. } => {
                out.push_str(&inline_md(content));
                at_start = false;
            }
        }
    }
    out
}

fn plain_text(content: &[Pm]) -> String {
    let mut s = String::new();
    for n in content {
        match n {
            Pm::Text { text, .. } => s.push_str(text),
            Pm::Elem { tag, .. } if tag == "hardBreak" => s.push('\n'),
            Pm::Elem { content, .. } => s.push_str(&plain_text(content)),
        }
    }
    s
}

fn indent(s: &str, first: &str, rest: &str) -> String {
    let mut out = String::new();
    for (i, line) in s.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let p = if i == 0 { first } else { rest };
        if line.is_empty() {
            out.push_str(p.trim_end());
        } else {
            out.push_str(p);
            out.push_str(line);
        }
    }
    out
}

fn is_list(n: &Pm) -> bool {
    matches!(n, Pm::Elem { tag, .. } if tag == "bulletList" || tag == "orderedList")
}

fn block_md(n: &Pm, depth: usize) -> Option<String> {
    let Pm::Elem { tag, content, .. } = n else {
        // Verirrter Text auf Blockebene.
        return Some(inline_md(std::slice::from_ref(n))).filter(|s| !s.trim().is_empty());
    };
    if depth > MAX_DEPTH {
        return None;
    }
    match tag.as_str() {
        "paragraph" => Some(inline_md(content)).filter(|s| !s.trim().is_empty()),
        "heading" => {
            let level = usize::try_from(n.attr_int("level").unwrap_or(1).clamp(1, 6)).unwrap_or(1);
            let text = inline_md(content);
            if text.trim().is_empty() {
                return None;
            }
            Some(format!("{} {}", "#".repeat(level), text.replace('\n', " ")))
        }
        "blockquote" => {
            let inner = blocks_md(content, depth + 1);
            if inner.is_empty() {
                return None;
            }
            Some(indent(&inner, "> ", "> "))
        }
        "codeBlock" => {
            let text = plain_text(content);
            let text = text.trim_end_matches('\n');
            let f = fence_for(text, '`', 3);
            let lang = n
                .attr_str("language")
                .filter(|l| {
                    l.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "+-_#.".contains(c))
                })
                .unwrap_or("");
            Some(format!("{f}{lang}\n{text}\n{f}"))
        }
        "horizontalRule" => Some("---".to_owned()),
        "bulletList" | "orderedList" => {
            let ordered = tag == "orderedList";
            let start = n.attr_int("start").unwrap_or(1).clamp(0, 1_000_000_000);
            let mut items = Vec::new();
            for (i, item) in content.iter().enumerate() {
                let Pm::Elem { content: ic, .. } = item else {
                    continue;
                };
                let marker = if ordered {
                    format!("{}. ", start + i64::try_from(i).unwrap_or(0))
                } else {
                    "- ".to_owned()
                };
                let pad = " ".repeat(marker.len());
                items.push(indent(&list_item_md(ic, depth + 1), &marker, &pad));
            }
            if items.is_empty() {
                None
            } else {
                Some(items.join("\n"))
            }
        }
        "hardBreak" | "mention" => None,
        _ => Some(inline_md(content)).filter(|s| !s.trim().is_empty()),
    }
}

fn list_item_md(content: &[Pm], depth: usize) -> String {
    let mut out = String::new();
    let mut first = true;
    for b in content {
        let Some(s) = block_md(b, depth) else {
            continue;
        };
        if !first {
            // Verschachtelte Listen eng anhängen, andere Blöcke mit Leerzeile (lose Liste).
            out.push_str(if is_list(b) { "\n" } else { "\n\n" });
        }
        out.push_str(&s);
        first = false;
    }
    out
}

fn blocks_md(blocks: &[Pm], depth: usize) -> String {
    blocks
        .iter()
        .filter_map(|b| block_md(b, depth))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Markdown eines einzelnen Blocks (leer, wenn der Block nichts enthält).
pub fn block_to_markdown(block: &Pm) -> String {
    block_md(block, 0).unwrap_or_default()
}

/// Markdown der gesamten Blockfolge.
pub fn to_markdown(blocks: &[Pm]) -> String {
    blocks_md(blocks, 0)
}
