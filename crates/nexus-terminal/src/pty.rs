//! PTY backend abstraction.
//!
//! Provides a platform-agnostic interface for creating and managing
//! pseudoterminals (PTYs) with support for different platform backends.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use portable_pty::{native_pty_system, Child, CommandBuilder, PtySize, PtySystem};
use tokio::sync::Mutex;

use nexus_core::TerminalSize;

/// PTY configuration.
#[derive(Debug, Clone)]
pub struct PtyConfig {
    pub size: TerminalSize,
    pub working_directory: PathBuf,
    pub environment: Vec<(String, String)>,
    pub shell: String,
    pub shell_args: Vec<String>,
}

impl Default for PtyConfig {
    fn default() -> Self {
        Self {
            size: TerminalSize::default(),
            working_directory: PathBuf::from("/workspace"),
            environment: Vec::new(),
            shell: "bash".to_string(),
            shell_args: Vec::new(),
        }
    }
}

/// A PTY pair with reader/writer handles.
///
/// The byte handles are shared behind a mutex so the session manager can hold
/// them inside structures that must be `Sync` (the server references terminal
/// state through `Arc`), while a dedicated thread can read output.
pub struct PtyPairHandle {
    pub reader: Arc<std::sync::Mutex<Box<dyn Read + Send>>>,
    pub writer: Arc<std::sync::Mutex<Box<dyn Write + Send>>>,
    pub process: Box<dyn Child + Send + Sync>,
}

/// PTY backend trait for platform-specific PTY implementations.
#[async_trait]
pub trait PtyBackend: Send + Sync {
    /// Creates a new PTY with the given configuration.
    async fn create(&self, config: PtyConfig) -> Result<PtyPairHandle>;

    /// Waits for the PTY process to exit.
    async fn wait(&self, handle: &mut PtyPairHandle) -> Result<i32>;

    /// Closes the PTY.
    async fn close(&self, handle: &mut PtyPairHandle) -> Result<()>;
}

/// Process signals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    SigInt,
    SigTerm,
    SigKill,
    SigWinch,
}

/// Native PTY backend using portable-pty.
pub struct NativePtyBackend {
    pty_system: Arc<Mutex<Box<dyn PtySystem + Send>>>,
}

impl NativePtyBackend {
    pub fn new() -> Self {
        Self {
            pty_system: Arc::new(Mutex::new(native_pty_system())),
        }
    }
}

#[async_trait]
impl PtyBackend for NativePtyBackend {
    async fn create(&self, config: PtyConfig) -> Result<PtyPairHandle> {
        let pty_size = PtySize {
            rows: config.size.rows,
            cols: config.size.cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pty_system = self.pty_system.lock().await;
        let pair = pty_system.openpty(pty_size)?;

        let mut cmd = CommandBuilder::new(&config.shell);
        cmd.args(&config.shell_args);
        cmd.cwd(&config.working_directory);
        for (key, value) in config.environment {
            cmd.env(key, value);
        }

        let child = pair.slave.spawn_command(cmd)?;

        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        Ok(PtyPairHandle {
            reader: Arc::new(std::sync::Mutex::new(Box::new(reader))),
            writer: Arc::new(std::sync::Mutex::new(Box::new(writer))),
            process: child,
        })
    }

    async fn wait(&self, handle: &mut PtyPairHandle) -> Result<i32> {
        let status = handle.process.wait()?;
        Ok(status.exit_code() as i32)
    }

    async fn close(&self, handle: &mut PtyPairHandle) -> Result<()> {
        // Try to terminate gracefully
        let _ = handle.process.kill();
        let _ = handle.process.wait();
        Ok(())
    }
}

impl Default for NativePtyBackend {
    fn default() -> Self {
        Self::new()
    }
}
