//! Edge-side serverbound Play decode: `mc_edge::decode::decode_play_frame`,
//! the pure frame -> `PlayerInput` mapping every Play frame passes through.
//!
//! First input byte selects a serverbound packet id from
//! `solaris_fuzz::PLAY_IDS`; the rest is the body. Property: a malformed body
//! is `Err`, never a panic or unbounded allocation.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mc_core::boundary::ConnectionId;
use mc_edge::decode::{DecodeState, decode_play_frame};
use solaris_fuzz::PLAY_IDS;

fuzz_target!(|data: &[u8]| {
    let Some((&selector, body)) = data.split_first() else {
        return;
    };
    let id = PLAY_IDS[usize::from(selector) % PLAY_IDS.len()];
    let mut state = DecodeState::default();
    let mut out = Vec::new();
    let _ = decode_play_frame(ConnectionId(1), id, body, &mut state, &mut out);
});
