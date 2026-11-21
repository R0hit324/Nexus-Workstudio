//! Terminal session management.
//!
//! Provides terminal session lifecycle management, configuration, and
//! integration with the runtime system.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use nexus_core::ids::RuntimeId;

/// Terminal session manager.
pub struct TerminalManager {
    sessions: Arc<RwLock<HashMap<nexus_core::TerminalId, ManagedTerminalSession>>>,
    pty_backend: Arc<dyn crate::pty::PtyBackend>,
    #[allow(dead_code)]
    default_config: TerminalSessionConfig,
    event_tx: tokio::sync::mpsc::UnboundedSender<crate::session::TerminalEvent>,
}

/// A managed terminal session with PTY handle.
#[allow(dead_code)]
struct ManagedTerminalSession {
    session: nexus_core::TerminalSession,
    pty_handle: Option<crate::pty::PtyPairHandle>,
    pty_backend: Arc<dyn crate::pty::PtyBackend>,
    output_tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    resize_tx: tokio::sync::mpsc::UnboundedSender<nexus_core::TerminalSize>,
    parser: crate::parser::AnsiParser,
    renderer: crate::render::TerminalRenderer,
    input_ownership: Option<nexus_core::InputOwnership>,
    shared_users: Vec<nexus_core::UserId>,
}

impl ManagedTerminalSession {
    fn new(
        _config: TerminalSessionConfig,
        _pty_backend: Arc<dyn crate::pty::PtyBackend>,
        _event_tx: tokio::sync::mpsc::UnboundedSender<crate::session::TerminalEvent>,
    ) -> Self {
        let (_output_tx, _output_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let (_resize_tx, _resize_rx) =
            tokio::sync::mpsc::unbounded_channel::<nexus_core::TerminalSize>();

        Self {
            session: nexus_core::TerminalSession {
                terminal_id: nexus_core::TerminalId::new(),
                cube_id: nexus_core::CubeId::new(),
                owner: nexus_core::UserId::new(),
                visibility: nexus_core::TerminalVisibility::Private,
                runtime_id: RuntimeId::new(),
                process_id: 0,
                shell: "bash".to_string(),
                working_directory: "/workspace".to_string(),
                size: nexus_core::TerminalSize::default(),
                state: nexus_core::TerminalState::Created,
                created_at: nexus_core::Timestamp::now(),
                started_at: None,
                exited_at: None,
                exit_code: None,
            },
            pty_handle: None,
            pty_backend: Arc::new(crate::pty::NativePtyBackend::new()),
            output_tx: tokio::sync::mpsc::unbounded_channel::<Vec<u8>>().0,
            resize_tx: tokio::sync::mpsc::unbounded_channel::<nexus_core::TerminalSize>().0,
            parser: crate::parser::AnsiParser::new(),
            renderer: crate::render::TerminalRenderer::new(crate::render::RenderConfig::default()),
            input_ownership: None,
            shared_users: Vec::new(),
        }
    }
}

/// Terminal session configuration.
#[derive(Debug, Clone)]
pub struct TerminalSessionConfig {
    pub shell: String,
    pub shell_args: Vec<String>,
    pub working_directory: String,
    pub size: nexus_core::TerminalSize,
    pub visibility: nexus_core::TerminalVisibility,
    pub environment: Vec<(String, String)>,
    pub auto_start: bool,
    pub scrollback_limit: usize,
}

impl Default for TerminalSessionConfig {
    fn default() -> Self {
        Self {
            shell: "bash".to_string(),
            shell_args: Vec::new(),
            working_directory: "/workspace".to_string(),
            size: nexus_core::TerminalSize::default(),
            visibility: nexus_core::TerminalVisibility::Private,
            environment: Vec::new(),
            auto_start: true,
            scrollback_limit: 10000,
        }
    }
}

/// Terminal events for the event bus.
#[derive(Debug, Clone)]
pub enum TerminalEvent {
    Created {
        terminal_id: nexus_core::TerminalId,
        cube_id: nexus_core::CubeId,
        owner: nexus_core::UserId,
        visibility: nexus_core::TerminalVisibility,
        timestamp: nexus_core::Timestamp,
    },
    Started {
        terminal_id: nexus_core::TerminalId,
        timestamp: nexus_core::Timestamp,
    },
    Output {
        terminal_id: nexus_core::TerminalId,
        data: Vec<u8>,
    },
    Resized {
        terminal_id: nexus_core::TerminalId,
        size: nexus_core::TerminalSize,
    },
    StateChanged {
        terminal_id: nexus_core::TerminalId,
        state: nexus_core::TerminalState,
    },
    Exited {
        terminal_id: nexus_core::TerminalId,
        exit_code: i32,
    },
    Closed {
        terminal_id: nexus_core::TerminalId,
        timestamp: nexus_core::Timestamp,
    },
    ControlRequest {
        terminal_id: nexus_core::TerminalId,
        requester: nexus_core::UserId,
    },
    ControlChanged {
        terminal_id: nexus_core::TerminalId,
        new_owner: nexus_core::UserId,
    },
}

impl TerminalManager {
    /// Creates a new terminal manager.
    pub fn new(
        event_tx: tokio::sync::mpsc::UnboundedSender<crate::session::TerminalEvent>,
    ) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            pty_backend: Arc::new(crate::pty::NativePtyBackend::new()),
            default_config: TerminalSessionConfig::default(),
            event_tx,
        }
    }

    /// Creates a new terminal session.
    pub async fn create_session(
        &self,
        cube_id: nexus_core::CubeId,
        owner: nexus_core::UserId,
        runtime_id: RuntimeId,
        config: TerminalSessionConfig,
    ) -> Result<nexus_core::TerminalId> {
        let terminal_id = nexus_core::TerminalId::new();

        let _session = nexus_core::TerminalSession {
            terminal_id,
            cube_id,
            owner,
            visibility: config.visibility,
            runtime_id,
            process_id: 0,
            shell: config.shell.clone(),
            working_directory: config.working_directory.clone(),
            size: config.size,
            state: nexus_core::TerminalState::Created,
            created_at: nexus_core::Timestamp::now(),
            started_at: None,
            exited_at: None,
            exit_code: None,
        };

        let _ = self.event_tx.send(crate::session::TerminalEvent::Created {
            terminal_id,
            cube_id,
            owner,
            visibility: config.visibility,
            timestamp: nexus_core::Timestamp::now(),
        });

        let mut sessions = self.sessions.write().await;
        sessions.insert(
            terminal_id,
            ManagedTerminalSession::new(config, self.pty_backend.clone(), self.event_tx.clone()),
        );

        Ok(terminal_id)
    }

    /// Starts a terminal session.
    pub async fn start_session(&self, terminal_id: nexus_core::TerminalId) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&terminal_id)
            .ok_or_else(|| anyhow::anyhow!("Terminal not found"))?;

        if session.session.state != nexus_core::TerminalState::Created {
            return Err(anyhow::anyhow!("Terminal not in Created state"));
        }

        let config = crate::pty::PtyConfig {
            size: session.session.size,
            working_directory: std::path::PathBuf::from(&session.session.working_directory),
            environment: Vec::new(),
            shell: session.session.shell.clone(),
            shell_args: Vec::new(),
        };

        let pty_handle = session.pty_backend.create(config).await?;
        session.session.process_id = pty_handle.process.process_id().unwrap_or(0) as u32;
        session.session.state = nexus_core::TerminalState::Starting;
        session.session.started_at = Some(nexus_core::Timestamp::now());

        // Spawn a dedicated reader thread: the PTY reader blocks, so it cannot
        // live on the async runtime. Output flows to subscribers via events.
        let reader = Arc::clone(&pty_handle.reader);
        let event_tx = self.event_tx.clone();
        let reader_terminal_id = session.session.terminal_id;
        let _reader_thread = std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = [0u8; 4096];
            loop {
                let n = {
                    let mut guard = reader
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    guard.read(&mut buf).unwrap_or(0)
                };
                if n == 0 {
                    break;
                }
                if event_tx
                    .send(crate::session::TerminalEvent::Output {
                        terminal_id: reader_terminal_id,
                        data: buf[..n].to_vec(),
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        session.pty_handle = Some(pty_handle);
        let _ = self.event_tx.send(crate::session::TerminalEvent::Started {
            terminal_id: session.session.terminal_id,
            timestamp: nexus_core::Timestamp::now(),
        });

        session.session.state = nexus_core::TerminalState::Running;
        Ok(())
    }

    /// Resizes a terminal.
    /// Note: PTY resize is not directly supported in portable-pty 0.9.
    /// This is a no-op for now.
    pub async fn resize(
        &self,
        _terminal_id: nexus_core::TerminalId,
        _size: nexus_core::TerminalSize,
    ) -> Result<()> {
        // PTY resize is not directly supported in portable-pty 0.9
        // This would require recreating the PTY with the new size
        Ok(())
    }

    /// Sends input to a terminal.
    pub async fn write_input(
        &self,
        terminal_id: nexus_core::TerminalId,
        data: &[u8],
    ) -> Result<()> {
        let sessions = self.sessions.write().await;
        if let Some(session) = sessions.get(&terminal_id) {
            if let Some(pty_handle) = &session.pty_handle {
                use std::io::Write;
                let mut writer = pty_handle
                    .writer
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                writer.write_all(data)?;
                writer.flush()?;
            } else {
                return Err(anyhow::anyhow!("Terminal not started"));
            }
        }
        Ok(())
    }

    /// Sends a signal to a terminal process.
    /// Note: Signal sending is not directly supported in portable-pty 0.9.
    /// This is a no-op for now.
    pub async fn signal(
        &self,
        _terminal_id: nexus_core::TerminalId,
        _signal: crate::pty::Signal,
    ) -> Result<()> {
        // Signal sending is not directly supported in portable-pty 0.9
        Ok(())
    }

    /// Closes a terminal session.
    pub async fn close_session(&self, terminal_id: nexus_core::TerminalId) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        if let Some(mut session) = sessions.remove(&terminal_id) {
            if let Some(mut pty_handle) = session.pty_handle.take() {
                session.pty_backend.close(&mut pty_handle).await?;
            }
            let _ = self.event_tx.send(crate::session::TerminalEvent::Closed {
                terminal_id,
                timestamp: nexus_core::Timestamp::now(),
            });
        }
        Ok(())
    }

    /// Gets a terminal session.
    pub async fn get_session(
        &self,
        terminal_id: nexus_core::TerminalId,
    ) -> Option<nexus_core::TerminalSession> {
        let sessions = self.sessions.read().await;
        sessions.get(&terminal_id).map(|s| s.session.clone())
    }

    /// Lists all terminal sessions for a cube.
    pub async fn list_sessions(
        &self,
        cube_id: nexus_core::CubeId,
    ) -> Vec<nexus_core::TerminalSession> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .filter(|s| s.session.cube_id == cube_id)
            .map(|s| s.session.clone())
            .collect()
    }
}

impl Default for TerminalManager {
    fn default() -> Self {
        let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
        Self::new(event_tx)
    }
}
