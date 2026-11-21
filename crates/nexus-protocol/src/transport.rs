use std::net::SocketAddr;

use async_trait::async_trait;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio_util::codec::Framed;

use crate::frame::FrameCodec;
use crate::message::Message;

/// Errors produced by a [`Transport`].
#[derive(Debug, Error)]
pub enum TransportError {
    /// The remote peer closed the connection.
    #[error("connection closed")]
    ConnectionClosed,
    /// Underlying I/O failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// A protocol/framing error occurred while exchanging messages.
    #[error("protocol error: {0}")]
    Protocol(String),
}

impl From<TransportError> for nexus_core::Error {
    fn from(err: TransportError) -> Self {
        match err {
            TransportError::ConnectionClosed => {
                nexus_core::Error::network("connection closed").with_internal(format!("{err}"))
            }
            TransportError::Io(io) => {
                nexus_core::Error::network("network I/O failure").with_source(io)
            }
            TransportError::Protocol(msg) => {
                nexus_core::Error::protocol(msg).with_internal("transport protocol error")
            }
        }
    }
}

/// The abstraction all Nexus domain logic depends on for communication.
///
/// Domain code must never touch TCP/QUIC/WebSocket details directly; this
/// trait keeps the protocol transport-replaceable (see `ADR-002`).
#[async_trait]
pub trait Transport: Send {
    /// Sends one message to the peer.
    async fn send(&mut self, message: &Message) -> Result<(), TransportError>;

    /// Receives the next message.
    ///
    /// Returns `Ok(None)` when the connection is closed by the peer.
    async fn receive(&mut self) -> Result<Option<Message>, TransportError>;

    /// Closes the connection gracefully, flushing any pending messages.
    async fn close(&mut self) -> Result<(), TransportError>;
}

/// A [`Transport`] over TCP using length-prefixed postcard frames.
pub struct TcpTransport {
    framed: Framed<TcpStream, FrameCodec>,
    peer: SocketAddr,
}

impl TcpTransport {
    /// Connects to `peer` and returns a ready transport.
    pub async fn connect(peer: SocketAddr) -> Result<Self, TransportError> {
        let stream = TcpStream::connect(peer).await?;
        let framed = Framed::new(stream, FrameCodec::default());
        Ok(Self { framed, peer })
    }

    /// Wraps an already-accepted TCP stream (used by the server).
    pub fn from_stream(stream: TcpStream, peer: SocketAddr) -> Self {
        let framed = Framed::new(stream, FrameCodec::default());
        Self { framed, peer }
    }

    /// The remote address this transport is connected to.
    #[must_use]
    pub const fn peer(&self) -> SocketAddr {
        self.peer
    }
}

#[async_trait]
impl Transport for TcpTransport {
    async fn send(&mut self, message: &Message) -> Result<(), TransportError> {
        use futures_util::SinkExt;
        self.framed.send(message.clone()).await?;
        self.framed.flush().await?;
        Ok(())
    }

    async fn receive(&mut self) -> Result<Option<Message>, TransportError> {
        use futures_util::StreamExt;
        match self.framed.next().await {
            Some(Ok(message)) => Ok(Some(message)),
            Some(Err(err)) => Err(TransportError::Protocol(err.to_string())),
            None => Ok(None),
        }
    }

    async fn close(&mut self) -> Result<(), TransportError> {
        use futures_util::SinkExt;
        self.framed.close().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn tcp_transport_roundtrips_messages() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let mut client = TcpTransport::connect(addr).await.unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let peer = stream.peer_addr().unwrap();
        let mut server = TcpTransport::from_stream(stream, peer);

        client
            .send(&Message::client_hello("transport-test", vec![]))
            .await
            .unwrap();

        let received = server.receive().await.unwrap().expect("message");
        match received {
            Message::ClientHello { client_name, .. } => assert_eq!(client_name, "transport-test"),
            other => panic!("unexpected {other:?}"),
        }

        server
            .send(&Message::server_hello("transport-test-server", vec![]))
            .await
            .unwrap();
        let received = client.receive().await.unwrap().expect("message");
        match received {
            Message::ServerHello { server_name, .. } => {
                assert_eq!(server_name, "transport-test-server");
            }
            other => panic!("unexpected {other:?}"),
        }

        client.close().await.unwrap();
        assert!(server.receive().await.unwrap().is_none());
    }
}
