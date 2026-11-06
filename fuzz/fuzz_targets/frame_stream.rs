//! Frame length / compression layer: `try_decode_frame` fed by a byte stream.
//!
//! Property: arbitrary stream bytes (or a valid frame sequence with bytes
//! corrupted) never panic or over-allocate, and decoding the stream in
//! arbitrary read-sized chunks yields exactly the frames decoding it in one
//! piece yields (same frames, same error-or-incomplete verdict). Uncorrupted
//! valid frames must round-trip.
#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use bytes::{Bytes, BytesMut};
use libfuzzer_sys::fuzz_target;
use mc_protocol::frame::{Compression, RawFrame, encode_frame, try_decode_frame};

#[derive(Debug, Arbitrary)]
enum Mode {
    /// Raw wire bytes.
    Raw(Vec<u8>),
    /// Valid frames encoded by the server encoder, then optionally corrupted.
    Valid {
        frames: Vec<(i32, Vec<u8>, u8)>,
        corrupt: Vec<(u16, u8)>,
    },
}

#[derive(Debug, Arbitrary)]
struct Input {
    compression: u8,
    threshold: u16,
    level: u8,
    chunk_sizes: Vec<u8>,
    mode: Mode,
}

fn compression(input: &Input) -> Compression {
    let threshold = usize::from(input.threshold);
    match input.compression % 5 {
        0 => Compression::Disabled,
        1 => Compression::Threshold(0),
        2 => Compression::Threshold(1),
        3 => Compression::Threshold(threshold),
        _ => Compression::ThresholdWithLevel {
            threshold,
            level: u32::from(input.level % 10),
        },
    }
}

/// Decodes `wire` all at once; returns frames and whether it ended in an error.
fn decode_whole(wire: &[u8], compression: Compression) -> (Vec<RawFrame>, bool) {
    let mut buf = BytesMut::from(wire);
    let mut frames = Vec::new();
    loop {
        match try_decode_frame(&mut buf, compression) {
            Ok(Some(frame)) => frames.push(frame),
            Ok(None) => return (frames, false),
            Err(_) => return (frames, true),
        }
    }
}

/// Decodes `wire` in read-sized chunks, like the edge socket loop.
fn decode_chunked(wire: &[u8], compression: Compression, sizes: &[u8]) -> (Vec<RawFrame>, bool) {
    let mut buf = BytesMut::new();
    let mut frames = Vec::new();
    let mut offset = 0;
    let mut size_index = 0;
    while offset < wire.len() {
        let size = usize::from(
            sizes
                .get(size_index % sizes.len().max(1))
                .copied()
                .unwrap_or(1),
        )
        .max(1);
        size_index += 1;
        let end = (offset + size).min(wire.len());
        buf.extend_from_slice(&wire[offset..end]);
        offset = end;
        loop {
            match try_decode_frame(&mut buf, compression) {
                Ok(Some(frame)) => frames.push(frame),
                Ok(None) => break,
                Err(_) => return (frames, true),
            }
        }
    }
    (frames, false)
}

fuzz_target!(|data: &[u8]| {
    // Byte 0 bit 7 clear: raw wire stream with a 4-byte header (compression,
    // threshold, chunk size) so valid streams are directly mutable seeds.
    // Bit 7 set: structure-aware `Input` built from the remaining bytes.
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let input = if mode & 0x80 == 0 {
        let [threshold_lo, threshold_hi, chunk, wire @ ..] = rest else {
            return;
        };
        Input {
            compression: mode,
            threshold: u16::from_le_bytes([*threshold_lo, *threshold_hi]),
            level: mode >> 3,
            chunk_sizes: vec![*chunk],
            mode: Mode::Raw(wire.to_vec()),
        }
    } else {
        match Input::arbitrary(&mut Unstructured::new(rest)) {
            Ok(input) => input,
            Err(_) => return,
        }
    };
    check(&input);
});

fn check(input: &Input) {
    let compression = compression(input);
    let (wire, expected) = match &input.mode {
        Mode::Raw(bytes) => (bytes.clone(), None),
        Mode::Valid { frames, corrupt } => {
            let mut wire = Vec::new();
            let mut expected = Vec::new();
            for (id, body, repeat) in frames {
                // Repeating makes bodies compressible and large enough to cross thresholds.
                let body = body.repeat(1 + usize::from(*repeat));
                if let Ok(encoded) = encode_frame(*id, &body, compression) {
                    wire.extend_from_slice(&encoded);
                    expected.push(RawFrame {
                        id: *id,
                        body: Bytes::from(body),
                    });
                }
            }
            let corrupted = !corrupt.is_empty() && !wire.is_empty();
            for (position, value) in corrupt {
                if wire.is_empty() {
                    break;
                }
                let index = usize::from(*position) % wire.len();
                wire[index] ^= *value;
            }
            (wire, (!corrupted).then_some(expected))
        }
    };

    let whole = decode_whole(&wire, compression);
    let chunked = decode_chunked(&wire, compression, &input.chunk_sizes);
    assert_eq!(
        whole.0, chunked.0,
        "chunked decode produced different frames"
    );
    assert_eq!(
        whole.1, chunked.1,
        "chunked decode produced a different verdict"
    );
    if let Some(expected) = expected {
        assert!(!whole.1, "valid frames failed to decode");
        assert_eq!(whole.0, expected, "valid frames did not round-trip");
    }
}
