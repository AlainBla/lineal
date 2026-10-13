//! Yjs-Dokument eines Raums: Erzeugen aus Markdown, (De-)Serialisierung des Zustands
//! (`contentState`), abgeleitetes Markdown, minimale Änderungen am Baum (API/MCP-Schreibvorgänge)
//! und Kommentar-Anker als relative Positionen.

use super::pm::{self, FRAGMENT, Mark, Pm};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use lineal_domain::rules::markdown::sanitize;
use std::collections::HashMap;
use std::sync::Arc;
use yrs::types::text::YChange;
use yrs::types::xml::XmlOut;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{
    Any, Assoc, Doc, IndexedSequence, OffsetKind, Options, Out, ReadTxn, StateVector, StickyIndex,
    Text, Transact, Update, XmlFragment, XmlTextRef,
};

/// Neues leeres Dokument. UTF-16-Offsets wie bei den JavaScript-Clients (Textpositionen, StickyIndex).
pub fn new_doc() -> Doc {
    Doc::with_options(Options {
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    })
}

/// Dokument aus Markdown (serverseitige Erstbefüllung: nie ein Client, damit gleichzeitiges
/// Öffnen den Inhalt nicht verdoppelt).
pub fn doc_from_markdown(markdown: &str) -> Doc {
    let doc = new_doc();
    let frag = doc.get_or_insert_xml_fragment(FRAGMENT);
    let blocks = pm::parse_markdown(markdown);
    let mut txn = doc.transact_mut();
    pm::insert_nodes(&frag, &mut txn, 0, &blocks);
    drop(txn);
    doc
}

/// Dokument aus gespeichertem Zustand (Base64 eines Yjs-Updates V1); `None` bei defektem Zustand.
pub fn doc_from_state(state_b64: &str) -> Option<Doc> {
    let bytes = STANDARD.decode(state_b64.trim()).ok()?;
    let update = Update::decode_v1(&bytes).ok()?;
    let doc = new_doc();
    // Das Fragment muss vor dem Anwenden existieren, damit der Typ erkannt wird.
    let _ = doc.get_or_insert_xml_fragment(FRAGMENT);
    doc.transact_mut().apply_update(update).ok()?;
    Some(doc)
}

/// Wendet einen Zustand/ein Update (Base64) auf ein bestehendes Dokument an und liefert das
/// dabei neu Hinzugekommene als Update (`None`, wenn nichts neu war oder der Zustand defekt ist).
pub fn merge_state(doc: &Doc, state_b64: &str) -> Option<Vec<u8>> {
    let bytes = STANDARD.decode(state_b64.trim()).ok()?;
    let update = Update::decode_v1(&bytes).ok()?;
    let mut txn = doc.transact_mut();
    let before = txn.state_vector();
    txn.apply_update(update).ok()?;
    let diff = txn.encode_state_as_update_v1(&before);
    // Ein leeres Update (nur Header) ist 2 Bytes lang.
    (diff.len() > 2).then_some(diff)
}

/// Gesamter Zustand als Base64 (`contentState`).
pub fn state_to_b64(doc: &Doc) -> String {
    let bytes = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    STANDARD.encode(bytes)
}

/// Größe des kodierten Gesamtzustands in Bytes.
pub fn state_size(doc: &Doc) -> usize {
    doc.transact()
        .encode_state_as_update_v1(&StateVector::default())
        .len()
}

/// Abgeleitetes Markdown (unbereinigt; der Aufrufer bereinigt beim Speichern).
pub fn markdown_of(doc: &Doc) -> String {
    let frag = doc.get_or_insert_xml_fragment(FRAGMENT);
    let txn = doc.transact();
    pm::to_markdown(&pm::read_fragment(&frag, &txn))
}

/// Wie [`markdown_of`], bereinigt (so wird es gespeichert).
pub fn sanitized_markdown_of(doc: &Doc) -> String {
    sanitize(&markdown_of(doc))
}

/// Gehört der Zustand zu diesem Markdown? Gespeichertes Markdown, das beim Parsen und Ausgeben
/// nicht stabil ist (`* a` statt `- a`, Umbrüche, Leerzeilen), gilt über seine kanonische Form:
/// sonst würde der Zustand bei jedem API-Schreiben verworfen und der Raum verlöre ungespeicherte
/// Eingaben (B2.01).
pub fn state_matches_markdown(doc: &Doc, markdown: &str) -> bool {
    let current = sanitized_markdown_of(doc);
    current == markdown || current == sanitized_markdown_of(&doc_from_markdown(markdown))
}

// --- Minimale Änderungen ------------------------------------------------------------------

/// Setzt den Inhalt per minimaler Änderung auf `new_md`: Blöcke werden über ihr Markdown
/// verglichen (LCS); unveränderte Blöcke bleiben samt Cursorn und Kommentar-Ankern unberührt,
/// in geänderten Absätzen/Überschriften wird nur die abweichende Textspanne ersetzt, übrige
/// Blöcke werden gelöscht/eingefügt. Liefert das Update (V1) für die Teilnehmer, `None` wenn
/// nichts zu ändern war.
pub fn apply_markdown(doc: &Doc, new_md: &str) -> Option<Vec<u8>> {
    let frag = doc.get_or_insert_xml_fragment(FRAGMENT);
    let new_blocks: Vec<Pm> = pm::parse_markdown(new_md);
    let new_keys: Vec<String> = new_blocks.iter().map(pm::block_to_markdown).collect();
    let mut txn = doc.transact_mut();
    let before = txn.state_vector();
    let old_blocks = pm::read_top_level(&frag, &txn);
    if u32::try_from(old_blocks.len()).ok() != Some(frag.len(&txn)) {
        return None;
    }
    // Leere Blöcke (z. B. der abschließende Absatz des Editors) bleiben unangetastet: sie sind im
    // Markdown unsichtbar. Gearbeitet wird auf den Indizes der Quellkinder.
    let old: Vec<(usize, String)> = old_blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (i, pm::block_to_markdown(b)))
        .filter(|(_, k)| !k.is_empty())
        .collect();
    let old_keys: Vec<&str> = old.iter().map(|(_, k)| k.as_str()).collect();
    let new_refs: Vec<&str> = new_keys.iter().map(String::as_str).collect();
    let ops = diff_blocks(&old_keys, &new_refs);
    if ops.is_empty() {
        return None;
    }
    let idx = |i: usize| u32::try_from(i).unwrap_or(0);
    // Von hinten nach vorn, damit die Indizes der vorderen Operationen gültig bleiben.
    for op in ops.into_iter().rev() {
        if op.old_len == op.new_len {
            for k in (0..op.old_len).rev() {
                let at = old[op.old_start + k].0;
                let new_block = &new_blocks[op.new_start + k];
                if !edit_block_in_place(&frag, &mut txn, at, &old_blocks[at], new_block) {
                    frag.remove_range(&mut txn, idx(at), 1);
                    pm::insert_nodes(&frag, &mut txn, idx(at), std::slice::from_ref(new_block));
                }
            }
            continue;
        }
        let (remove_from, remove_len) = if op.old_len == 0 {
            (0, 0)
        } else {
            let first = old[op.old_start].0;
            let last = old[op.old_start + op.old_len - 1].0;
            (first, last - first + 1)
        };
        let insert_at = if op.old_len > 0 {
            remove_from
        } else if op.old_start < old.len() {
            // Vor dem alten Block `old_start` bzw. ans Ende der inhaltlichen Blöcke.
            old[op.old_start].0
        } else {
            old.last().map_or(0, |(i, _)| i + 1)
        };
        if remove_len > 0 {
            frag.remove_range(&mut txn, idx(remove_from), idx(remove_len));
        }
        let slice = &new_blocks[op.new_start..op.new_start + op.new_len];
        if !slice.is_empty() {
            pm::insert_nodes(&frag, &mut txn, idx(insert_at), slice);
        }
    }
    let update = txn.encode_state_as_update_v1(&before);
    drop(txn);
    Some(update)
}

/// Marken je Zeichen (als Index in `table`), UTF-16-Länge je Zeichen.
fn char_marks<'a>(content: &'a [Pm], table: &mut Vec<&'a [Mark]>) -> Option<Vec<(char, usize)>> {
    let mut out = Vec::new();
    for n in content {
        let Pm::Text { text, marks } = n else {
            return None;
        };
        let id = table
            .iter()
            .position(|m| *m == marks.as_slice())
            .unwrap_or_else(|| {
                table.push(marks);
                table.len() - 1
            });
        out.extend(text.chars().map(|c| (c, id)));
    }
    Some(out)
}

fn utf16_len(chars: &[(char, usize)]) -> u32 {
    chars
        .iter()
        .map(|(c, _)| u32::try_from(c.len_utf16()).unwrap_or(1))
        .sum()
}

/// Ersetzt in einem Absatz/einer Überschrift nur die abweichende Textspanne. `false`, wenn der
/// Block dafür nicht geeignet ist (anderer Typ/Attribute, kein reiner Text) – dann ersetzt der
/// Aufrufer den ganzen Block.
fn edit_block_in_place<F: XmlFragment>(
    frag: &F,
    txn: &mut yrs::TransactionMut,
    at: usize,
    old: &Pm,
    new: &Pm,
) -> bool {
    let (
        Pm::Elem {
            tag: ot,
            attrs: oa,
            content: oc,
        },
        Pm::Elem {
            tag: nt,
            attrs: na,
            content: nc,
        },
    ) = (old, new)
    else {
        return false;
    };
    if ot != nt || !matches!(ot.as_str(), "paragraph" | "heading") || oa != na {
        return false;
    }
    let mut table: Vec<&[Mark]> = Vec::new();
    let (Some(oc_chars), Some(nc_chars)) = (char_marks(oc, &mut table), char_marks(nc, &mut table))
    else {
        return false;
    };
    if oc_chars.is_empty() {
        return false;
    }
    // Y-Seite: genau ein XmlText als Kind.
    let Some(XmlOut::Element(el)) = u32::try_from(at).ok().and_then(|i| frag.get(&*txn, i)) else {
        return false;
    };
    if el.len(&*txn) != 1 {
        return false;
    }
    let Some(XmlOut::Text(text)) = el.get(&*txn, 0) else {
        return false;
    };
    // Der gelesene Text muss die Länge des Y-Texts exakt abbilden (Einbettungen u. Ä. würden die
    // Offsets verschieben): sonst wird der ganze Block ersetzt statt falsch zu schneiden.
    if text.len(&*txn) != utf16_len(&oc_chars) {
        return false;
    }
    let mut pre = 0;
    while pre < oc_chars.len() && pre < nc_chars.len() && oc_chars[pre] == nc_chars[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < oc_chars.len() - pre
        && suf < nc_chars.len() - pre
        && oc_chars[oc_chars.len() - 1 - suf] == nc_chars[nc_chars.len() - 1 - suf]
    {
        suf += 1;
    }
    let old_mid = &oc_chars[pre..oc_chars.len() - suf];
    let new_mid = &nc_chars[pre..nc_chars.len() - suf];
    if old_mid.is_empty() && new_mid.is_empty() {
        return true;
    }
    let start = utf16_len(&oc_chars[..pre]);
    let del = utf16_len(old_mid);
    if del > 0 {
        text.remove_range(txn, start, del);
    }
    // Neue Spanne in Läufen gleicher Marken einfügen.
    let mut at_u16 = start;
    let mut i = 0;
    while i < new_mid.len() {
        let id = new_mid[i].1;
        let mut j = i;
        let mut chunk = String::new();
        while j < new_mid.len() && new_mid[j].1 == id {
            chunk.push(new_mid[j].0);
            j += 1;
        }
        let mut attrs: HashMap<Arc<str>, Any> = HashMap::new();
        for m in table[id] {
            attrs.insert(m.name().into(), m.attrs());
        }
        text.insert_with_attributes(txn, at_u16, &chunk, attrs);
        at_u16 += u32::try_from(chunk.encode_utf16().count()).unwrap_or(0);
        i = j;
    }
    true
}

#[derive(Debug, PartialEq, Eq)]
struct BlockOp {
    old_start: usize,
    old_len: usize,
    new_start: usize,
    new_len: usize,
}

/// Zusammenhängende Änderungsbereiche zwischen zwei Blockfolgen (LCS; bei sehr großen Eingaben
/// nur Präfix/Suffix-Abgleich).
fn diff_blocks(old: &[&str], new: &[&str]) -> Vec<BlockOp> {
    let mut pre = 0;
    while pre < old.len() && pre < new.len() && old[pre] == new[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < old.len() - pre
        && suf < new.len() - pre
        && old[old.len() - 1 - suf] == new[new.len() - 1 - suf]
    {
        suf += 1;
    }
    let o = &old[pre..old.len() - suf];
    let n = &new[pre..new.len() - suf];
    if o.is_empty() && n.is_empty() {
        return vec![];
    }
    if o.is_empty() || n.is_empty() || o.len().saturating_mul(n.len()) > 4_000_000 {
        return vec![BlockOp {
            old_start: pre,
            old_len: o.len(),
            new_start: pre,
            new_len: n.len(),
        }];
    }
    let w = n.len() + 1;
    let mut t = vec![0u32; (o.len() + 1) * w];
    for i in (0..o.len()).rev() {
        for j in (0..n.len()).rev() {
            t[i * w + j] = if o[i] == n[j] {
                t[(i + 1) * w + j + 1] + 1
            } else {
                t[(i + 1) * w + j].max(t[i * w + j + 1])
            };
        }
    }
    let mut ops: Vec<BlockOp> = Vec::new();
    let (mut i, mut j) = (0, 0);
    let mut cur: Option<BlockOp> = None;
    while i < o.len() || j < n.len() {
        if i < o.len() && j < n.len() && o[i] == n[j] {
            ops.extend(cur.take());
            i += 1;
            j += 1;
        } else if j < n.len() && (i == o.len() || t[i * w + j + 1] >= t[(i + 1) * w + j]) {
            let c = cur.get_or_insert(BlockOp {
                old_start: pre + i,
                old_len: 0,
                new_start: pre + j,
                new_len: 0,
            });
            c.new_len += 1;
            j += 1;
        } else {
            let c = cur.get_or_insert(BlockOp {
                old_start: pre + i,
                old_len: 0,
                new_start: pre + j,
                new_len: 0,
            });
            c.old_len += 1;
            i += 1;
        }
    }
    ops.extend(cur.take());
    ops
}

// --- Kommentar-Anker ----------------------------------------------------------------------

/// Zustand eines Kommentar-Ankers (docs/06 §1.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorState {
    Attached,
    Detached,
}

impl AnchorState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Attached => "attached",
            Self::Detached => "detached",
        }
    }
}

/// Höchstlänge eines kodierten Ankers (Base64).
pub const MAX_ANCHOR_B64: usize = 512;

fn decode_sticky(b64: &str) -> Option<StickyIndex> {
    if b64.len() > MAX_ANCHOR_B64 {
        return None;
    }
    let bytes = STANDARD.decode(b64.trim()).ok()?;
    StickyIndex::decode_v1(&bytes).ok()
}

/// Ob der Anker (relative Position, Base64 von `Y.encodeRelativePosition`) gültig kodiert ist.
pub fn anchor_is_valid(b64: &str) -> bool {
    decode_sticky(b64).is_some()
}

/// Löst einen Anker gegen das Dokument auf: `Detached`, wenn eine Position nicht mehr existiert
/// oder der markierte Bereich vollständig gelöscht wurde (Start und Ende fallen zusammen).
/// `None`, wenn die Kodierung unlesbar ist.
pub fn anchor_state(doc: &Doc, start_b64: &str, end_b64: &str) -> Option<AnchorState> {
    let start = decode_sticky(start_b64)?;
    let end = decode_sticky(end_b64)?;
    let txn = doc.transact();
    let (Some(a), Some(b)) = (start.get_offset(&txn), end.get_offset(&txn)) else {
        return Some(AnchorState::Detached);
    };
    if a.branch == b.branch && a.index >= b.index {
        return Some(AnchorState::Detached);
    }
    Some(AnchorState::Attached)
}

fn plain_of(text: &XmlTextRef, txn: &impl ReadTxn) -> String {
    let mut s = String::new();
    for d in text.diff(txn, |y: YChange| y) {
        if let Out::Any(Any::String(t)) = &d.insert {
            s.push_str(t);
        }
    }
    s
}

/// Leitet für einen Inline-Kommentar ohne Positionsanker (Altbestand) Anker aus `quotedText` ab:
/// erstes Vorkommen im Klartext eines Textknotens (docs/06 §1.8 Regel 1). `None`, wenn der Text
/// nicht (innerhalb eines Knotens) vorkommt – dann gilt weiter der Fallback `quotedText`.
pub fn anchors_for_quote(doc: &Doc, quoted: &str) -> Option<(String, String)> {
    let quoted = quoted.trim();
    if quoted.is_empty() {
        return None;
    }
    let frag = doc.get_or_insert_xml_fragment(FRAGMENT);
    let txn = doc.transact();
    for node in frag.successors(&txn) {
        let XmlOut::Text(t) = node else { continue };
        let plain = plain_of(&t, &txn);
        let Some(byte_at) = plain.find(quoted) else {
            continue;
        };
        let start = u32::try_from(plain[..byte_at].encode_utf16().count()).ok()?;
        let len = u32::try_from(quoted.encode_utf16().count()).ok()?;
        let end = start.checked_add(len)?;
        // Offsets nur, wenn der gelesene Text die Länge des Y-Texts exakt abbildet.
        if t.len(&txn) != u32::try_from(plain.encode_utf16().count()).ok()? {
            continue;
        }
        let a = t.sticky_index(&txn, start, Assoc::After)?;
        let b = t.sticky_index(&txn, end, Assoc::Before)?;
        return Some((
            STANDARD.encode(a.encode_v1()),
            STANDARD.encode(b.encode_v1()),
        ));
    }
    None
}

/// Verletzung der Strukturgrenzen eines Dokuments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitError {
    TooDeep,
    TooManyNodes,
}

/// Prüft Tiefe ([`pm::MAX_DEPTH`]) und Knotenzahl ([`pm::MAX_NODES`]) des Baums **iterativ**
/// (eigener Stapel, keine Rekursion), damit auch feindliche Bäume keinen Stack-Überlauf auslösen.
pub fn check_limits(doc: &Doc) -> Result<(), LimitError> {
    let frag = doc.get_or_insert_xml_fragment(FRAGMENT);
    let txn = doc.transact();
    let mut nodes = 0usize;
    let mut stack: Vec<(yrs::XmlElementRef, usize)> = Vec::new();
    for child in frag.children(&txn) {
        nodes += 1;
        if let XmlOut::Element(e) = child {
            stack.push((e, 1));
        }
    }
    while let Some((el, depth)) = stack.pop() {
        if depth > pm::MAX_DEPTH {
            return Err(LimitError::TooDeep);
        }
        for child in el.children(&txn) {
            nodes += 1;
            if nodes > pm::MAX_NODES {
                return Err(LimitError::TooManyNodes);
            }
            if let XmlOut::Element(e) = child {
                stack.push((e, depth + 1));
            }
        }
    }
    if nodes > pm::MAX_NODES {
        return Err(LimitError::TooManyNodes);
    }
    Ok(())
}

/// Zahl der Kindknoten der Wurzel (Tests/Diagnose).
pub fn block_count(doc: &Doc) -> u32 {
    let frag = doc.get_or_insert_xml_fragment(FRAGMENT);
    frag.len(&doc.transact())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Titel\n\nEin **fetter** und *kursiver* Text mit `code` und ~~weg~~ sowie [Link](https://example.com/a).\n\n- eins\n- zwei\n  - zwei-a\n- drei\n\n1. a\n2. b\n\n> Zitat\n\n```rust\nfn main() {}\n```\n\n---\n\n## Klein\n\nLetzter Absatz";

    #[test]
    fn markdown_round_trips_through_the_tree() {
        let doc = doc_from_markdown(SAMPLE);
        assert_eq!(markdown_of(&doc), SAMPLE);
        let again = doc_from_state(&state_to_b64(&doc)).expect("state");
        assert_eq!(markdown_of(&again), SAMPLE);
    }

    #[test]
    fn special_characters_survive() {
        for md in [
            "a \\* b",
            "snake_case und \\_lead",
            "\\# keine Überschrift",
            "1\\. keine Liste",
            "- [ ] offen",
            "Text mit \\<tag>",
        ] {
            let doc = doc_from_markdown(md);
            assert_eq!(markdown_of(&doc), md, "{md}");
        }
    }

    #[test]
    fn unsafe_links_lose_the_target() {
        let doc = doc_from_markdown("[x](javascript:alert(1))");
        assert_eq!(markdown_of(&doc), "x");
    }

    #[test]
    fn apply_markdown_is_minimal_and_keeps_untouched_blocks() {
        let doc = doc_from_markdown("eins\n\nzwei\n\ndrei");
        let other = doc_from_state(&state_to_b64(&doc)).expect("state");
        let update = apply_markdown(&doc, "eins\n\nzwei\n\nneu\n\ndrei").expect("changed");
        assert_eq!(markdown_of(&doc), "eins\n\nzwei\n\nneu\n\ndrei");
        // Nur ein Block wurde neu erzeugt: das Update ist klein.
        assert!(update.len() < 80, "update len {}", update.len());
        other
            .transact_mut()
            .apply_update(Update::decode_v1(&update).expect("decode"))
            .expect("apply");
        assert_eq!(markdown_of(&other), "eins\n\nzwei\n\nneu\n\ndrei");
        assert!(apply_markdown(&doc, "eins\n\nzwei\n\nneu\n\ndrei").is_none());
        apply_markdown(&doc, "eins\n\ndrei").expect("changed");
        assert_eq!(markdown_of(&doc), "eins\n\ndrei");
        apply_markdown(&doc, "").expect("changed");
        assert_eq!(markdown_of(&doc), "");
    }

    #[test]
    fn apply_markdown_edits_text_in_place() {
        let doc = doc_from_markdown("Hallo **fette** Welt\n\nzweiter");
        apply_markdown(&doc, "Hallo schöne **fette** Welt\n\nzweiter").expect("changed");
        assert_eq!(markdown_of(&doc), "Hallo schöne **fette** Welt\n\nzweiter");
        apply_markdown(&doc, "Hallo schöne **fette** Welt\n\nzweiter Absatz").expect("changed");
        assert_eq!(
            markdown_of(&doc),
            "Hallo schöne **fette** Welt\n\nzweiter Absatz"
        );
        // Marken ändern sich: wird korrekt übernommen.
        apply_markdown(&doc, "Hallo schöne fette Welt\n\nzweiter Absatz").expect("changed");
        assert_eq!(
            markdown_of(&doc),
            "Hallo schöne fette Welt\n\nzweiter Absatz"
        );
    }

    #[test]
    fn prepend_after_front_deletion() {
        let d = doc_from_markdown("A\n\nB");
        apply_markdown(&d, "B").expect("c");
        assert_eq!(markdown_of(&d), "B");
        apply_markdown(&d, "W\n\nB").expect("c");
        assert_eq!(markdown_of(&d), "W\n\nB");
    }

    #[test]
    fn prepend_multiple() {
        let d =
            doc_from_state("AQPXuI22AQAHAQdkZWZhdWx0AwlwYXJhZ3JhcGgHANe4jbYBAAYEANe4jbYBAQNlzIEA")
                .expect("s");
        apply_markdown(&d, "a\n\nb\n\nc\n\ne\u{301}").expect("c");
        assert_eq!(markdown_of(&d), "a\n\nb\n\nc\n\ne\u{301}");
    }

    #[test]
    fn prepend_multiple_plain() {
        let d = doc_from_state(&state_to_b64(&doc_from_markdown("e"))).expect("s");
        let frag = d.get_or_insert_xml_fragment(FRAGMENT);
        let blocks = pm::parse_markdown("a\n\nb\n\nc");
        let mut txn = d.transact_mut();
        let n = pm::insert_nodes(&frag, &mut txn, 0, &blocks);
        let out = pm::to_markdown(&pm::read_fragment(&frag, &txn));
        assert_eq!((n, out.as_str()), (3, "a\n\nb\n\nc\n\ne"));
    }

    #[test]
    fn diff_blocks_finds_ranges() {
        let ops = diff_blocks(&["a", "b", "c"], &["a", "x", "c", "d"]);
        assert_eq!(
            ops,
            vec![
                BlockOp {
                    old_start: 1,
                    old_len: 1,
                    new_start: 1,
                    new_len: 1
                },
                BlockOp {
                    old_start: 3,
                    old_len: 0,
                    new_start: 3,
                    new_len: 1
                },
            ]
        );
    }

    #[test]
    fn anchors_survive_edits_before_and_detach_on_delete() {
        let doc = doc_from_markdown("Hallo schöne Welt");
        let (a, b) = anchors_for_quote(&doc, "schöne").expect("anchors");
        assert_eq!(anchor_state(&doc, &a, &b), Some(AnchorState::Attached));
        // Einfügen davor: der Anker wandert mit.
        apply_markdown(&doc, "Vorwort Hallo schöne Welt").expect("changed");
        assert_eq!(anchor_state(&doc, &a, &b), Some(AnchorState::Attached));
        // Der Bereich wird entfernt: Anker lösen sich.
        apply_markdown(&doc, "Vorwort Hallo Welt").expect("changed");
        assert_eq!(anchor_state(&doc, &a, &b), Some(AnchorState::Detached));
        assert!(anchors_for_quote(&doc, "gibt es nicht").is_none());
        assert!(!anchor_is_valid("!!!"));
    }

    #[test]
    fn hostile_nesting_is_bounded() {
        // Tief verschachtelte Strukturen: nur Tiefenlimit, keine Panik, kein Stack-Überlauf.
        for unit in ["> ", "- ", "1. ", "*", "[", "# "] {
            let md = format!("{}x", unit.repeat(10_000));
            let doc = doc_from_markdown(&md);
            let _ = markdown_of(&doc);
            assert!(check_limits(&doc).is_ok(), "{unit}");
            let other = doc_from_markdown("a");
            let _ = apply_markdown(&other, &md);
            let _ = sanitized_markdown_of(&other);
        }
    }

    #[test]
    fn deep_trees_from_clients_are_detected_and_read_safely() {
        let doc = new_doc();
        let frag = doc.get_or_insert_xml_fragment(FRAGMENT);
        {
            let mut txn = doc.transact_mut();
            let mut cur = frag.insert(&mut txn, 0, yrs::XmlElementPrelim::empty("blockquote"));
            for _ in 0..10_000 {
                cur = cur.insert(&mut txn, 0, yrs::XmlElementPrelim::empty("blockquote"));
            }
        }
        assert_eq!(check_limits(&doc), Err(LimitError::TooDeep));
        // Lesen/Ableiten bleibt trotzdem sicher (Tiefenschnitt).
        let _ = markdown_of(&doc);
        let _ = state_to_b64(&doc);
        let copy = doc_from_state(&state_to_b64(&doc)).expect("state");
        assert_eq!(check_limits(&copy), Err(LimitError::TooDeep));
    }

    // --- Unicode und Fuzz ---------------------------------------------------------------

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            usize::try_from(self.next() % u64::try_from(n).unwrap_or(1)).unwrap_or(0)
        }
    }

    const PIECES: [&str; 12] = [
        "a",
        "Ä",
        "ß",
        "日本",
        "😀",
        "👨‍👩‍👧",
        "e\u{301}",
        " ",
        "**x**",
        "wort",
        "\u{1F1E9}\u{1F1EA}",
        "é",
    ];

    fn random_md(rng: &mut Rng) -> String {
        let blocks = rng.below(4) + 1;
        (0..blocks)
            .map(|_| {
                (0..rng.below(5) + 1)
                    .map(|_| PIECES[rng.below(PIECES.len())])
                    .collect::<Vec<_>>()
                    .join("")
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn sync(a: &Doc, b: &Doc) {
        let ua = a
            .transact()
            .encode_state_as_update_v1(&b.transact().state_vector());
        let ub = b
            .transact()
            .encode_state_as_update_v1(&a.transact().state_vector());
        b.transact_mut()
            .apply_update(Update::decode_v1(&ua).expect("ua"))
            .expect("apply");
        a.transact_mut()
            .apply_update(Update::decode_v1(&ub).expect("ub"))
            .expect("apply");
    }

    #[test]
    fn multibyte_texts_round_trip_and_edit_in_place() {
        for md in [
            "😀 Start",
            "Mitte 日本語 Mitte",
            "Ende 👨‍👩‍👧",
            "Ää",
            "e\u{301}x",
        ] {
            let doc = doc_from_markdown(md);
            assert_eq!(markdown_of(&doc), md);
            // Vorn, mittig und hinten einfügen bzw. löschen.
            for edit in [
                format!("😀{md}"),
                format!("{md}😀"),
                md.replacen(' ', " 日 ", 1),
                md.chars().skip(1).collect(),
            ] {
                let d = doc_from_markdown(md);
                apply_markdown(&d, &edit);
                assert_eq!(
                    markdown_of(&d),
                    markdown_of(&doc_from_markdown(&edit)),
                    "{md} -> {edit}"
                );
            }
        }
    }

    #[test]
    fn concurrent_edits_over_multibyte_text_converge() {
        let base = doc_from_markdown("Grüße 😀 aus 日本 und mehr 👨‍👩‍👧 Text");
        let a = doc_from_state(&state_to_b64(&base)).expect("a");
        let b = doc_from_state(&state_to_b64(&base)).expect("b");
        apply_markdown(&a, "Grüße aus 日本 und mehr 👨‍👩‍👧 Text");
        apply_markdown(&b, "Grüße 😀 aus 日 und mehr Text 😀");
        sync(&a, &b);
        assert_eq!(markdown_of(&a), markdown_of(&b));
        assert!(check_limits(&a).is_ok());
    }

    #[test]
    fn fuzz_two_clients_converge_without_panic() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for round in 0..300 {
            let base = doc_from_markdown(&random_md(&mut rng));
            let a = doc_from_state(&state_to_b64(&base)).expect("a");
            let b = doc_from_state(&state_to_b64(&base)).expect("b");
            let a = {
                // Gegenstelle wie ein JS-Client: mit Garbage Collection.
                let d = Doc::with_options(Options {
                    offset_kind: OffsetKind::Utf16,
                    skip_gc: false,
                    ..Options::default()
                });
                let _ = d.get_or_insert_xml_fragment(FRAGMENT);
                d.transact_mut()
                    .apply_update(
                        Update::decode_v1(&STANDARD.decode(state_to_b64(&a)).unwrap()).unwrap(),
                    )
                    .unwrap();
                d
            };
            for _ in 0..rng.below(6) + 1 {
                let target = if rng.below(2) == 0 { &a } else { &b };
                let md = random_md(&mut rng);
                apply_markdown(target, &md);
                // Abgeleitetes Markdown entspricht dem Y-Inhalt.
                assert_eq!(
                    markdown_of(target),
                    markdown_of(&doc_from_markdown(&md)),
                    "round {round}: {md:?}"
                );
                if rng.below(3) == 0 {
                    sync(&a, &b);
                }
            }
            sync(&a, &b);
            assert_eq!(markdown_of(&a), markdown_of(&b), "round {round}");
            assert!(check_limits(&a).is_ok());
            // Zustand und Neuaufbau stimmen überein.
            let again = doc_from_state(&state_to_b64(&a)).expect("state");
            assert_eq!(markdown_of(&again), markdown_of(&a));
        }
    }
}
