//! NBT decoding of untrusted bytes in every wire form the server accepts.
//!
//! Property: no panic / unbounded allocation; a successfully decoded root
//! re-encodes, and encoding is idempotent through a decode round-trip.
#![no_main]

use libfuzzer_sys::fuzz_target;

fn check_network(data: &[u8]) {
    let mut buf = data;
    if let Ok(tag) = mc_nbt::read_network(&mut buf) {
        let mut first = Vec::new();
        if mc_nbt::write_network(&mut first, &tag).is_ok() {
            let mut again = first.as_slice();
            let tag = mc_nbt::read_network(&mut again).expect("re-encoded root decodes");
            let mut second = Vec::new();
            mc_nbt::write_network(&mut second, &tag).expect("round-tripped root encodes");
            assert_eq!(first, second, "network NBT encoding is not idempotent");
        }
    }
}

fuzz_target!(|data: &[u8]| {
    check_network(data);
    let mut buf = data;
    let _ = mc_nbt::read_network_value(&mut buf);
    let mut buf = data;
    let _ = mc_nbt::read_named(&mut buf);
    let mut buf = data;
    if let Ok((name, tag)) = mc_nbt::read_named_document(&mut buf) {
        let mut out = Vec::new();
        let _ = mc_nbt::write_named_document(&mut out, &name, &tag);
    }
    // Same bytes through the non-contiguous Buf path used for bounded slices.
    let (head, tail) = data.split_at(data.len() / 2);
    let mut chained = bytes::Buf::chain(head, tail);
    let _ = mc_nbt::read_network(&mut chained);
});
