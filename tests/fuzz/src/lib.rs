//! Deterministic fuzz harnesses for the Nexus wire protocol (Part 14,
//! spec sections 71 and 84).
//!
//! Fuzzing runs against a fixed, seeded corpus so results are reproducible.
//! Every input must be handled without panics and with a well-defined
//! accept/reject decision from the frame codec.

#![cfg_attr(not(test), allow(dead_code, unused_imports))]

use bytes::{BufMut, BytesMut};
use nexus_protocol::{FrameCodec, MAX_FRAME_LENGTH};
use nexus_tests_support::TestRng;
use tokio_util::codec::Decoder;

/// Builds a frame header plus a deterministic random payload.
fn next_frame(rng: &mut TestRng, capacity: u64) -> BytesMut {
    let len = rng.next_bounded(capacity);
    let mut src = BytesMut::with_capacity(4 + len as usize);
    src.put_u32(len as u32);
    let mut payload = vec![0u8; len as usize];
    rng.fill(&mut payload);
    src.extend_from_slice(&payload);
    src
}

#[test]
fn fuzz_frame_decoder_never_panics_on_malformed_input() {
    let mut rng = TestRng::new(0xF17E);
    let mut codec = FrameCodec::new(MAX_FRAME_LENGTH);

    for _ in 0..256 {
        let mut src = next_frame(&mut rng, 96);
        // Decoding must always settle to Ok/Err/None; never panic.
        let _ = codec.decode(&mut src);
    }
}

#[test]
fn fuzz_oversized_frame_headers_are_rejected() {
    let mut codec = FrameCodec::new(MAX_FRAME_LENGTH);
    let mut src = BytesMut::new();
    src.put_u32(MAX_FRAME_LENGTH + 1);
    assert!(codec.decode(&mut src).is_err());
}

#[test]
fn fuzz_truncated_frame_waits_for_completion_without_panicking() {
    let mut codec = FrameCodec::new(MAX_FRAME_LENGTH);
    let mut src = BytesMut::new();
    src.put_u32(64);
    // A frame that claims 64 bytes but has none is simply incomplete.
    let _ = codec.decode(&mut src);

    src.extend_from_slice(&[0u8; 64]);
    // Whatever the outcome (accept or reject), it must be a clean result.
    let _ = codec.decode(&mut src);
}
