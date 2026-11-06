//! Serverbound packets of Handshake, Status, Login and Configuration.
//!
//! First input byte selects the packet type (see `solaris_fuzz::PRE_PLAY`);
//! the rest is the packet body exactly as the framing layer hands it over.
//! Property: decoding never panics or over-allocates.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| solaris_fuzz::run(solaris_fuzz::PRE_PLAY, data));
