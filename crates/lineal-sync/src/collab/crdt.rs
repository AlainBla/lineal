//! Yjs-Dokument eines Raums: Erzeugen aus Markdown, (De-)Serialisierung des Zustands
//! (`contentState`), abgeleitetes Markdown und minimale Änderungen am Baum (API/MCP-Schreibvorgänge).

use super::pm::{self, FRAGMENT, Pm};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use lineal_domain::rules::markdown::sanitize;
use yrs::updates::decoder::Decode;
use yrs::{Doc, OffsetKind, Options, ReadTxn, StateVector, Transact, Update, XmlFragment};

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

/// Setzt den Inhalt per minimaler Blockänderung auf `new_md`: Blöcke werden über ihr Markdown
/// verglichen (LCS), nur abweichende Blöcke werden gelöscht/eingefügt; unveränderte bleiben samt
/// Cursorn und Kommentar-Ankern unberührt. Liefert das Update (V1) für die Teilnehmer, `None`
/// wenn nichts zu ändern war.
pub fn apply_markdown(doc: &Doc, new_md: &str) -> Option<Vec<u8>> {
    let frag = doc.get_or_insert_xml_fragment(FRAGMENT);
    let new_blocks: Vec<Pm> = pm::parse_markdown(new_md);
    let new_keys: Vec<String> = new_blocks.iter().map(pm::block_to_markdown).collect();
    let mut txn = doc.transact_mut();
    let before = txn.state_vector();
    let old_blocks = pm::read_fragment(&frag, &txn);
    // Leere Blöcke (z. B. der abschließende Absatz des Editors) bleiben unangetastet: sie sind im
    // Markdown unsichtbar. Wir arbeiten auf den Indizes der Quellkinder.
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
    // Von hinten nach vorn anwenden, damit Indizes gültig bleiben.
    for op in ops.into_iter().rev() {
        let (remove_from, remove_len) = if op.old_len == 0 {
            (0usize, 0usize)
        } else {
            let first = old[op.old_start].0;
            let last = old[op.old_start + op.old_len - 1].0;
            (first, last - first + 1)
        };
        let insert_at = if op.old_len == 0 {
            // Vor dem alten Block `old_start` bzw. ans Ende der inhaltlichen Blöcke.
            if op.old_start < old.len() {
                old[op.old_start].0
            } else {
                old.last().map_or(0, |(i, _)| i + 1)
            }
        } else {
            remove_from
        };
        if remove_len > 0 {
            frag.remove_range(
                &mut txn,
                u32::try_from(remove_from).unwrap_or(0),
                u32::try_from(remove_len).unwrap_or(0),
            );
        }
        let slice = &new_blocks[op.new_start..op.new_start + op.new_len];
        if !slice.is_empty() {
            pm::insert_nodes(
                &frag,
                &mut txn,
                u32::try_from(insert_at).unwrap_or(0),
                slice,
            );
        }
    }
    let update = txn.encode_state_as_update_v1(&before);
    drop(txn);
    Some(update)
}

#[derive(Debug, PartialEq, Eq)]
struct BlockOp {
    old_start: usize,
    old_len: usize,
    new_start: usize,
    new_len: usize,
}

/// Zusammenhängende Änderungsbereiche zwischen zwei Blockfolgen (LCS; bei sehr großen Eingaben
/// Präfix/Suffix-Abgleich).
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
    // LCS-Tabelle.
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
    let flush = |cur: &mut Option<BlockOp>, ops: &mut Vec<BlockOp>| {
        if let Some(op) = cur.take() {
            ops.push(op);
        }
    };
    while i < o.len() || j < n.len() {
        if i < o.len() && j < n.len() && o[i] == n[j] {
            flush(&mut cur, &mut ops);
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
    flush(&mut cur, &mut ops);
    ops
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
        // Durch Zustand und zurück.
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
        let before = doc.transact().state_vector();
        let update = apply_markdown(&doc, "eins\n\nzwei\n\nneu\n\ndrei").expect("changed");
        assert_eq!(markdown_of(&doc), "eins\n\nzwei\n\nneu\n\ndrei");
        // Nur ein Block wurde neu erzeugt: das Update ist klein.
        assert!(update.len() < 80, "update len {}", update.len());
        let _ = before;
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
    fn hostile_nesting_is_bounded() {
        // Tief verschachtelte Zitate: nur Tiefenlimit, keine Panik.
        let md = format!("{}x", "> ".repeat(500));
        let doc = doc_from_markdown(&md);
        let _ = markdown_of(&doc);
    }
}
