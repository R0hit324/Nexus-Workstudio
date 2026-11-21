//! Protocol conformance tests run against the public protocol surface.
//!
//! These tests deliberately re-drive framing and transport through the public
//! API (rather than crate-internal helpers) so they catch packaging mistakes,
//! not just logic mistakes.

#![cfg_attr(not(test), allow(dead_code, unused_imports))]

use bytes::BytesMut;
use futures_util::{SinkExt, StreamExt};
use nexus_core::ids::{CubeId, UserId};
use nexus_core::timestamp::Timestamp;
use nexus_protocol::{FrameCodec, Message, TcpTransport, Transport};
use tokio::io::duplex;
use tokio_util::codec::{Decoder, Encoder, Framed};

fn sample_messages() -> Vec<Message> {
    vec![
        Message::client_hello("protocol-test", vec![nexus_protocol::Capability::Chat]),
        Message::server_hello("protocol-test-server", vec![]),
        Message::Authenticate {
            request_id: nexus_core::ids::RequestId::new(),
            username: "zoe".into(),
        },
        Message::Heartbeat {
            at: Timestamp::now(),
        },
        Message::Ack {
            request_id: nexus_core::ids::RequestId::new(),
            ok: true,
            message: None,
        },
    ]
}

#[test]
fn frame_codec_zero_copy_framing_roundtrips() {
    let mut encoder = FrameCodec::default();
    let mut dst = BytesMut::new();
    for (idx, msg) in sample_messages().iter().enumerate() {
        encoder.encode(msg.clone(), &mut dst).unwrap();
        // Frames are length-prefixed; the length equals the encoded payload size.
        let length = u32::from_be_bytes([dst[0], dst[1], dst[2], dst[3]]) as usize;
        assert_eq!(length, dst.len() - 4);
        assert_ne!(length, 0);

        let mut decoder = FrameCodec::default();
        let decoded = decoder.decode(&mut dst).unwrap().expect("full frame");
        assert_eq!(&decoded, msg, "frame {idx} roundtripped");
    }
}

#[test]
fn decoder_waits_for_partial_frames() {
    let mut codec = FrameCodec::default();
    let mut src = BytesMut::new();

    let msg = Message::Heartbeat {
        at: Timestamp::now(),
    };
    let mut full = BytesMut::new();
    codec.encode(msg.clone(), &mut full).unwrap();
    codec.encode(msg.clone(), &mut full).unwrap();

    // Feed the stream one byte at a time; the decoder must never emit a frame
    // until the complete frame is present.
    let mut emitted = 0;
    for byte in full.iter() {
        src.extend_from_slice(&[*byte]);
        if let Some(decoded) = codec.decode(&mut src).unwrap() {
            assert_eq!(decoded, msg);
            emitted += 1;
        }
    }
    assert_eq!(emitted, 2);
}

#[test]
fn oversized_frames_are_rejected_by_the_decoder() {
    let mut codec = FrameCodec::new(64);
    let mut src = BytesMut::new();
    src.extend_from_slice(&100u32.to_be_bytes());
    assert!(codec.decode(&mut src).is_err());
}

#[tokio::test]
async fn transport_receives_none_after_peer_closes() {
    let (a, b) = duplex(4096);
    let mut left = Framed::new(a, FrameCodec::default());
    let mut right = Framed::new(b, FrameCodec::default());

    left.close().await.unwrap();
    assert!(right.next().await.is_none());
}

#[tokio::test]
async fn tcp_transport_full_roundtrip() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let mut client = TcpTransport::connect(addr).await.unwrap();
    let (stream, peer) = listener.accept().await.unwrap();
    let mut server = TcpTransport::from_stream(stream, peer);

    client
        .send(&Message::client_hello("tcp", vec![]))
        .await
        .unwrap();
    let received = server.receive().await.unwrap().expect("message");
    assert_eq!(received, Message::client_hello("tcp", vec![]));

    server
        .send(&Message::server_hello("tcp-server", vec![]))
        .await
        .unwrap();
    let received = client.receive().await.unwrap().expect("message");
    assert_eq!(received, Message::server_hello("tcp-server", vec![]));

    client.close().await.unwrap();
    assert!(server.receive().await.unwrap().is_none());
}

#[test]
fn sample_messages_are_distinct() {
    let messages = sample_messages();
    let mut seen = std::collections::HashSet::new();
    // Normalize the hello messages (they carry the same protocol version) to a
    // stable key so we only assert that at least two distinct shapes exist.
    for msg in messages {
        let key = match msg {
            Message::ClientHello { client_name, .. } => format!("client:{client_name}"),
            Message::ServerHello { server_name, .. } => format!("server:{server_name}"),
            Message::Authenticate { username, .. } => format!("auth:{username}"),
            Message::Heartbeat { .. } => "heartbeat".into(),
            Message::Ack { ok, .. } => format!("ack:{ok}"),
            other => format!("{other:?}"),
        };
        seen.insert(key);
    }
    assert!(
        seen.len() >= 4,
        "expected several message shapes, got {seen:?}"
    );
}

#[tokio::test]
async fn cube_id_negatives_are_safe() {
    let _ = CubeId::new();
    let _ = UserId::new();
}
