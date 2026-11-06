//! Loader client messages carried in custom payloads (Configuration ack and
//! artifact request, Play view request): JSON documents the client controls.
//!
//! First input byte selects the decoder; the rest is the payload. Property:
//! decoding never panics or over-allocates; unknown/oversized input is an error.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mc_loader::{LoaderArtifactRequest, LoaderClientAck, decode_loader_view_client_request};

fuzz_target!(|data: &[u8]| {
    let Some((&selector, payload)) = data.split_first() else {
        return;
    };
    match selector % 3 {
        0 => {
            let _ = LoaderClientAck::decode(payload);
        }
        1 => {
            let _ = LoaderArtifactRequest::decode(payload);
        }
        _ => {
            let _ = decode_loader_view_client_request(payload);
        }
    }
});
