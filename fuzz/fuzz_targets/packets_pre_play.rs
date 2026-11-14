//! Serverbound packets of Handshake, Status, Login and Configuration.
//!
//! First input byte selects the packet type (see `krasnokamen_fuzz::PRE_PLAY`);
//! the rest is the packet body exactly as the framing layer hands it over.
//! Property: decoding never panics or over-allocates.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| krasnokamen_fuzz::run(krasnokamen_fuzz::PRE_PLAY, data));
