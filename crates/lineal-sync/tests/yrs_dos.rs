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
