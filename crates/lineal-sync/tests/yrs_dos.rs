//! Regressionstests für F-02/F-03: Peer-gelieferte Längenfelder und Verschachtelungstiefe
//! dürfen im vendorierten yrs weder riesige Allokationen noch Stack-Überläufe auslösen.

use yrs::Update;
use yrs::any::Any;
use yrs::encoding::read::Cursor;
use yrs::sync::Message as YMessage;
use yrs::updates::decoder::Decode;

fn nested_any(depth: usize) -> Vec<u8> {
    // 117 = Array, Länge 1, ... , 126 = null
    let mut buf = Vec::with_capacity(depth * 2 + 1);
    for _ in 0..depth {
        buf.extend_from_slice(&[117, 1]);
    }
    buf.push(126);
    buf
}

#[test]
fn huge_length_prefix_is_rejected_without_allocation() {
    // SyncStep1 mit Zustandsvektor-Länge ~4,29 Mrd. ohne Daten (8 Bytes).
    let msg = [0x00, 0x00, 0x05, 0xff, 0xff, 0xff, 0xd0, 0x2f];
    assert!(YMessage::decode_v1(&msg).is_err());
}

#[test]
fn huge_length_prefixes_in_other_decoders_are_rejected() {
    // Update: clients_len = u32::MAX ohne Daten.
    assert!(Update::decode_v1(&[0xff, 0xff, 0xff, 0xff, 0x0f]).is_err());
    // Update: ein Client, blocks_len = u32::MAX ohne Daten.
    assert!(Update::decode_v1(&[1, 0xff, 0xff, 0xff, 0xff, 0x0f, 1, 0]).is_err());
    // Any-Array/Map mit riesiger Länge ohne Daten.
    for tag in [117u8, 118] {
        let buf = [tag, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f];
        assert!(Any::decode(&mut Cursor::new(&buf)).is_err());
    }
}

#[test]
fn deeply_nested_any_is_rejected_without_stack_overflow() {
    let buf = nested_any(300_000);
    assert!(Any::decode(&mut Cursor::new(&buf)).is_err());
}

#[test]
fn deeply_nested_any_inside_update_is_rejected() {
    // 1 Client (id 1), 1 Block (clock 0), Item mit Any-Inhalt (Ref 8), Root-Parent "t".
    let build = |depth| {
        let mut buf = vec![1, 1, 1, 0, 0x08, 1, 1, b't', 1];
        buf.extend(nested_any(depth));
        buf.push(0); // leeres Delete-Set
        buf
    };
    assert!(
        Update::decode_v1(&build(10)).is_ok(),
        "Rahmen muss gültig sein"
    );
    assert!(Update::decode_v1(&build(250_000)).is_err());
}

#[test]
fn moderate_nesting_still_decodes() {
    let buf = nested_any(100);
    assert!(Any::decode(&mut Cursor::new(&buf)).is_ok());
}

#[test]
fn invalid_utf8_in_any_string_is_rejected() {
    // Tag 119 = String, Länge 2, dann ungültige UTF-8-Bytes: muss Fehler sein, kein UB.
    let buf = [119u8, 2, 0xff, 0xfe];
    assert!(Any::decode(&mut Cursor::new(&buf)).is_err());
    // Gültiger String dekodiert weiterhin.
    let ok = [119u8, 2, b'h', b'i'];
    assert!(Any::decode(&mut Cursor::new(&ok)).is_ok());
}

/// Ein tief verschachtelter XML-Baum als Update. Gebaut auf großem Stack, damit nur die Empfängerseite
/// geprüft wird.
fn nested_tree(depth: usize, root: &'static str, delete: bool) -> Vec<u8> {
    use yrs::{Doc, Options, ReadTxn, StateVector, Transact, XmlElementPrelim, XmlFragment};
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let doc = Doc::with_options(Options {
                client_id: 1,
                skip_gc: true,
                ..Options::default()
            });
            let frag = doc.get_or_insert_xml_fragment(root);
            {
                let mut txn = doc.transact_mut();
                let mut el = frag.insert(&mut txn, 0, XmlElementPrelim::empty("a"));
                for _ in 1..depth {
                    el = el.insert(&mut txn, 0, XmlElementPrelim::empty("a"));
                }
            }
            if delete {
                frag.remove_range(&mut doc.transact_mut(), 0, 1);
            }
            doc.transact()
                .encode_state_as_update_v1(&StateVector::default())
        })
        .expect("thread")
        .join()
        .expect("build")
}

#[test]
fn deleting_a_deep_tree_does_not_overflow_the_stack() {
    use yrs::{Doc, Transact};
    // Knapp unter `max_update_bytes`: Das Limit lässt so ein Update durch.
    let update = nested_tree(60_000, "default", true);
    assert!(update.len() < 512 * 1024, "{}", update.len());
    // Ein Stack-Überlauf bricht den ganzen Prozess ab und damit diesen Test.
    let applied = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let doc = Doc::new();
            doc.get_or_insert_xml_fragment("default");
            let update = Update::decode_v1(&update).expect("decode");
            doc.transact_mut().apply_update(update).is_ok()
        })
        .expect("thread")
        .join()
        .expect("apply");
    assert!(applied);
}
