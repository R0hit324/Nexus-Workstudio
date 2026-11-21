use bytes::{Buf, BufMut, BytesMut};
use std::io;
use tokio_util::codec::{Decoder, Encoder};

use crate::message::Message;

/// Maximum accepted frame payload size (16 MiB). Guards against oversized
/// frames from malicious or buggy peers.
pub const MAX_FRAME_LENGTH: u32 = 16 * 1024 * 1024;

/// Encodes [`Message`] instances as length-prefixed, postcard-serialized
/// frames:
///
/// ```text
/// +--------+-------------+
/// |  len   |  payload    |
/// | u32 BE | postcard    |
/// +--------+-------------+
/// ```
///
/// `postcard` was chosen for its compactness and speed over the LAN protocol
/// (see `docs/decisions/ADR-003-serialization-format.md`).
#[derive(Debug, Clone, Copy)]
pub struct FrameCodec {
    max_frame_length: u32,
}

impl FrameCodec {
    #[must_use]
    pub const fn new(max_frame_length: u32) -> Self {
        Self { max_frame_length }
    }
}

impl Default for FrameCodec {
    fn default() -> Self {
        Self::new(MAX_FRAME_LENGTH)
    }
}

impl Encoder<Message> for FrameCodec {
    type Error = io::Error;

    fn encode(&mut self, item: Message, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let payload = postcard::to_stdvec(&item).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to serialize message: {e}"),
            )
        })?;

        let length = u32::try_from(payload.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "message too large"))?;

        if length > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("frame exceeds max length {length}"),
            ));
        }

        dst.reserve(4 + payload.len());
        dst.put_u32(length);
        dst.extend_from_slice(&payload);
        Ok(())
    }
}

impl Decoder for FrameCodec {
    type Item = Message;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < 4 {
            return Ok(None);
        }

        // Peek at the header without consuming it: a `Decoder` that returns
        // `Ok(None)` must not have advanced the buffer.
        let length = u32::from_be_bytes([src[0], src[1], src[2], src[3]]);
        if length > self.max_frame_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("frame exceeds max length {length}"),
            ));
        }

        let length = length as usize;
        if src.len() < 4 + length {
            src.reserve(4 + length - src.len());
            return Ok(None);
        }

        src.advance(4);
        let payload = src.split_to(length);
        let message = postcard::from_bytes::<Message>(&payload).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to deserialize message: {e}"),
            )
        })?;

        Ok(Some(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use futures_util::{SinkExt, StreamExt};
    use nexus_core::ids::RequestId;
    use tokio::io::duplex;
    use tokio_util::codec::Framed;

    #[tokio::test]
    async fn frame_roundtrips_over_duplex_stream() {
        let (a, b) = duplex(8192);
        let mut left = Framed::new(a, FrameCodec::default());
        let mut right = Framed::new(b, FrameCodec::default());

        let msg = Message::client_hello("test", vec![]);

        left.send(msg.clone()).await.unwrap();
        left.flush().await.unwrap();

        let received = right.next().await.unwrap().unwrap();
        assert_eq!(received, msg);
    }

    #[tokio::test]
    async fn multiple_frames_are_delivered_in_order() {
        let (a, b) = duplex(8192);
        let mut left = Framed::new(a, FrameCodec::default());
        let mut right = Framed::new(b, FrameCodec::default());

        for i in 0..5u32 {
            let msg = Message::Heartbeat {
                at: nexus_core::timestamp::Timestamp::from_millis(i as u64),
            };
            left.send(msg).await.unwrap();
        }
        left.flush().await.unwrap();

        for i in 0..5u32 {
            let received = right.next().await.unwrap().unwrap();
            match received {
                Message::Heartbeat { at } => {
                    assert_eq!(at, nexus_core::timestamp::Timestamp::from_millis(i as u64));
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn oversized_frames_are_rejected() {
        let (a, b) = duplex(1024 * 1024);
        let mut left = Framed::new(a, FrameCodec::new(64));
        let _right = Framed::new(b, FrameCodec::new(64));

        let big_text = "x".repeat(1024);
        let msg = Message::Authenticate {
            request_id: RequestId::new(),
            username: big_text,
        };

        // Encoding fails locally since the payload exceeds the limit.
        let encode_result = left.send(msg.clone()).await;
        assert!(encode_result.is_err());

        // Contract check: the payload really is larger than 64 bytes.
        let payload = postcard::to_stdvec(&msg).unwrap();
        assert!(payload.len() > 64);

        // And a raw oversized frame is rejected by the decoder.
        let mut raw = BytesMut::with_capacity(4 + payload.len());
        raw.put_u32(1024);
        raw.extend_from_slice(&payload);
        let mut codec = FrameCodec::new(64);
        let decoded = codec.decode(&mut raw);
        assert!(decoded.is_err());
    }

    #[test]
    fn truncated_frames_wait_for_more_data() {
        let mut codec = FrameCodec::default();
        let mut src = BytesMut::new();
        src.put_u32(100); // claims 100 bytes but sends none
        assert!(codec.decode(&mut src).unwrap().is_none());
    }

    #[test]
    fn decode_rejects_garbage_payload() {
        let mut codec = FrameCodec::default();
        let mut src = BytesMut::new();
        src.put_u32(4);
        src.extend_from_slice(&[0xFF, 0xFE, 0xFD, 0xFC]);
        assert!(codec.decode(&mut src).is_err());
    }
}
