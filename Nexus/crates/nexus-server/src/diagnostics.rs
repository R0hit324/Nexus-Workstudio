//! Operational diagnostics and crash reporting (Part 15, sections 52-53).
//!
//! Deployment operations require two observability surfaces that the domain
//! tests cover indirectly but production needs explicitly:
//!
//! - **Diagnostics**: a structured snapshot of an otherwise-running server,
//!   used for support bundles and troubleshooting output.
//! - **Crash handling**: a process-wide panic hook that writes a crash report
//!   before the default handler takes over.

use std::time::Duration;

use crate::lifecycle::ServerLifecycle;
use crate::server::RunningServer;
use crate::SERVER_NAME;
use nexus_core::version::PROTOCOL_VERSION;

/// Version reported by this server build.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Snapshot of operational state used for diagnostic bundles and support
/// output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticReport {
    /// Name of the server as reported during the handshake.
    pub server_name: &'static str,
    /// Semantic version of this server build.
    pub server_version: String,
    /// Wire protocol version in use.
    pub protocol_version: u32,
    /// Current lifecycle stage.
    pub lifecycle: ServerLifecycle,
    /// How long the server has been up.
    pub uptime: Duration,
    /// Currently accepted connections.
    pub active_connections: usize,
    /// Currently authenticated sessions.
    pub active_sessions: usize,
}

impl DiagnosticReport {
    /// Builds a diagnostic report from explicit facts (used by tests).
    #[must_use]
    pub fn new(
        server_name: &'static str,
        server_version: impl Into<String>,
        protocol_version: u32,
        lifecycle: ServerLifecycle,
        uptime: Duration,
        active_connections: usize,
        active_sessions: usize,
    ) -> Self {
        Self {
            server_name,
            server_version: server_version.into(),
            protocol_version,
            lifecycle,
            uptime,
            active_connections,
            active_sessions,
        }
    }

    /// True when the reported state can still serve traffic.
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        !matches!(
            self.lifecycle,
            ServerLifecycle::Failed | ServerLifecycle::Stopped
        )
    }
}

/// Gathers the diagnostic report for a running server.
#[must_use]
pub fn collect(server: &RunningServer, uptime: Duration) -> DiagnosticReport {
    DiagnosticReport {
        server_name: SERVER_NAME,
        server_version: SERVER_VERSION.to_string(),
        protocol_version: PROTOCOL_VERSION.get(),
        lifecycle: ServerLifecycle::Running,
        uptime,
        active_connections: server
            .state()
            .active_connections
            .load(std::sync::atomic::Ordering::Relaxed),
        active_sessions: server
            .state()
            .active_sessions
            .load(std::sync::atomic::Ordering::Relaxed),
    }
}

/// Crash report captured by the process-wide panic hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashReport {
    /// Name of the panicking thread.
    pub thread: String,
    pub location: String,
    pub message: String,
    pub timestamp_ms: u64,
}

impl CrashReport {
    /// Builds a crash report from panic details.
    #[must_use]
    pub fn new(
        thread: impl Into<String>,
        location: impl Into<String>,
        message: impl Into<String>,
        timestamp_ms: u64,
    ) -> Self {
        Self {
            thread: thread.into(),
            location: location.into(),
            message: message.into(),
            timestamp_ms,
        }
    }
}

impl std::fmt::Display for CrashReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "panic at {} on thread {}: {}",
            self.location, self.thread, self.message
        )
    }
}

impl std::fmt::Display for DiagnosticReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} v{} (protocol {}) lifecycle {:?} uptime {:?} connections {} sessions {}",
            self.server_name,
            self.server_version,
            self.protocol_version,
            self.lifecycle,
            self.uptime,
            self.active_connections,
            self.active_sessions
        )
    }
}

/// Captures the current panic information into a [`CrashReport`].
fn capture(info: &std::panic::PanicHookInfo<'_>) -> CrashReport {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&'static str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_string());
    CrashReport::new(
        std::thread::current().name().unwrap_or("unknown"),
        info.location()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        message,
        0,
    )
}

/// Installs a process-wide panic hook that writes a crash report to `path`
/// (when provided) before delegating to the previous hook.
///
/// Write failures are ignored; the default hook still runs.
pub fn install_crash_handler(path: Option<&std::path::Path>) {
    let default_hook = std::panic::take_hook();
    let owned_path = path.map(std::path::Path::to_path_buf);
    std::panic::set_hook(Box::new(move |info| {
        let report = capture(info);
        if let Some(path) = owned_path.as_deref() {
            let line = format!("{report}\n");
            let _ = std::fs::write(path, line);
        }
        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn diagnostic_report_new_reports_explicit_facts() {
        let report = DiagnosticReport::new(
            "nexus-server",
            "0.1.0",
            1,
            ServerLifecycle::Running,
            Duration::from_secs(3),
            1,
            2,
        );
        assert_eq!(report.server_name, "nexus-server");
        assert_eq!(report.server_version, "0.1.0");
        assert_eq!(report.protocol_version, 1);
        assert!(report.is_healthy());
        assert_eq!(report.active_connections, 1);
        assert_eq!(report.active_sessions, 2);
    }

    #[test]
    fn diagnostic_report_is_unhealthy_after_stop() {
        let report = DiagnosticReport::new(
            "nexus-server",
            "0.1.0",
            1,
            ServerLifecycle::Stopped,
            Duration::ZERO,
            0,
            0,
        );
        assert!(!report.is_healthy());
    }

    #[test]
    fn diagnostic_report_display_contains_key_facts() {
        let report = DiagnosticReport::new(
            "nexus-server",
            "0.1.0",
            1,
            ServerLifecycle::Running,
            Duration::from_secs(3),
            1,
            2,
        );
        let rendered = report.to_string();
        assert!(rendered.contains("nexus-server"));
        assert!(rendered.contains("0.1.0"));
        assert!(rendered.contains("connections 1"));
    }

    #[test]
    fn crash_report_formats_location_and_message() {
        let report = CrashReport::new("main", "/tmp/x.rs:12:5", "boom", 42);
        assert_eq!(report.thread, "main");
        assert_eq!(report.location, "/tmp/x.rs:12:5");
        assert_eq!(report.message, "boom");
        assert_eq!(report.timestamp_ms, 42);
        assert!(report.to_string().contains("boom"));
    }

    #[test]
    fn crash_handler_install_is_safe_to_call() {
        install_crash_handler(Some(std::path::Path::new("/tmp/nexus-crash-report.txt")));
    }
}
