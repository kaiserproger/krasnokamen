//! Serverbound Play packets, including the item-stack / data-component and
//! nested NBT decoders they reach (creative slots, container clicks, books,
//! signs, chat sessions, custom payloads).
//!
//! First input byte selects the packet type (see `solaris_fuzz::PLAY`); the
//! rest is the body. Property: decoding never panics or over-allocates.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| solaris_fuzz::run(solaris_fuzz::PLAY, data));
