//! Terminal service: bridges the `nexus-terminal` manager to protocol messages.
//!
//! Owns a [`nexus_terminal::TerminalManager`], tracks which session owns which
//! terminal, pumps PTY output back to the owning connection, and exposes the
//! operations the protocol dispatcher needs.

use std::collections::HashMap;
use std::sync::Arc;

use nexus_core::ids::{CubeId, SessionId, TerminalId, UserId};
use nexus_core::{TerminalSession, TerminalSize, TerminalState};
use nexus_protocol::message::{Message, TerminalSignalKind};
use tokio::sync::RwLock;

use crate::connection::manager::{ConnectionManager, Delivery};

/// Server-facing facade over terminal sessions.
pub struct TerminalService {
    manager: nexus_terminal::TerminalManager,
    /// Maps each terminal to the session that owns its output stream.
    routes: Arc<RwLock<HashMap<TerminalId, SessionId>>>,
}

impl TerminalService {
    /// Creates the service, wiring its output pump to `connection_manager`.
    #[must_use]
    pub fn new(connection_manager: Arc<ConnectionManager>) -> Self {
        let (event_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let manager = nexus_terminal::TerminalManager::new(event_tx);
        let routes: Arc<RwLock<HashMap<TerminalId, SessionId>>> =
            Arc::new(RwLock::new(HashMap::new()));

        // Output pump: forwards PTY bytes to the owning session's connection.
        let pump_routes = routes.clone();
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                match event {
                    nexus_terminal::TerminalEvent::Output { terminal_id, data } => {
                        let session_id = pump_routes.read().await.get(&terminal_id).copied();
                        if let Some(session_id) = session_id {
                            let _ = connection_manager
                                .send_to_session(
                                    session_id,
                                    Message::TerminalOutput { terminal_id, data },
                                    Delivery::Try,
                                )
                                .await;
                        }
                    }
                    nexus_terminal::TerminalEvent::Closed { terminal_id, .. } => {
                        // Capture the owner before dropping the route.
                        let session_id = pump_routes.read().await.get(&terminal_id).copied();
                        pump_routes.write().await.remove(&terminal_id);
                        // Inform the owner the terminal is gone (state change
                        // covers both exit and close).
                        if let Some(session_id) = session_id {
                            let _ = connection_manager
                                .send_to_session(
                                    session_id,
                                    Message::TerminalStateChanged {
                                        terminal_id,
                                        state: TerminalState::Stopped,
                                    },
                                    Delivery::Try,
                                )
                                .await;
                        }
                    }
                    _ => {}
                }
            }
        });

        Self { manager, routes }
    }

    /// Creates and starts a terminal session; returns its descriptor.
    pub async fn create(
        &self,
        cube_id: CubeId,
        owner: UserId,
        session_id: SessionId,
        shell: Option<String>,
        working_directory: String,
        size: TerminalSize,
        visibility: nexus_core::TerminalVisibility,
    ) -> anyhow::Result<TerminalSession> {
        let config = nexus_terminal::TerminalSessionConfig {
            shell: shell.unwrap_or_else(default_shell),
            shell_args: Vec::new(),
            working_directory,
            size,
            visibility,
            environment: Vec::new(),
            auto_start: true,
            scrollback_limit: 10_000,
        };

        let terminal_id = self
            .manager
            .create_session(cube_id, owner, nexus_core::ids::RuntimeId::new(), config)
            .await?;

        self.routes.write().await.insert(terminal_id, session_id);

        // Start the PTY; if spawning fails, remove the route and surface the
        // error to the caller (the session stays listable as failed).
        if let Err(err) = self.manager.start_session(terminal_id).await {
            self.routes.write().await.remove(&terminal_id);
            return Err(err);
        }

        self.manager
            .get_session(terminal_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("terminal vanished immediately after start"))
    }

    /// Writes stdin bytes to a terminal.
    pub async fn write(&self, terminal_id: TerminalId, data: &[u8]) -> anyhow::Result<()> {
        self.manager.write_input(terminal_id, data).await
    }

    /// Resizes a terminal.
    pub async fn resize(&self, terminal_id: TerminalId, size: TerminalSize) -> anyhow::Result<()> {
        self.manager.resize(terminal_id, size).await
    }

    /// Delivers a control signal.
    pub async fn signal(
        &self,
        terminal_id: TerminalId,
        signal: TerminalSignalKind,
    ) -> anyhow::Result<()> {
        let mapped = match signal {
            TerminalSignalKind::Interrupt => nexus_terminal::pty::Signal::SigInt,
            TerminalSignalKind::Terminate
            | TerminalSignalKind::Hangup
            | TerminalSignalKind::Quit => nexus_terminal::pty::Signal::SigTerm,
            TerminalSignalKind::Kill => nexus_terminal::pty::Signal::SigKill,
        };
        self.manager.signal(terminal_id, mapped).await
    }

    /// Closes a terminal; returns the last observed exit code when available.
    pub async fn close(&self, terminal_id: TerminalId) -> anyhow::Result<Option<i32>> {
        let session = self.get(terminal_id).await;
        self.manager.close_session(terminal_id).await?;
        self.routes.write().await.remove(&terminal_id);
        Ok(session.and_then(|s| s.exit_code))
    }

    /// Fetches a session descriptor.
    pub async fn get(&self, terminal_id: TerminalId) -> Option<TerminalSession> {
        self.manager.get_session(terminal_id).await
    }

    /// Lists the terminals of a Cube.
    pub async fn list(&self, cube_id: CubeId) -> Vec<TerminalSession> {
        self.manager.list_sessions(cube_id).await
    }
}

/// Platform-appropriate default shell when the client does not name one.
#[must_use]
fn default_shell() -> String {
    if cfg!(windows) {
        "powershell".to_string()
    } else {
        "/bin/bash".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn close_unknown_terminal_is_ok() {
        let service = TerminalService::new(Arc::new(ConnectionManager::new()));
        let result = service.close(TerminalId::new()).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_empty_cube_has_no_terminals() {
        let service = TerminalService::new(Arc::new(ConnectionManager::new()));
        assert!(service.list(CubeId::new()).await.is_empty());
    }
}
