use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use nexus_core::config::ClientConfig;
use nexus_core::cube::CubeDescriptor;
use nexus_core::error::{Error, Result};
use nexus_core::ids::{CubeId, RequestId};
use nexus_core::session::SessionDescriptor;
use nexus_core::timestamp::Timestamp;
use nexus_core::version::Version;
use nexus_core::workspace::{FileEntry, WorkspaceMetadata};
use nexus_protocol::{Capability, Message, Transport, TransportError};
use tokio::sync::Mutex;
use tracing::{debug, warn};

use crate::{client_capabilities, CLIENT_NAME};

/// Timeout for a single request/response round trip.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Result of the initial connection handshake.
#[derive(Debug)]
pub struct Handshake {
    pub server_name: String,
    pub server_version: nexus_core::version::Version,
    pub server_capabilities: Vec<Capability>,
}

/// A connection to a Nexus server.
///
/// Part 1 operates strictly sequentially per connection: each call performs a
/// request/response exchange while holding the transport lock. Concurrent
/// event push (consumed independently of requests) arrives in Part 4, when the
/// protocol gains a proper split transport.
pub struct Client {
    transport: Mutex<Box<dyn Transport>>,
    pending_seq: AtomicU64,
    closed: AtomicBool,
    session: Mutex<Option<SessionDescriptor>>,
    handshake: Handshake,
}

impl Client {
    /// Establishes a TCP connection and performs the version handshake.
    ///
    /// # Errors
    /// Connection, version, or handshake failures.
    pub async fn connect(config: &ClientConfig) -> Result<Self> {
        let addr: SocketAddr = format!("{}:{}", config.server_address, config.server_port)
            .parse()
            .map_err(|e| Error::network("invalid server address").with_source(e))?;

        let transport: Box<dyn Transport> = Box::new(
            nexus_protocol::TcpTransport::connect(addr)
                .await
                .map_err(to_core_error)?,
        );
        let mut transport = transport;

        transport
            .send(&Message::client_hello(CLIENT_NAME, client_capabilities()))
            .await
            .map_err(to_core_error)?;

        let handshake = match transport.receive().await.map_err(to_core_error)? {
            Some(Message::ServerHello {
                server_name,
                protocol_version,
                capabilities,
            }) => {
                if protocol_version != nexus_protocol::PROTOCOL_VERSION {
                    return Err(Error::protocol(format!(
                        "server speaks protocol {protocol_version}, client understands {}",
                        nexus_protocol::PROTOCOL_VERSION
                    )));
                }
                debug!(%server_name, %protocol_version, ?capabilities, "handshake complete");
                Handshake {
                    server_name,
                    server_version: protocol_version,
                    server_capabilities: capabilities,
                }
            }
            Some(Message::Error { code, message, .. }) => {
                return Err(Error::protocol(format!(
                    "server rejected handshake ({code:?}): {message}"
                )));
            }
            other => {
                return Err(Error::protocol(format!(
                    "unexpected handshake reply: {other:?}"
                )));
            }
        };

        Ok(Self {
            transport: Mutex::new(transport),
            pending_seq: AtomicU64::new(1),
            closed: AtomicBool::new(false),
            session: Mutex::new(None),
            handshake,
        })
    }

    /// The negotiated handshake parameters.
    #[must_use]
    pub const fn handshake(&self) -> &Handshake {
        &self.handshake
    }

    /// Authenticates as `username` and caches the resulting session.
    ///
    /// # Errors
    /// Authentication failures and transport errors.
    pub async fn authenticate(&self, username: &str) -> Result<SessionDescriptor> {
        let request_id = self.next_request_id();
        let reply = self
            .request(
                Message::Authenticate {
                    request_id,
                    username: username.into(),
                },
                request_id,
            )
            .await?;

        match reply {
            Message::AuthenticationResult { session, .. } => {
                self.session.lock().await.replace(session.clone());
                Ok(session)
            }
            Message::Error { message, .. } => Err(Error::authentication(message)),
            other => Err(Error::protocol(format!(
                "unexpected authentication reply: {other:?}"
            ))),
        }
    }

    /// The cached session, if authenticated.
    pub async fn session(&self) -> Option<SessionDescriptor> {
        self.session.lock().await.clone()
    }

    /// Creates a Cube owned by the authenticated user.
    ///
    /// # Errors
    /// Transport, authorization, or domain errors.
    pub async fn create_cube(&self, name: &str) -> Result<CubeDescriptor> {
        let request_id = self.next_request_id();
        let reply = self
            .request(
                Message::CreateCube {
                    request_id,
                    name: name.into(),
                },
                request_id,
            )
            .await?;
        match reply {
            Message::CubeCreated { cube, .. } => Ok(cube),
            Message::Error { message, .. } => Err(Error::workspace(message)),
            other => Err(Error::protocol(format!(
                "unexpected create reply: {other:?}"
            ))),
        }
    }

    /// Lists Cubes visible to the authenticated user.
    ///
    /// # Errors
    /// Transport or domain errors.
    pub async fn list_cubes(&self) -> Result<Vec<CubeDescriptor>> {
        let request_id = self.next_request_id();
        let reply = self
            .request(Message::ListCubes { request_id }, request_id)
            .await?;
        match reply {
            Message::CubeList { cubes, .. } => Ok(cubes),
            Message::Error { message, .. } => Err(Error::workspace(message)),
            other => Err(Error::protocol(format!("unexpected list reply: {other:?}"))),
        }
    }

    /// Requests the server plugin roster (Part 11). Not yet presented by
    /// session consumers; transports the raw records.
    ///
    /// # Errors
    /// Transport or domain errors.
    pub async fn plugin_status(&self) -> Result<Vec<nexus_protocol::PluginRecordInfo>> {
        let request_id = self.next_request_id();
        let reply = self
            .request(Message::PluginStatusRequest { request_id }, request_id)
            .await?;
        match reply {
            Message::PluginStatus { plugins, .. } => Ok(plugins),
            Message::Error { message, .. } => Err(Error::workspace(message)),
            other => Err(Error::protocol(format!(
                "unexpected plugin status reply: {other:?}"
            ))),
        }
    }

    /// Joins a Cube.
    ///
    /// # Errors
    /// Transport, authorization, or domain errors.
    pub async fn join_cube(&self, cube_id: CubeId) -> Result<CubeDescriptor> {
        let request_id = self.next_request_id();
        let reply = self
            .request(
                Message::JoinCube {
                    request_id,
                    cube_id,
                },
                request_id,
            )
            .await?;
        match reply {
            Message::CubeJoined { cube, .. } => Ok(cube),
            Message::Error { message, .. } => Err(Error::workspace(message)),
            other => Err(Error::protocol(format!("unexpected join reply: {other:?}"))),
        }
    }

    /// Leaves a Cube.
    ///
    /// # Errors
    /// Transport, authorization, or domain errors.
    pub async fn leave_cube(&self, cube_id: CubeId) -> Result<()> {
        let request_id = self.next_request_id();
        let reply = self
            .request(
                Message::LeaveCube {
                    request_id,
                    cube_id,
                },
                request_id,
            )
            .await?;
        match reply {
            Message::CubeLeft { .. } => Ok(()),
            Message::Error { message, .. } => Err(Error::workspace(message)),
            other => Err(Error::protocol(format!(
                "unexpected leave reply: {other:?}"
            ))),
        }
    }

    /// Lists the workspace files of a Cube (workspace metadata + file listing).
    ///
    /// # Errors
    /// Transport, authorization, or domain errors.
    pub async fn list_workspace(
        &self,
        cube_id: CubeId,
    ) -> Result<(WorkspaceMetadata, Vec<FileEntry>)> {
        let request_id = self.next_request_id();
        let reply = self
            .request(
                Message::ListWorkspace {
                    request_id,
                    cube_id,
                },
                request_id,
            )
            .await?;
        match reply {
            Message::WorkspaceList {
                workspace, files, ..
            } => Ok((workspace, files)),
            Message::Error { message, .. } => Err(Error::workspace(message)),
            other => Err(Error::protocol(format!(
                "unexpected workspace list reply: {other:?}"
            ))),
        }
    }

    /// Creates a file in a Cube workspace.
    ///
    /// # Errors
    /// Transport, authorization, or domain errors.
    pub async fn create_file(&self, cube_id: CubeId, path: &str) -> Result<()> {
        self.ack_request(Message::CreateFile {
            request_id: self.next_request_id(),
            cube_id,
            path: path.into(),
        })
        .await
    }

    /// Reads the contents of a file from a Cube workspace.
    ///
    /// # Errors
    /// Transport, authorization, or domain errors.
    pub async fn read_file(&self, cube_id: CubeId, path: &str) -> Result<(Version, Vec<u8>)> {
        let request_id = self.next_request_id();
        let reply = self
            .request(
                Message::ReadFile {
                    request_id,
                    cube_id,
                    path: path.into(),
                },
                request_id,
            )
            .await?;
        match reply {
            Message::FileContent {
                version, content, ..
            } => Ok((version, content)),
            Message::Error { message, .. } => Err(Error::workspace(message)),
            other => Err(Error::protocol(format!(
                "unexpected file content reply: {other:?}"
            ))),
        }
    }

    /// Writes the contents of a file into a Cube workspace.
    ///
    /// # Errors
    /// Transport, authorization, or domain errors.
    pub async fn write_file(&self, cube_id: CubeId, path: &str, content: &[u8]) -> Result<()> {
        self.ack_request(Message::WriteFile {
            request_id: self.next_request_id(),
            cube_id,
            path: path.into(),
            content: content.to_vec(),
        })
        .await
    }

    /// Deletes a file from a Cube workspace.
    ///
    /// # Errors
    /// Transport, authorization, or domain errors.
    pub async fn delete_file(&self, cube_id: CubeId, path: &str) -> Result<()> {
        self.ack_request(Message::DeleteFile {
            request_id: self.next_request_id(),
            cube_id,
            path: path.into(),
        })
        .await
    }

    /// Renames a file inside a Cube workspace.
    ///
    /// # Errors
    /// Transport, authorization, or domain errors.
    pub async fn rename_file(&self, cube_id: CubeId, from: &str, to: &str) -> Result<()> {
        self.ack_request(Message::RenameFile {
            request_id: self.next_request_id(),
            cube_id,
            from: from.into(),
            to: to.into(),
        })
        .await
    }

    // ---- Part 10: builds ----------------------------------------------------

    /// Submits a build to the server's queue for `cube_id`.
    ///
    /// # Errors
    /// Transport, authorization, or domain errors.
    pub async fn submit_build(
        &self,
        cube_id: CubeId,
        target: nexus_core::BuildTarget,
        configuration: nexus_core::BuildConfiguration,
    ) -> Result<nexus_core::ids::BuildId> {
        let request_id = self.next_request_id();
        let reply = self
            .request(
                Message::BuildSubmit {
                    request_id,
                    cube_id,
                    target,
                    configuration,
                },
                request_id,
            )
            .await?;
        match reply {
            Message::BuildQueued { build_id, .. } => Ok(build_id),
            Message::Error { message, .. } => Err(Error::workspace(message)),
            other => Err(Error::protocol(format!(
                "unexpected build submit reply: {other:?}"
            ))),
        }
    }

    /// Requests cancellation of a running or queued build.
    ///
    /// # Errors
    /// Transport, authorization, or domain errors.
    pub async fn cancel_build(&self, build_id: nexus_core::ids::BuildId) -> Result<()> {
        let request_id = self.next_request_id();
        let reply = self
            .request(
                Message::BuildCancel {
                    request_id,
                    build_id,
                },
                request_id,
            )
            .await?;
        match reply {
            Message::BuildCancelled { .. } => Ok(()),
            Message::Error { message, .. } => Err(Error::workspace(message)),
            other => Err(Error::protocol(format!(
                "unexpected build cancel reply: {other:?}"
            ))),
        }
    }

    // ---- Part 10: terminals -------------------------------------------------

    /// Creates and starts a PTY terminal inside `cube_id`.
    ///
    /// # Errors
    /// Transport, authorization, or PTY spawn errors.
    pub async fn create_terminal(
        &self,
        cube_id: CubeId,
        shell: Option<String>,
        working_directory: &str,
        size: nexus_core::TerminalSize,
        visibility: nexus_core::TerminalVisibility,
    ) -> Result<nexus_core::TerminalSession> {
        let request_id = self.next_request_id();
        let reply = self
            .request(
                Message::TerminalCreate {
                    request_id,
                    cube_id,
                    shell,
                    working_directory: working_directory.into(),
                    size,
                    visibility,
                },
                request_id,
            )
            .await?;
        match reply {
            Message::TerminalCreated { session, .. } => Ok(session),
            Message::Error { message, .. } => Err(Error::workspace(message)),
            other => Err(Error::protocol(format!(
                "unexpected terminal create reply: {other:?}"
            ))),
        }
    }

    /// Writes stdin bytes to a terminal (one-way, no reply expected).
    ///
    /// # Errors
    /// Transport errors, or a closed connection.
    pub async fn write_terminal(
        &self,
        terminal_id: nexus_core::ids::TerminalId,
        data: &[u8],
    ) -> Result<()> {
        self.send_one_way(Message::TerminalWrite {
            terminal_id,
            data: data.to_vec(),
        })
        .await
    }

    /// Sends a control signal to a terminal.
    ///
    /// # Errors
    /// Transport, authorization, or domain errors.
    pub async fn signal_terminal(
        &self,
        terminal_id: nexus_core::ids::TerminalId,
        signal: nexus_protocol::message::TerminalSignalKind,
    ) -> Result<()> {
        self.ack_request(Message::TerminalSignal {
            request_id: self.next_request_id(),
            terminal_id,
            signal,
        })
        .await
    }

    /// Closes a terminal, returning its exit code when the server captured it.
    ///
    /// # Errors
    /// Transport, authorization, or domain errors.
    pub async fn close_terminal(
        &self,
        terminal_id: nexus_core::ids::TerminalId,
    ) -> Result<Option<i32>> {
        let request_id = self.next_request_id();
        let reply = self
            .request(
                Message::TerminalClose {
                    request_id,
                    terminal_id,
                },
                request_id,
            )
            .await?;
        match reply {
            Message::TerminalClosed { exit_code, .. } => Ok(exit_code),
            Message::Error { message, .. } => Err(Error::workspace(message)),
            other => Err(Error::protocol(format!(
                "unexpected terminal close reply: {other:?}"
            ))),
        }
    }

    /// Sends a keepalive heartbeat and awaits the server's acknowledgement.
    ///
    /// # Errors
    /// Transport errors, or a closed connection.
    pub async fn send_heartbeat(&self) -> Result<()> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(Error::network("client is closed"));
        }
        let mut transport = self.transport.lock().await;
        transport
            .send(&Message::Heartbeat {
                at: Timestamp::now(),
            })
            .await
            .map_err(to_core_error)?;

        loop {
            match transport.receive().await.map_err(to_core_error)? {
                Some(Message::HeartbeatAck { .. }) => return Ok(()),
                Some(_other) => continue,
                None => return Err(Error::network("connection closed by server")),
            }
        }
    }

    /// Closes the connection gracefully.
    pub async fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::Relaxed) {
            return Ok(());
        }
        let mut transport = self.transport.lock().await;
        let _ = transport
            .send(&Message::Disconnect {
                reason: "client closing".into(),
            })
            .await;
        transport.close().await.map_err(to_core_error)
    }

    fn next_request_id(&self) -> RequestId {
        RequestId::from_u64(self.pending_seq.fetch_add(1, Ordering::Relaxed))
    }

    /// Sends a request whose success response is an `Ack`.
    async fn ack_request(&self, message: Message) -> Result<()> {
        let request_id = match message.response_request_id() {
            Some(id) => id,
            None => return Err(Error::protocol("request carries no id")),
        };
        let reply = self.request(message, request_id).await?;
        match reply {
            Message::Ack { ok: true, .. } => Ok(()),
            Message::Ack {
                ok: false, message, ..
            } => Err(Error::workspace(
                message.unwrap_or_else(|| "operation failed".into()),
            )),
            Message::Error { message, .. } => Err(Error::workspace(message)),
            other => Err(Error::protocol(format!(
                "unexpected acknowledgement reply: {other:?}"
            ))),
        }
    }

    /// Sends a fire-and-forget message without expecting a reply.
    async fn send_one_way(&self, message: Message) -> Result<()> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(Error::network("client is closed"));
        }
        let mut transport = self.transport.lock().await;
        transport.send(&message).await.map_err(to_core_error)
    }

    async fn request(&self, message: Message, request_id: RequestId) -> Result<Message> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(Error::network("client is closed"));
        }
        let mut transport = self.transport.lock().await;
        transport.send(&message).await.map_err(to_core_error)?;

        loop {
            let received = match tokio::time::timeout(REQUEST_TIMEOUT, transport.receive()).await {
                Ok(Ok(Some(reply))) => reply,
                Ok(Ok(None)) => return Err(Error::network("connection closed by server")),
                Ok(Err(err)) => return Err(to_core_error(err)),
                Err(_) => return Err(Error::network("request timed out")),
            };

            if request_id_of(&received) == Some(request_id) {
                return Ok(received);
            }
            match received {
                Message::HeartbeatAck { .. } => continue,
                other => {
                    warn!("received unsolicited server message; ignoring");
                    let _ = other;
                }
            }
        }
    }
}

fn request_id_of(message: &Message) -> Option<RequestId> {
    match message {
        Message::AuthenticationResult { request_id, .. }
        | Message::CubeCreated { request_id, .. }
        | Message::CubeList { request_id, .. }
        | Message::CubeJoined { request_id, .. }
        | Message::CubeLeft { request_id, .. }
        | Message::WorkspaceList { request_id, .. }
        | Message::FileContent { request_id, .. }
        | Message::Ack { request_id, .. }
        | Message::BuildQueued { request_id, .. }
        | Message::BuildCancelled { request_id, .. }
        | Message::TerminalCreated { request_id, .. }
        | Message::TerminalResize { request_id, .. }
        | Message::TerminalSignal { request_id, .. }
        | Message::TerminalClose { request_id, .. }
        | Message::TerminalClosed { request_id, .. }
        | Message::ListTerminals { request_id, .. }
        | Message::TerminalList { request_id, .. }
        | Message::PluginStatus { request_id, .. } => Some(*request_id),
        Message::Error { request_id, .. } => *request_id,
        _ => None,
    }
}

fn to_core_error(err: TransportError) -> Error {
    match err {
        TransportError::ConnectionClosed => {
            Error::network("connection closed").with_internal("transport closed")
        }
        TransportError::Io(io) => Error::network("network I/O failure").with_source(io),
        TransportError::Protocol(msg) => {
            Error::protocol(msg).with_internal("transport protocol error")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_increment() {
        assert_ne!(RequestId::from_u64(1), RequestId::from_u64(2));
    }

    #[test]
    fn request_id_extraction() {
        let msg = Message::CubeCreated {
            request_id: RequestId::from_u64(7),
            cube: CubeDescriptor::new(CubeId::new(), "x", nexus_core::ids::UserId::new()),
        };
        assert_eq!(request_id_of(&msg), Some(RequestId::from_u64(7)));
        assert_eq!(
            request_id_of(&Message::Heartbeat {
                at: Timestamp::now()
            }),
            None
        );
    }
}
