//! Patch-Operationen für Rich-Text (docs/07 §2.6, docs/00 §6).
//!
//! Reine, atomare Funktion: Bei einem Fehler wird **kein** Ergebnis geliefert, der Eingabetext
//! bleibt unverändert (der Adapter schreibt nichts). Fehler entsprechen `conflict` (409) mit
//! `details: { opIndex, op, anchor, matches }`.

use serde::{Deserialize, Serialize};

/// Mindest-/Höchstzahl Operationen pro Patch.
pub const MAX_OPS: usize = 50;
/// Maximale Textlänge in Zeichen (wie die Issue-Beschreibung), für Eingabe und Ergebnis.
pub const MAX_TEXT_CHARS: usize = 500_000;
/// Obergrenze in Bytes (UTF-8, höchstens 4 Bytes je Zeichen); schützt vor Allokationen.
pub const MAX_TEXT_BYTES: usize = MAX_TEXT_CHARS * 4;
/// Maximale Länge eines Ankers (`old_string`, `anchor`, `from`, `to`) in Bytes.
pub const MAX_ANCHOR_BYTES: usize = 10_000;

/// Eine Patch-Operation (JSON-Form wie in docs/07 §2.6, Feld `op` als Tag).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum PatchOp {
    /// `old_string` genau einmal ersetzen (oder mit `replace_all` alle nicht überlappenden).
    Replace {
        /// Zu ersetzender Text (nicht leer).
        old_string: String,
        /// Ersatz (leer = löschen).
        new_string: String,
        /// Alle Vorkommen ersetzen (mindestens eines nötig).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        replace_all: Option<bool>,
    },
    /// Text direkt vor dem Anker einfügen.
    InsertBefore {
        /// Anker (nicht leer, genau einmal).
        anchor: String,
        /// Einzufügender Text (nicht leer).
        text: String,
    },
    /// Text direkt nach dem Anker einfügen.
    InsertAfter {
        /// Anker (nicht leer, genau einmal).
        anchor: String,
        /// Einzufügender Text (nicht leer).
        text: String,
    },
    /// Text ganz am Anfang einfügen.
    Prepend {
        /// Einzufügender Text (nicht leer).
        text: String,
    },
    /// Text ganz am Ende einfügen.
    Append {
        /// Einzufügender Text (nicht leer).
        text: String,
    },
    /// `[start(from), start(to))` ersetzen; `to` bleibt stehen.
    ReplaceRange {
        /// Beginn (inklusiv, genau einmal).
        from: String,
        /// Ende (exklusiv, genau einmal **nach** `from`).
        to: String,
        /// Ersatz (leer = löschen).
        new_string: String,
    },
}

impl PatchOp {
    /// Anker-Strings der Operation.
    fn anchors(&self) -> Vec<&str> {
        match self {
            Self::Replace { old_string, .. } => vec![old_string],
            Self::InsertBefore { anchor, .. } | Self::InsertAfter { anchor, .. } => vec![anchor],
            Self::ReplaceRange { from, to, .. } => vec![from, to],
            Self::Prepend { .. } | Self::Append { .. } => vec![],
        }
    }

    /// Einzufügende/ersetzende Texte der Operation.
    fn texts(&self) -> Vec<&str> {
        match self {
            Self::Replace { new_string, .. } | Self::ReplaceRange { new_string, .. } => {
                vec![new_string]
            }
            Self::InsertBefore { text, .. }
            | Self::InsertAfter { text, .. }
            | Self::Prepend { text }
            | Self::Append { text } => vec![text],
        }
    }

    /// Name der Operation (`op`).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Replace { .. } => "replace",
            Self::InsertBefore { .. } => "insert_before",
            Self::InsertAfter { .. } => "insert_after",
            Self::Prepend { .. } => "prepend",
            Self::Append { .. } => "append",
            Self::ReplaceRange { .. } => "replace_range",
        }
    }
}

/// Grund eines Patch-Fehlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchFailure {
    /// Keine Operation übergeben.
    NoOps,
    /// Mehr als [`MAX_OPS`] Operationen.
    TooManyOps,
    /// Pflichtstring (Anker, `old_string`, `text`, `from`, `to`) ist leer.
    EmptyArgument,
    /// Anker kommt nicht vor.
    AnchorNotFound,
    /// Anker kommt mehrfach vor (auch überlappend gezählt).
    AnchorAmbiguous,
    /// Größenlimit überschritten (Text, Anker oder Ergebnis) → `limit_exceeded`.
    LimitExceeded,
}

/// Fehler einer Patch-Operation (`conflict`, docs/07 §2.6).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("patch operation {op_index} ({op}): {reason_text}", reason_text = self.reason_text())]
pub struct PatchError {
    /// Index der scheiternden Operation (0-basiert).
    pub op_index: usize,
    /// Name der Operation.
    pub op: &'static str,
    /// Grund.
    pub reason: PatchFailure,
    /// Der betroffene Anker (bei Anker-Fehlern).
    pub anchor: Option<String>,
    /// Anzahl gefundener Treffer (bei Anker-Fehlern).
    pub matches: Option<usize>,
}

impl PatchError {
    fn reason_text(&self) -> String {
        match self.reason {
            PatchFailure::NoOps => "at least one operation is required".into(),
            PatchFailure::TooManyOps => format!("at most {MAX_OPS} operations are allowed"),
            PatchFailure::EmptyArgument => "a required string is empty".into(),
            PatchFailure::LimitExceeded => format!(
                "size limit exceeded (text/result max {MAX_TEXT_CHARS} characters, anchors max {MAX_ANCHOR_BYTES} bytes)"
            ),
            PatchFailure::AnchorNotFound | PatchFailure::AnchorAmbiguous => format!(
                "anchor must match the current content exactly once, found {} matches",
                self.matches.unwrap_or(0)
            ),
        }
    }
}

fn err(op_index: usize, op: &PatchOp, reason: PatchFailure) -> PatchError {
    PatchError {
        op_index,
        op: op.name(),
        reason,
        anchor: None,
        matches: None,
    }
}

fn anchor_err(i: usize, op: &PatchOp, anchor: &str, matches: usize) -> PatchError {
    PatchError {
        op_index: i,
        op: op.name(),
        reason: if matches == 0 {
            PatchFailure::AnchorNotFound
        } else {
            PatchFailure::AnchorAmbiguous
        },
        anchor: Some(anchor.to_owned()),
        matches: Some(matches),
    }
}

fn norm(s: &str) -> String {
    s.replace("\r\n", "\n")
}

/// Zählt alle Treffer **inklusive überlappender** in linearer Zeit (KMP, O(n + m)) und liefert
/// die Zahl sowie die erste Position (Byte-Offset). UTF-8 ist selbstsynchronisierend, Treffer
/// liegen daher immer auf Zeichengrenzen.
fn count_matches(hay: &str, needle: &str) -> (usize, usize) {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    let mut fail = vec![0usize; n.len()];
    let mut k = 0;
    for i in 1..n.len() {
        while k > 0 && n[i] != n[k] {
            k = fail[k - 1];
        }
        if n[i] == n[k] {
            k += 1;
        }
        fail[i] = k;
    }
    let (mut count, mut first, mut k) = (0, 0, 0);
    for (i, &c) in h.iter().enumerate() {
        while k > 0 && c != n[k] {
            k = fail[k - 1];
        }
        if c == n[k] {
            k += 1;
        }
        if k == n.len() {
            if count == 0 {
                first = i + 1 - n.len();
            }
            count += 1;
            k = fail[k - 1];
        }
    }
    (count, first)
}

/// Genau ein Treffer ab Offset `from` → absolute Position, sonst Trefferzahl.
fn find_unique(buf: &str, needle: &str, from: usize) -> Result<usize, usize> {
    match count_matches(&buf[from..], needle) {
        (1, p) => Ok(from + p),
        (n, _) => Err(n),
    }
}

/// Wendet `ops` sequenziell und atomar auf `text` an (docs/07 §2.6).
///
/// * Zeilenenden werden vorab auf `\n` normalisiert (auch in Ankern/Texten der Operationen).
/// * Jeder Anker wird gegen den Puffer **nach den vorherigen Operationen** geprüft und muss
///   **genau einmal** vorkommen (überlappende Treffer zählen mit). Ausnahme: `replace` mit
///   `replace_all: true` (mindestens ein Treffer; ersetzt alle nicht überlappenden).
/// * `replace_range`: `from` genau einmal, `to` genau einmal **nach dem Ende von `from`**.
/// * Fehler → `Err(PatchError)`; es gibt kein Teilergebnis, `text` bleibt unberührt.
/// * Das Ergebnis darf leer sein.
pub fn apply(text: &str, ops: &[PatchOp]) -> Result<String, PatchError> {
    if ops.is_empty() {
        return Err(PatchError {
            op_index: 0,
            op: "",
            reason: PatchFailure::NoOps,
            anchor: None,
            matches: None,
        });
    }
    if ops.len() > MAX_OPS {
        return Err(PatchError {
            op_index: MAX_OPS,
            op: ops[MAX_OPS].name(),
            reason: PatchFailure::TooManyOps,
            anchor: None,
            matches: None,
        });
    }
    let mut buf = norm(text);
    if buf.len() > MAX_TEXT_BYTES || buf.chars().count() > MAX_TEXT_CHARS {
        return Err(PatchError {
            op_index: 0,
            op: ops[0].name(),
            reason: PatchFailure::LimitExceeded,
            anchor: None,
            matches: None,
        });
    }
    for (i, op) in ops.iter().enumerate() {
        // Argumentgrößen vor jeder Normalisierung/Allokation prüfen.
        if op.anchors().iter().any(|a| a.len() > MAX_ANCHOR_BYTES)
            || op.texts().iter().any(|t| t.len() > MAX_TEXT_BYTES)
        {
            return Err(err(i, op, PatchFailure::LimitExceeded));
        }
        let room = |projected: usize| -> Result<(), PatchError> {
            if projected > MAX_TEXT_BYTES {
                Err(err(i, op, PatchFailure::LimitExceeded))
            } else {
                Ok(())
            }
        };
        let empty = |s: &str| -> Result<(), PatchError> {
            if s.is_empty() {
                Err(err(i, op, PatchFailure::EmptyArgument))
            } else {
                Ok(())
            }
        };
        match op {
            PatchOp::Replace {
                old_string,
                new_string,
                replace_all,
            } => {
                let (old, new) = (norm(old_string), norm(new_string));
                empty(&old)?;
                if replace_all.unwrap_or(false) {
                    let count = buf.matches(old.as_str()).count();
                    if count == 0 {
                        return Err(anchor_err(i, op, &old, 0));
                    }
                    // Wachstum vor der Allokation abschätzen (Treffer × (neu − alt)).
                    let grow = count.saturating_mul(new.len().saturating_sub(old.len()));
                    room(buf.len().saturating_add(grow))?;
                    buf = buf.replace(&old, &new);
                } else {
                    let p = find_unique(&buf, &old, 0).map_err(|n| anchor_err(i, op, &old, n))?;
                    room(buf.len().saturating_add(new.len()))?;
                    buf.replace_range(p..p + old.len(), &new);
                }
            }
            PatchOp::InsertBefore { anchor, text } => {
                let (anchor, text) = (norm(anchor), norm(text));
                empty(&anchor)?;
                empty(&text)?;
                let p = find_unique(&buf, &anchor, 0).map_err(|n| anchor_err(i, op, &anchor, n))?;
                room(buf.len().saturating_add(text.len()))?;
                buf.insert_str(p, &text);
            }
            PatchOp::InsertAfter { anchor, text } => {
                let (anchor, text) = (norm(anchor), norm(text));
                empty(&anchor)?;
                empty(&text)?;
                let p = find_unique(&buf, &anchor, 0).map_err(|n| anchor_err(i, op, &anchor, n))?;
                room(buf.len().saturating_add(text.len()))?;
                buf.insert_str(p + anchor.len(), &text);
            }
            PatchOp::Prepend { text } => {
                let text = norm(text);
                empty(&text)?;
                room(buf.len().saturating_add(text.len()))?;
                buf.insert_str(0, &text);
            }
            PatchOp::Append { text } => {
                let text = norm(text);
                empty(&text)?;
                room(buf.len().saturating_add(text.len()))?;
                buf.push_str(&text);
            }
            PatchOp::ReplaceRange {
                from,
                to,
                new_string,
            } => {
                let (from, to, new) = (norm(from), norm(to), norm(new_string));
                empty(&from)?;
                empty(&to)?;
                let a = find_unique(&buf, &from, 0).map_err(|n| anchor_err(i, op, &from, n))?;
                let b = find_unique(&buf, &to, a + from.len())
                    .map_err(|n| anchor_err(i, op, &to, n))?;
                room(buf.len().saturating_add(new.len()))?;
                buf.replace_range(a..b, &new);
            }
        }
        if buf.chars().count() > MAX_TEXT_CHARS {
            return Err(err(i, op, PatchFailure::LimitExceeded));
        }
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replace(old: &str, new: &str) -> PatchOp {
        PatchOp::Replace {
            old_string: old.into(),
            new_string: new.into(),
            replace_all: None,
        }
    }
    fn replace_all(old: &str, new: &str) -> PatchOp {
        PatchOp::Replace {
            old_string: old.into(),
            new_string: new.into(),
            replace_all: Some(true),
        }
    }
    fn before(a: &str, t: &str) -> PatchOp {
        PatchOp::InsertBefore {
            anchor: a.into(),
            text: t.into(),
        }
    }
    fn append(t: &str) -> PatchOp {
        PatchOp::Append { text: t.into() }
    }
    fn after(a: &str, t: &str) -> PatchOp {
        PatchOp::InsertAfter {
            anchor: a.into(),
            text: t.into(),
        }
    }
    fn range(f: &str, t: &str, n: &str) -> PatchOp {
        PatchOp::ReplaceRange {
            from: f.into(),
            to: t.into(),
            new_string: n.into(),
        }
    }

    const DOC: &str = "# Titel\n\n## Schritte\n\n1. Eins\n2. Zwei\n\n## Schritte\n\nEnde\n";

    #[test]
    fn each_op_happy_path() {
        assert_eq!(apply("a b c", &[replace("b", "X")]).unwrap(), "a X c");
        assert_eq!(apply("a b c", &[replace("b ", "")]).unwrap(), "a c");
        assert_eq!(apply("a b c", &[before("b", "<")]).unwrap(), "a <b c");
        assert_eq!(apply("a b c", &[after("b", ">")]).unwrap(), "a b> c");
        assert_eq!(
            apply("a", &[PatchOp::Prepend { text: "x".into() }]).unwrap(),
            "xa"
        );
        assert_eq!(
            apply("a", &[PatchOp::Append { text: "x".into() }]).unwrap(),
            "ax"
        );
        assert_eq!(
            apply("k [A] mid [B] z", &[range("[A]", "[B]", "-")]).unwrap(),
            "k -[B] z"
        );
        assert_eq!(apply("abc", &[replace("abc", "")]).unwrap(), ""); // leeres Ergebnis erlaubt
        assert_eq!(
            apply("", &[PatchOp::Append { text: "x".into() }]).unwrap(),
            "x"
        );
    }

    /// Review-Fokus 4: mehrdeutiger Anker -> Fehler, Text unverändert.
    #[test]
    fn ambiguous_anchor_fails_and_text_is_unchanged() {
        let original = DOC.to_string();
        let ops = [replace("Ende", "Schluss"), after("## Schritte", "\n\nNeu")];
        let e = apply(&original, &ops).unwrap_err();
        assert_eq!(e.op_index, 1);
        assert_eq!(e.op, "insert_after");
        assert_eq!(e.reason, PatchFailure::AnchorAmbiguous);
        assert_eq!(e.matches, Some(2));
        assert_eq!(e.anchor.as_deref(), Some("## Schritte"));
        assert_eq!(original, DOC); // Eingabe unberührt (reine Funktion)
        // Atomar: ein erfolgreicher Op davor bleibt ohne Wirkung -> der Aufrufer behält `original`.
        assert!(e.to_string().contains("found 2 matches"));
    }

    #[test]
    fn missing_anchor_reports_zero_matches() {
        let e = apply("abc", &[before("zzz", "x")]).unwrap_err();
        assert_eq!(e.reason, PatchFailure::AnchorNotFound);
        assert_eq!(e.matches, Some(0));
        let e = apply("abc", &[replace("zzz", "x")]).unwrap_err();
        assert_eq!(
            (e.reason, e.matches),
            (PatchFailure::AnchorNotFound, Some(0))
        );
    }

    #[test]
    fn overlapping_occurrences_count() {
        // "aa" kommt in "aaa" überlappend zweimal vor -> mehrdeutig.
        let e = apply("aaa", &[replace("aa", "b")]).unwrap_err();
        assert_eq!(e.matches, Some(2));
        let e = apply("aaaa", &[after("aa", "x")]).unwrap_err();
        assert_eq!(e.matches, Some(3));
        // Eindeutig, obwohl Teilstring von sich selbst nicht überlappt.
        assert_eq!(apply("abab", &[replace("bab", "X")]).unwrap(), "aX");
    }

    #[test]
    fn overlapping_count_is_char_boundary_safe() {
        assert_eq!(
            apply("äää", &[replace("ä", "x")]).unwrap_err().matches,
            Some(3)
        );
        assert_eq!(apply("a😀b", &[replace("😀", "x")]).unwrap(), "axb");
    }

    #[test]
    fn replace_all_replaces_non_overlapping_left_to_right() {
        assert_eq!(apply("aaa", &[replace_all("aa", "b")]).unwrap(), "ba");
        assert_eq!(apply("x-x-x", &[replace_all("x", "y")]).unwrap(), "y-y-y");
        let e = apply("abc", &[replace_all("q", "y")]).unwrap_err();
        assert_eq!(e.reason, PatchFailure::AnchorNotFound);
    }

    #[test]
    fn ops_apply_sequentially_on_the_updated_buffer() {
        // Zweiter Anker existiert erst durch die erste Op.
        let out = apply(
            "alpha",
            &[replace("alpha", "beta GAMMA"), replace("GAMMA", "delta")],
        )
        .unwrap();
        assert_eq!(out, "beta delta");
        // Erste Op erzeugt Mehrdeutigkeit für die zweite.
        let e = apply("x y", &[replace("y", "x"), replace("x", "z")]).unwrap_err();
        assert_eq!((e.op_index, e.matches), (1, Some(2)));
        // Erste Op beseitigt die Mehrdeutigkeit.
        let out = apply("x x", &[replace("x x", "x"), replace("x", "z")]).unwrap();
        assert_eq!(out, "z");
    }

    #[test]
    fn failing_later_op_discards_all_earlier_ops() {
        let src = "one two";
        let r = apply(
            src,
            &[
                replace("one", "1"),
                replace("two", "2"),
                replace("three", "3"),
            ],
        );
        let e = r.unwrap_err();
        assert_eq!(e.op_index, 2);
        assert_eq!(src, "one two");
    }

    #[test]
    fn replace_range_semantics() {
        // `to` bleibt stehen, `from` wird mit ersetzt.
        assert_eq!(
            apply("A1 xx B2 yy", &[range("A1", "B2", "")]).unwrap(),
            "B2 yy"
        );
        // `to` muss NACH from gesucht werden: ein früheres Vorkommen von `to` zählt nicht.
        assert_eq!(
            apply("END a START b END c", &[range("START", "END", "-")]).unwrap(),
            "END a -END c"
        );
        // `to` mehrfach nach from -> mehrdeutig (Fehler nennt `to` als Anker).
        let e = apply("S a E b E", &[range("S", "E", "")]).unwrap_err();
        assert_eq!(
            (e.op_index, e.matches, e.anchor.as_deref()),
            (0, Some(2), Some("E"))
        );
        // `to` nicht nach from vorhanden.
        let e = apply("E a S b", &[range("S", "E", "")]).unwrap_err();
        assert_eq!(e.reason, PatchFailure::AnchorNotFound);
        // `from` mehrdeutig.
        let e = apply("S S E", &[range("S", "E", "")]).unwrap_err();
        assert_eq!((e.matches, e.anchor.as_deref()), (Some(2), Some("S")));
        // `to` überlappt nicht mit from (beginnt erst nach dem Ende von from).
        let e = apply("abc", &[range("ab", "b", "")]).unwrap_err();
        assert_eq!(e.reason, PatchFailure::AnchorNotFound);
    }

    #[test]
    fn crlf_is_normalized() {
        assert_eq!(apply("a\r\nb\r\n", &[replace("a\nb", "X")]).unwrap(), "X\n");
        assert_eq!(apply("a\nb", &[replace("a\r\nb", "X")]).unwrap(), "X");
        assert_eq!(
            apply(
                "a",
                &[PatchOp::Append {
                    text: "\r\nb".into()
                }]
            )
            .unwrap(),
            "a\nb"
        );
    }

    #[test]
    fn matching_is_exact_and_case_sensitive() {
        assert!(apply("Hello", &[replace("hello", "x")]).is_err());
        assert!(apply("a  b", &[replace("a b", "x")]).is_err()); // Whitespace nicht normalisiert
    }

    #[test]
    fn empty_arguments_and_op_count_limits() {
        assert_eq!(apply("a", &[]).unwrap_err().reason, PatchFailure::NoOps);
        let too_many = vec![PatchOp::Append { text: "x".into() }; MAX_OPS + 1];
        assert_eq!(
            apply("", &too_many).unwrap_err().reason,
            PatchFailure::TooManyOps
        );
        let max = vec![PatchOp::Append { text: "x".into() }; MAX_OPS];
        assert_eq!(apply("", &max).unwrap().len(), MAX_OPS);
        for op in [
            replace("", "x"),
            before("", "x"),
            before("a", ""),
            after("", "x"),
            PatchOp::Prepend { text: "".into() },
            PatchOp::Append { text: "".into() },
            range("", "a", ""),
            range("a", "", ""),
        ] {
            let e = apply("a", &[op]).unwrap_err();
            assert_eq!(e.reason, PatchFailure::EmptyArgument);
        }
    }

    #[test]
    fn serde_wire_form_matches_spec() {
        let ops: Vec<PatchOp> = serde_json::from_str(
            r#"[
              {"op":"replace","old_string":"a","new_string":"b","replace_all":true},
              {"op":"insert_before","anchor":"a","text":"t"},
              {"op":"insert_after","anchor":"a","text":"t"},
              {"op":"prepend","text":"t"},
              {"op":"append","text":"t"},
              {"op":"replace_range","from":"a","to":"b","new_string":""}
            ]"#,
        )
        .unwrap();
        assert_eq!(ops.len(), 6);
        assert_eq!(ops[0], replace_all("a", "b"));
        assert_eq!(ops[5], range("a", "b", ""));
        // Unbekannte Felder und Ops werden abgelehnt.
        assert!(serde_json::from_str::<PatchOp>(r#"{"op":"append","text":"t","x":1}"#).is_err());
        assert!(serde_json::from_str::<PatchOp>(r#"{"op":"delete","text":"t"}"#).is_err());
        assert_eq!(
            serde_json::to_string(&PatchOp::Append { text: "t".into() }).unwrap(),
            r#"{"op":"append","text":"t"}"#
        );
    }

    #[test]
    fn pathological_anchors_run_in_linear_time() {
        // Quadratisch wäre: Anker "aaa…a" in Text "aaa…a" mit überlappenden Treffern.
        let text = "a".repeat(400_000);
        let anchor = "a".repeat(9_000);
        let started = std::time::Instant::now();
        let e = apply(&text, &[replace(&anchor, "x")]).unwrap_err();
        assert_eq!(e.reason, PatchFailure::AnchorAmbiguous);
        assert_eq!(e.matches, Some(400_000 - 9_000 + 1));
        let text2 = format!("{}b", "a".repeat(400_000));
        assert!(apply(&text2, &[after(&anchor, "x")]).is_err());
        assert!(apply(&text, &[range(&"a".repeat(5000), &"a".repeat(5000), "")]).is_err());
        assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
    }

    #[test]
    fn kmp_counts_match_naive() {
        for (h, n) in [
            ("aaaa", "aa"),
            ("abababa", "aba"),
            ("äää", "ä"),
            ("abc", "d"),
            ("a", "abc"),
        ] {
            let naive = (0..h.len())
                .filter(|i| h.is_char_boundary(*i) && h[*i..].starts_with(n))
                .count();
            assert_eq!(count_matches(h, n).0, naive, "{h} {n}");
        }
    }

    #[test]
    fn size_limits_for_input_anchors_and_result() {
        let big = "x".repeat(MAX_TEXT_CHARS + 1);
        assert_eq!(
            apply(&big, &[append("y")]).unwrap_err().reason,
            PatchFailure::LimitExceeded
        );
        let long_anchor = "x".repeat(MAX_ANCHOR_BYTES + 1);
        assert_eq!(
            apply("x", &[replace(&long_anchor, "y")])
                .unwrap_err()
                .reason,
            PatchFailure::LimitExceeded
        );
        let max = "x".repeat(MAX_TEXT_CHARS);
        assert_eq!(
            apply(&max[..MAX_TEXT_CHARS - 1], &[append("y")])
                .unwrap()
                .len(),
            MAX_TEXT_CHARS
        );
        assert_eq!(
            apply(&max, &[append("y")]).unwrap_err().reason,
            PatchFailure::LimitExceeded
        );
    }

    #[test]
    fn amplification_is_stopped_and_text_unchanged() {
        let text = "a".repeat(100_000);
        let e = apply(&text, &[replace_all("a", &"b".repeat(100))]).unwrap_err();
        assert_eq!(e.reason, PatchFailure::LimitExceeded);
        assert_eq!(e.op_index, 0);
        let ops = vec![replace_all("a", "aa"); 30];
        let e = apply("aaaa", &ops).unwrap_err();
        assert_eq!(e.reason, PatchFailure::LimitExceeded);
        assert!(e.op_index > 0 && e.op_index < 30);
        let chunk = "z".repeat(200_000);
        let ops: Vec<PatchOp> = (0..5).map(|_| append(&chunk)).collect();
        assert_eq!(
            apply("", &ops).unwrap_err().reason,
            PatchFailure::LimitExceeded
        );
        assert_eq!(text.len(), 100_000);
    }
}
