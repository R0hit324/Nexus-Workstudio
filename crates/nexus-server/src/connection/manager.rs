use nexus_core::ids::{ConnectionId, CubeId, SessionId};
use nexus_protocol::message::Message;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::mpsc;

use crate::metrics::Metrics;

/// Delivery guarantee for an outbound message.
///
/// - [`Delivery::Await`] blocks until a queue slot is free (or the connection
///   closes). Used for request/response traffic where loss is not acceptable.
/// - [`Delivery::Try`] drops the message when the outbound queue is full and
///   counts the drop. Used for event fan-out so one slow consumer cannot
///   stall the producer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Await,
    Try,
}

/// Outcome of a non-blocking (Try) push.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendOutcome {
    Sent,
    QueueFull,
    ConnectionClosed,
}

#[derive(Debug)]
struct ConnectionHandle {
    outbound: mpsc::Sender<Message>,
    peer: SocketAddr,
    session_id: Option<SessionId>,
    cube_id: Option<CubeId>,
}

/// Routes messages to live connections.
///
/// Each connection owns a **bounded** outbound queue. The connection task is
/// the sole consumer; every other module injects via this manager. Sends are
/// performed *outside* the manager's lock (a clone of the channel sender is
/// used), so slow consumers never hold up the routing path or reader tasks.
pub struct ConnectionManager {
    connections: Mutex<HashMap<ConnectionId, ConnectionHandle>>,
    by_session: Mutex<HashMap<SessionId, ConnectionId>>,
    metrics: Option<Arc<Metrics>>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            by_session: Mutex::new(HashMap::new()),
            metrics: None,
        }
    }

    /// Attaches observability counters.
    pub fn with_metrics(mut self, metrics: Arc<Metrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Registers a connection and returns the outbound receiver the
    /// connection task drains. The queue capacity is bounded by
    /// `max_outbound_queue` (config).
    #[must_use]
    pub fn register(
        &self,
        connection_id: ConnectionId,
        peer: SocketAddr,
        max_outbound_queue: usize,
    ) -> mpsc::Receiver<Message> {
        let (outbound, receiver) = mpsc::channel(max_outbound_queue.max(1));
        self.connections
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                connection_id,
                ConnectionHandle {
                    outbound,
                    peer,
                    session_id: None,
                    cube_id: None,
                },
            );
        receiver
    }

    pub fn bind_session(&self, connection_id: ConnectionId, session_id: SessionId) {
        {
            let mut connections = self.connections.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(handle) = connections.get_mut(&connection_id) {
                handle.session_id = Some(session_id);
            }
        }
        self.by_session
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(session_id, connection_id);
    }

    pub fn bind_cube(&self, connection_id: ConnectionId, cube_id: Option<CubeId>) {
        self.connections
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(&connection_id)
            .map(|handle| handle.cube_id = cube_id);
    }

    /// Removes the connection and all indexes referencing it.
    pub fn remove(&self, connection_id: ConnectionId) {
        let mut connections = self.connections.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(handle) = connections.remove(&connection_id) {
            drop(handle); // closes the outbound channel; writer task sees None.
        }
        let mut by_session = self.by_session.lock().unwrap_or_else(|p| p.into_inner());
        by_session.retain(|_, conn| *conn != connection_id);
    }

    /// Sends `message` to a specific connection honoring `delivery`.
    pub async fn send_to_connection(
        &self,
        connection_id: ConnectionId,
        message: Message,
        delivery: Delivery,
    ) -> Result<SendOutcome, SendOutcome> {
        let sender = {
            self.connections
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&connection_id)
                .map(|h| h.outbound.clone())
        };
        let Some(sender) = sender else {
            return Err(SendOutcome::ConnectionClosed);
        };
        let sent = match delivery {
            Delivery::Await => sender.send(message).await.is_ok(),
            Delivery::Try => sender.try_send(message).is_ok(),
        };
        if !sent {
            if delivery == Delivery::Try {
                if let Some(metrics) = &self.metrics {
                    metrics.events_dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            return Err(SendOutcome::QueueFull);
        }
        Ok(SendOutcome::Sent)
    }

    /// Sends to whichever connection currently owns `session_id`.
    pub async fn send_to_session(
        &self,
        session_id: SessionId,
        message: Message,
        delivery: Delivery,
    ) -> Result<SendOutcome, SendOutcome> {
        let connection_id = {
            self.by_session
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&session_id)
                .copied()
        };
        let Some(connection_id) = connection_id else {
            return Err(SendOutcome::ConnectionClosed);
        };
        self.send_to_connection(connection_id, message, delivery)
            .await
    }

    /// Fans `message` out to every connection currently inside `cube_id`.
    /// Uses the `Try` policy: slow or unresponsive members never block the
    /// producer, and drops are counted.
    pub fn broadcast_to_cube(&self, cube_id: CubeId, message: Message) -> usize {
        self.broadcast_to_cube_except(cube_id, message, None)
    }

    /// Like [`Self::broadcast_to_cube`] but skips `exclude` (used to avoid
    /// echoing an action back to the connection that caused it).
    pub fn broadcast_to_cube_except(
        &self,
        cube_id: CubeId,
        message: Message,
        exclude: Option<ConnectionId>,
    ) -> usize {
        let senders: Vec<(ConnectionId, mpsc::Sender<Message>)> = {
            let connections = self.connections.lock().unwrap_or_else(|p| p.into_inner());
            connections
                .iter()
                .filter(|(id, handle)| handle.cube_id == Some(cube_id) && Some(**id) != exclude)
                .map(|(id, handle)| (*id, handle.outbound.clone()))
                .collect()
        };
        let mut sent = 0;
        for (_id, sender) in senders {
            if sender.try_send(message.clone()).is_ok() {
                sent += 1;
            } else if let Some(metrics) = &self.metrics {
                metrics.events_dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        sent
    }

    /// Number of live (registered) connections.
    pub fn active_connections(&self) -> usize {
        self.connections
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }

    /// Number of connections currently inside a cube.
    pub fn connections_in_cube(&self, cube_id: CubeId) -> usize {
        self.connections
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter(|h| h.cube_id == Some(cube_id))
            .count()
    }

    /// Metadata for diagnostics (peer and connection uptime).
    pub fn peer_of(&self, connection_id: ConnectionId) -> Option<SocketAddr> {
        self.connections
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&connection_id)
            .map(|h| h.peer)
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_protocol::message::Message;

    #[tokio::test]
    async fn register_and_drain() {
        let manager = ConnectionManager::new();
        let connection = ConnectionId::new();
        let mut rx = manager.register(connection, "127.0.0.1:1234".parse().unwrap(), 4);
        assert_eq!(manager.active_connections(), 1);

        let outcome = manager
            .send_to_connection(
                connection,
                Message::server_hello("nexus-server", vec![]),
                Delivery::Await,
            )
            .await;
        assert_eq!(outcome, Ok(SendOutcome::Sent));

        let received = rx.recv().await.expect("queued message");
        assert!(matches!(received, Message::ServerHello { .. }));

        manager.remove(connection);
        assert_eq!(manager.active_connections(), 0);
    }

    #[tokio::test]
    async fn missing_connection_is_closed() {
        let manager = ConnectionManager::new();
        let outcome = manager
            .send_to_connection(
                ConnectionId::new(),
                Message::server_hello("nexus-server", vec![]),
                Delivery::Await,
            )
            .await;
        assert_eq!(outcome, Err(SendOutcome::ConnectionClosed));
    }

    #[tokio::test]
    async fn event_drop_when_queue_full() {
        let metrics = Arc::new(Metrics::new());
        let manager = ConnectionManager::new().with_metrics(metrics.clone());
        let connection = ConnectionId::new();
        let _rx = manager.register(connection, "127.0.0.1:1234".parse().unwrap(), 1);
        // Fill the single slot.
        manager
            .send_to_connection(
                connection,
                Message::server_hello("nexus-server", vec![]),
                Delivery::Try,
            )
            .await
            .unwrap();
        let outcome = manager
            .send_to_connection(
                connection,
                Message::server_hello("nexus-server", vec![]),
                Delivery::Try,
            )
            .await;
        assert_eq!(outcome, Err(SendOutcome::QueueFull));
        assert_eq!(metrics.events_dropped.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn send_to_session_bridged() {
        let manager = ConnectionManager::new();
        let connection = ConnectionId::new();
        let mut rx = manager.register(connection, "127.0.0.1:1234".parse().unwrap(), 4);
        let session = SessionId::new();
        manager.bind_session(connection, session);

        let outcome = manager
            .send_to_session(
                session,
                Message::server_hello("nexus-server", vec![]),
                Delivery::Await,
            )
            .await;
        assert_eq!(outcome, Ok(SendOutcome::Sent));
        assert!(rx.recv().await.is_some());
    }
}
