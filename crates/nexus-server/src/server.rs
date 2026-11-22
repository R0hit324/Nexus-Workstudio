use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nexus_core::config::ServerConfig;
use nexus_core::cube::CubeManager;
use nexus_core::ids::{ConnectionId, SessionId, UserId};
use nexus_core::session::SessionDescriptor;
use nexus_core::timestamp::Timestamp;
use nexus_core::PerformanceMetrics;
use nexus_protocol::{ErrorCode as ProtoError, Message, TcpTransport, Transport};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::auth::{AuthenticationRequest, Authenticator, TrustAuthenticator};
use crate::connection::ConnectionManager;
use crate::events::{EventBus, EventRouter};
use crate::lifecycle::{LifecycleHandle, ServerLifecycle};
use crate::manager::InMemoryCubeManager;
use crate::metrics::Metrics;
use crate::permissions::PermissionManager;
use crate::plugins::PluginHost;
use crate::protocol::{to_protocol_error, DispatchContext, Dispatcher};
use crate::session::SessionManager;
use crate::shutdown::Shutdown;
use crate::storage::{InMemoryStorage, Storage};
use crate::users::UserRegistry;
use crate::workspace::WorkspaceManager;
use crate::SERVER_NAME;

/// Shared, observable server runtime state.
#[derive(Debug, Default)]
pub struct ServerState {
    /// Number of concurrently accepted connections.
    pub active_connections: AtomicUsize,
    /// Number of authenticated sessions.
    pub active_sessions: AtomicUsize,
}

/// The Nexus server. Holds the authoritative domain services.
pub struct Server {
    config: ServerConfig,
    cube_manager: Arc<dyn CubeManager>,
    storage: Arc<dyn Storage>,
    event_log_dir: Option<std::path::PathBuf>,
}

/// A bound, running server handle.
pub struct RunningServer {
    pub local_addr: SocketAddr,
    task: Option<JoinHandle<()>>,
    state: Arc<ServerState>,
    shutdown: Arc<Shutdown>,
    plugins: Arc<PluginHost>,
}

impl RunningServer {
    /// Waits for the accept loop to finish (normally only on shutdown).
    pub async fn wait(&mut self) {
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }

    /// The shared runtime state (counters, etc.).
    #[must_use]
    pub fn state(&self) -> &Arc<ServerState> {
        &self.state
    }

    /// The server's plugin host (commands, registry, metrics).
    #[must_use]
    pub fn plugins(&self) -> &Arc<PluginHost> {
        &self.plugins
    }

    /// Asks the server to shut down gracefully: listeners stop accepting and
    /// connection tasks are signalled to flush and close.
    pub fn request_shutdown(&self) {
        self.shutdown.trigger();
    }
}

impl Server {
    /// Creates a server with default in-memory Cube management and storage.
    #[must_use]
    pub fn new(config: ServerConfig) -> Self {
        let cube_manager = Arc::new(InMemoryCubeManager::new(config.max_cubes));
        Self::with_manager(config, cube_manager)
    }

    /// Creates a server with a custom Cube manager.
    #[must_use]
    pub fn with_manager(config: ServerConfig, cube_manager: Arc<dyn CubeManager>) -> Self {
        Self {
            config,
            cube_manager,
            storage: Arc::new(InMemoryStorage::new()),
            event_log_dir: None,
        }
    }

    /// Creates a server that persists workspace data under the given storage
    /// (e.g. one rooted at `config.data_dir` for production deployments).
    /// Uses the default in-memory Cube manager.
    #[must_use]
    pub fn with_storage(config: ServerConfig, storage: Arc<dyn Storage>) -> Self {
        Self {
            cube_manager: Arc::new(InMemoryCubeManager::new(config.max_cubes)),
            config,
            storage,
            event_log_dir: None,
        }
    }

    /// Routes the Part 4 event log to `dir` (append-only, checksummed
    /// segments). Without this the engine runs over an in-memory store.
    #[must_use]
    pub fn with_event_log(mut self, dir: impl AsRef<std::path::Path>) -> Self {
        self.event_log_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    fn build_event_engine(&self) -> anyhow::Result<Arc<nexus_events::bus::EventBus>> {
        use nexus_events::metrics::EventMetrics;
        use nexus_events::store::{EventStore, MemoryEventStore, SegmentedEventStore};

        let store: Arc<dyn EventStore> = match &self.event_log_dir {
            Some(dir) => {
                std::fs::create_dir_all(dir)?;
                Arc::new(SegmentedEventStore::open(dir)?)
            }
            None => Arc::new(MemoryEventStore::new()),
        };
        Ok(Arc::new(nexus_events::bus::EventBus::new(
            store,
            None,
            Arc::new(EventMetrics::new()),
        )))
    }

    /// Binds the listener and spawns the accept loop.
    ///
    /// # Errors
    /// Returns an error if the listener cannot bind.
    pub async fn start(self) -> anyhow::Result<RunningServer> {
        let lifecycle = LifecycleHandle::new_booting();
        if !lifecycle.transition_to(ServerLifecycle::Initializing) {
            return Err(anyhow::anyhow!("server lifecycle failed to start"));
        }

        let state = Arc::new(ServerState::default());
        let shutdown = Arc::new(Shutdown::new());
        let metrics = Arc::new(Metrics::new());

        // ---- Domain services -------------------------------------------------
        let session_manager = Arc::new(SessionManager::new());
        let connection_manager = Arc::new(ConnectionManager::new().with_metrics(metrics.clone()));
        let users = Arc::new(UserRegistry::new());
        let authenticator: Arc<dyn Authenticator> = Arc::new(TrustAuthenticator::new(users));
        let workspace_manager = Arc::new(WorkspaceManager::new(self.storage.clone()));
        let permission_manager = Arc::new(PermissionManager::new());
        let event_bus = Arc::new(EventBus::new());
        let event_engine = self.build_event_engine()?;
        let event_router = Arc::new(EventRouter::with_engine(
            event_bus,
            event_engine,
            connection_manager.clone(),
            session_manager.clone(),
        ));
        let dispatcher = Dispatcher::new(
            self.cube_manager.clone(),
            session_manager.clone(),
            connection_manager.clone(),
            permission_manager.clone(),
            workspace_manager.clone(),
            event_router.clone(),
            metrics.clone(),
        );

        // ---- Plugin host ---------------------------------------------------------
        // Built-ins are trusted; external manifests come from data/plugins.
        let plugin_host = Arc::new(PluginHost::new(
            vec![std::path::PathBuf::from(&self.config.data_dir).join("plugins")],
            None,
        ));
        if let Err(failure) = plugin_host.start().await {
            // Plugin boot problems must never prevent the server from serving.
            warn!(error = %failure.message, "plugin_host_boot_failed");
        }
        info!(summary = %plugin_host.summary(), "plugins_loaded");
        let dispatcher = Arc::new(dispatcher.with_plugins(Arc::clone(&plugin_host)));
        let perf = Arc::new(PerformanceMetrics::new());

        let inner = Arc::new(ServerInner {
            config: self.config,
            state: state.clone(),
            shutdown: shutdown.clone(),
            lifecycle,
            session_manager,
            connection_manager,
            dispatcher,
            authenticator,
            metrics,
            perf,
            plugins: Arc::clone(&plugin_host),
        });

        // ---- Bind and serve --------------------------------------------------
        let listener = TcpListener::bind((inner.config.bind.as_str(), inner.config.port)).await?;
        let local_addr = listener.local_addr()?;
        if !inner.lifecycle.transition_to(ServerLifecycle::Listening) {
            return Err(anyhow::anyhow!("server lifecycle stalled before listening"));
        }
        info!(%local_addr, "server_listening");

        let accept = inner.clone();
        let task = tokio::spawn(accept.accept_loop(listener));
        if !inner.lifecycle.transition_to(ServerLifecycle::Running) {
            return Err(anyhow::anyhow!("server lifecycle stalled entering running"));
        }
        Ok(RunningServer {
            local_addr,
            task: Some(task),
            state,
            shutdown,
            plugins: plugin_host,
        })
    }
}

/// Services shared by every connection task.
struct ServerInner {
    config: ServerConfig,
    state: Arc<ServerState>,
    shutdown: Arc<Shutdown>,
    lifecycle: LifecycleHandle,
    session_manager: Arc<SessionManager>,
    connection_manager: Arc<ConnectionManager>,
    dispatcher: Arc<Dispatcher>,
    authenticator: Arc<dyn Authenticator>,
    metrics: Arc<Metrics>,
    perf: Arc<PerformanceMetrics>,
    plugins: Arc<PluginHost>,
}

impl ServerInner {
    async fn accept_loop(self: Arc<Self>, listener: TcpListener) {
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, peer)) => {
                            let active = self
                                .state
                                .active_connections
                                .fetch_add(1, Ordering::Relaxed);
                            if active >= self.config.max_clients as usize {
                                warn!(%peer, "connection_limit_reached");
                                self.state.active_connections.fetch_sub(1, Ordering::Relaxed);
                                drop(stream);
                            } else {
                                let inner = self.clone();
                                tokio::spawn(inner.handle_connection(stream, peer));
                            }
                        }
                        Err(err) => {
                            warn!(error = %err, "accept_failed");
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
                _ = self.shutdown.delay() => {
                    info!("accept_loop_stopping");
                    break;
                }
            }
        }
        let _ = self.plugins.shutdown().await;
        let _ = self.lifecycle.transition_to(ServerLifecycle::Stopping);
        let _ = self.lifecycle.transition_to(ServerLifecycle::Stopped);
    }

    async fn handle_connection(self: Arc<Self>, stream: TcpStream, peer: SocketAddr) {
        let connection_id = ConnectionId::new();
        let outbound =
            self.connection_manager
                .register(connection_id, peer, self.config.max_outbound_queue);
        if let Err(err) = self
            .run_connection(connection_id, stream, peer, outbound)
            .await
        {
            debug!(%peer, error = %err, "connection_closed");
        }
        self.connection_manager.remove(connection_id);
        self.state
            .active_connections
            .fetch_sub(1, Ordering::Relaxed);
    }

    async fn run_connection(
        self: &Arc<Self>,
        connection_id: ConnectionId,
        stream: TcpStream,
        peer: SocketAddr,
        mut outbound: mpsc::Receiver<Message>,
    ) -> anyhow::Result<()> {
        let mut transport = TcpTransport::from_stream(stream, peer);

        // ---- Handshake -------------------------------------------------------
        let handshake = crate::protocol::perform_handshake(
            &mut transport,
            Duration::from_secs(self.config.handshake_timeout_secs),
            SERVER_NAME,
        )
        .await?;
        info!(
            client = %handshake.client_name,
            capabilities = ?handshake.capabilities,
            %peer,
            "client_connected"
        );

        // ---- Authentication ---------------------------------------------------
        let auth = tokio::time::timeout(
            Duration::from_secs(self.config.authentication_timeout_secs),
            transport.receive(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("authentication timeout"))?
        .map_err(|err| anyhow::anyhow!("{err}"))?
        .ok_or_else(|| anyhow::anyhow!("peer closed during authentication"))?;

        let Some((session_id, user_id)) = self
            .authenticate(auth, connection_id, peer, &mut transport)
            .await?
        else {
            // An explicit error reply was already sent; nothing else to do.
            return Ok(());
        };
        self.state.active_sessions.fetch_add(1, Ordering::Relaxed);

        // ---- Dispatch loop ----------------------------------------------------
        let outcome = self
            .dispatch_loop(
                connection_id,
                session_id,
                user_id,
                &mut transport,
                &mut outbound,
            )
            .await;

        // ---- Cleanup -----------------------------------------------------------
        if let Some(cube_id) = self
            .session_manager
            .get(session_id)
            .and_then(|s| s.cube_id())
        {
            self.connection_manager.bind_cube(connection_id, None);
            self.dispatcher.presence(cube_id, user_id, true, None);
        }
        self.session_manager.begin_disconnect(session_id);
        self.session_manager.destroy(session_id);
        self.state.active_sessions.fetch_sub(1, Ordering::Relaxed);
        info!(%session_id, %peer, "session_closed");

        outcome
    }

    /// Verifies the client identity and opens a session. Returns `None` when
    /// authentication failed but an error reply was already sent.
    async fn authenticate(
        &self,
        message: Message,
        connection_id: ConnectionId,
        peer: SocketAddr,
        transport: &mut dyn Transport,
    ) -> anyhow::Result<Option<(SessionId, UserId)>> {
        let Message::Authenticate {
            request_id,
            username,
        } = message
        else {
            transport
                .send(&Message::error(
                    None,
                    ProtoError::ProtocolViolation,
                    "expected Authenticate",
                ))
                .await?;
            return Ok(None);
        };

        let request = AuthenticationRequest {
            username: username.clone(),
            peer,
        };
        self.metrics.authentication_attempt();
        match self.authenticator.authenticate(request).await {
            Ok(identity) => {
                let session = self.session_manager.create(
                    connection_id,
                    identity.user_id,
                    &identity.username,
                );
                let session_id = session.session_id();
                self.session_manager.mark_authenticated(session_id);
                self.connection_manager
                    .bind_session(connection_id, session_id);

                let now = Timestamp::now();
                let descriptor = SessionDescriptor {
                    id: session_id,
                    user_id: identity.user_id,
                    username: identity.username.clone(),
                    cube_id: None,
                    connected_at: now,
                    last_activity: now,
                };
                transport
                    .send(&Message::AuthenticationResult {
                        request_id,
                        session: descriptor,
                    })
                    .await?;
                info!(%session_id, username = %identity.username, %peer, "session_opened");
                Ok(Some((session_id, identity.user_id)))
            }
            Err(err) => {
                let code = to_protocol_error(err.code());
                transport
                    .send(&Message::error(Some(request_id), code, err.user_message()))
                    .await?;
                Ok(None)
            }
        }
    }

    /// The heart of a connection: select inbound messages, queued outbound
    /// broadcast events, the idle deadline, and the shutdown signal.
    async fn dispatch_loop(
        &self,
        connection_id: ConnectionId,
        session_id: SessionId,
        user_id: UserId,
        transport: &mut dyn Transport,
        outbound: &mut mpsc::Receiver<Message>,
    ) -> anyhow::Result<()> {
        let ctx = DispatchContext {
            session_id,
            user_id,
            connection_id,
        };
        let idle = Duration::from_secs(self.config.heartbeat_timeout_secs);
        let write_timeout = Duration::from_secs(self.config.write_timeout_secs);

        loop {
            enum Outcome {
                Continue,
                Close,
            }
            let outcome = tokio::select! {
                received = transport.receive() => match received {
                    Ok(Some(message)) => {
                        self.metrics.message_in();
                        self.session_manager.note_received(session_id);
                        let perf_started = Instant::now();
                        let action = self.dispatcher.handle(message, ctx).await;
                        self.perf
                            .record_latency_ms(perf_started.elapsed().as_millis() as u64);
                        self.perf.record_operation();
                        if let Some(reply) = action.reply() {
                            if let Err(err) = timeout(
                                write_timeout,
                                transport.send(reply),
                            )
                            .await
                            .map_err(|_| anyhow::anyhow!("write timeout"))
                            .and_then(|res| res.map_err(|err| anyhow::anyhow!("send failed: {err}")))
                            {
                                debug!(%connection_id, error = %err, "reply_failed");
                                return Err(err);
                            }
                            self.metrics.message_out();
                        }
                        if action.is_close() {
                            Outcome::Close
                        } else {
                            Outcome::Continue
                        }
                    }
                    Ok(None) => {
                        debug!(%connection_id, "peer_closed");
                        Outcome::Close
                    }
                    Err(err) => return Err(anyhow::anyhow!("receive failed: {err}")),
                },
                queued = outbound.recv() => {
                    match queued {
                        Some(message) => {
                            timeout(
                                write_timeout,
                                transport.send(&message),
                            )
                            .await
                            .map_err(|_| anyhow::anyhow!("write timeout"))
                            .and_then(|res| res.map_err(|err| anyhow::anyhow!("send failed: {err}")))?;
                            self.metrics.message_out();
                            Outcome::Continue
                        }
                        None => {
                            debug!(%connection_id, "outbound_channel_closed");
                            Outcome::Close
                        }
                    }
                }
                _ = tokio::time::sleep(idle) => {
                    warn!(%connection_id, session_id = %session_id, "idle_timeout");
                    Outcome::Close
                }
                _ = self.shutdown.delay() => {
                    info!(%session_id, "shutdown_signalled");
                    let _ = timeout(
                        write_timeout,
                        transport.send(&Message::Disconnect {
                            reason: "server shutting down".into(),
                        }),
                    )
                    .await;
                    Outcome::Close
                }
            };

            if matches!(outcome, Outcome::Close) {
                break;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::config::ServerConfig;
    use nexus_core::ids::RequestId;

    async fn spawn_server() -> SocketAddr {
        let config = ServerConfig {
            bind: "127.0.0.1".into(),
            port: 0,
            ..ServerConfig::default()
        };
        let server = Server::new(config);
        let running = server.start().await.unwrap();
        running.local_addr
    }

    async fn connect_and_auth(username: &str) -> TcpTransport {
        let addr = spawn_server().await;
        let mut transport = TcpTransport::connect(addr).await.unwrap();
        transport
            .send(&Message::client_hello("test", vec![]))
            .await
            .unwrap();
        transport.receive().await.unwrap().unwrap();
        transport
            .send(&Message::Authenticate {
                request_id: RequestId::new(),
                username: username.into(),
            })
            .await
            .unwrap();
        let auth = transport.receive().await.unwrap().unwrap();
        match auth {
            Message::AuthenticationResult { session, .. } => {
                assert_eq!(session.username, username);
            }
            other => panic!("unexpected {other:?}"),
        }
        transport
    }

    #[tokio::test]
    async fn handshake_and_authentication() {
        let mut transport = connect_and_auth("alice").await;
        transport.close().await.unwrap();
    }

    #[tokio::test]
    async fn create_join_list_lifecycle_over_the_wire() {
        let mut transport = connect_and_auth("bob").await;

        transport
            .send(&Message::CreateCube {
                request_id: RequestId::new(),
                name: "demo".into(),
            })
            .await
            .unwrap();
        let cube = match transport.receive().await.unwrap().unwrap() {
            Message::CubeCreated { cube, .. } => cube,
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(cube.name, "demo");

        transport
            .send(&Message::ListCubes {
                request_id: RequestId::new(),
            })
            .await
            .unwrap();
        match transport.receive().await.unwrap().unwrap() {
            Message::CubeList { cubes, .. } => assert_eq!(cubes.len(), 1),
            other => panic!("unexpected {other:?}"),
        }

        transport
            .send(&Message::Heartbeat {
                at: nexus_core::timestamp::Timestamp::now(),
            })
            .await
            .unwrap();
        match transport.receive().await.unwrap().unwrap() {
            Message::HeartbeatAck { .. } => {}
            other => panic!("unexpected {other:?}"),
        }

        transport.close().await.unwrap();
    }

    #[tokio::test]
    async fn wrong_first_message_is_rejected() {
        let addr = spawn_server().await;
        let mut transport = TcpTransport::connect(addr).await.unwrap();
        // A client must open with ClientHello; anything else is answered with
        // a ProtocolViolation error and the connection is closed.
        transport
            .send(&Message::server_hello("not-a-client", vec![]))
            .await
            .unwrap();
        match transport.receive().await.unwrap().unwrap() {
            Message::Error { code, .. } => assert_eq!(code, ProtoError::ProtocolViolation),
            other => panic!("unexpected {other:?}"),
        }
        let eof = transport.receive().await.unwrap();
        assert!(eof.is_none());
        transport.close().await.unwrap();
    }

    #[tokio::test]
    async fn wrong_second_message_is_rejected_with_error() {
        let addr = spawn_server().await;
        let mut transport = TcpTransport::connect(addr).await.unwrap();
        transport
            .send(&Message::client_hello("test", vec![]))
            .await
            .unwrap();
        transport.receive().await.unwrap().unwrap();
        // A client must authenticate; sending a workspace request is rejected.
        transport
            .send(&Message::ListCubes {
                request_id: RequestId::new(),
            })
            .await
            .unwrap();
        match transport.receive().await.unwrap().unwrap() {
            Message::Error { code, .. } => assert_eq!(code, ProtoError::ProtocolViolation),
            other => panic!("unexpected {other:?}"),
        }
        transport.close().await.unwrap();
    }
}
