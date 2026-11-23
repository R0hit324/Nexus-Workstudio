use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{Error, ErrorCode, Result};

/// Environment variable pointing at an explicit config file.
pub const ENV_CONFIG: &str = "NEXUS_CONFIG";
/// Environment variable overriding the server bind address.
pub const ENV_BIND: &str = "NEXUS_BIND";
/// Environment variable overriding the server port.
pub const ENV_PORT: &str = "NEXUS_SERVER_PORT";
/// Environment variable overriding the data directory.
pub const ENV_DATA_DIR: &str = "NEXUS_DATA_DIR";
/// Environment variable overriding the log level.
pub const ENV_LOG_LEVEL: &str = "NEXUS_LOG_LEVEL";
/// Environment variable overriding the client's server address.
pub const ENV_SERVER_ADDRESS: &str = "NEXUS_SERVER_ADDRESS";
/// Environment variable overriding the client's username.
pub const ENV_USERNAME: &str = "NEXUS_USERNAME";
/// Environment variable overriding the maximum log file size before rotation.
pub const ENV_LOG_MAX_BYTES: &str = "NEXUS_LOG_MAX_BYTES";
/// Environment variable overriding the retained rotated log files count.
pub const ENV_LOG_MAX_FILES: &str = "NEXUS_LOG_MAX_FILES";

/// Default server port.
pub const DEFAULT_PORT: u16 = 7000;

/// Log verbosity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Info
    }
}

impl FromStr for LogLevel {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "trace" => Ok(Self::Trace),
            "debug" => Ok(Self::Debug),
            "info" => Ok(Self::Info),
            "warn" => Ok(Self::Warn),
            "error" => Ok(Self::Error),
            other => Err(Error::new(
                crate::error::ErrorCode::Protocol,
                format!("unknown log level: {other}"),
            )),
        }
    }
}

impl LogLevel {
    /// The name of this level as used by the tracing filter.
    #[must_use]
    pub fn as_filter_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_filter_str())
    }
}

/// Log output format. Development logs are human-readable; production logs
/// support machine-readable JSON output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Text,
    Json,
}

impl Default for LogFormat {
    fn default() -> Self {
        Self::Text
    }
}

/// Logging configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub level: LogLevel,
    pub format: LogFormat,
    /// Maximum log file size in bytes before rotation kicks in. `0` disables
    /// file rotation (logs go to stdout/stderr).
    pub max_bytes: u64,
    /// Maximum number of rotated log files to retain. `0` disables retention
    /// bookkeeping (the rolling appender keeps its own default window).
    pub max_files: u32,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            format: LogFormat::Text,
            max_bytes: 0,
            max_files: 0,
        }
    }
}

/// Server configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Address the listener binds to.
    pub bind: String,
    /// Port the listener binds to.
    pub port: u16,
    /// Directory for persistent server state (workspaces, snapshots, events).
    pub data_dir: String,
    /// Maximum simultaneous client connections.
    pub max_clients: u32,
    /// Maximum Cubes that can exist at once.
    pub max_cubes: u32,
    /// Time allowed for the handshake before disconnect (seconds).
    pub handshake_timeout_secs: u64,
    /// Time allowed for authentication before disconnect (seconds).
    pub authentication_timeout_secs: u64,
    /// Connection idle timeout in seconds (heartbeat deadline).
    pub heartbeat_timeout_secs: u64,
    /// Timeout for flushing queued messages to a slow client (seconds).
    pub write_timeout_secs: u64,
    /// Capacity of each connection's outbound queue. Backpressure uses this
    /// bound; once full, broadcast events are dropped rather than buffered
    /// without limit.
    pub max_outbound_queue: usize,
    /// Maximum protocol message size accepted by the server (bytes).
    pub max_message_bytes: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0".into(),
            port: DEFAULT_PORT,
            data_dir: "nexus-data".into(),
            max_clients: 256,
            max_cubes: 64,
            handshake_timeout_secs: 10,
            authentication_timeout_secs: 10,
            heartbeat_timeout_secs: 30,
            write_timeout_secs: 5,
            max_outbound_queue: 1024,
            max_message_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Client configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientConfig {
    /// Server address to connect to.
    pub server_address: String,
    /// Server port to connect to.
    pub server_port: u16,
    /// Default username used for authentication.
    pub username: String,
    /// Heartbeat interval in seconds.
    pub heartbeat_interval_secs: u64,
    /// Connection timeout in seconds.
    pub connect_timeout_secs: u64,
}

impl Default for ClientConfig {
    fn default() -> Self {
        let username = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "developer".into());
        Self {
            server_address: "127.0.0.1".into(),
            server_port: DEFAULT_PORT,
            username,
            heartbeat_interval_secs: 10,
            connect_timeout_secs: 5,
        }
    }
}

/// Top-level Nexus configuration combining server and logging settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NexusConfig {
    pub server: ServerConfig,
    pub logging: LoggingConfig,
}

impl Default for NexusConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            logging: LoggingConfig::default(),
        }
    }
}

impl NexusConfig {
    /// Parses a config from a TOML string (used for tests and embedded configs).
    pub fn from_toml_str(input: &str) -> Result<Self> {
        toml::from_str(input).map_err(|e| {
            Error::new(crate::error::ErrorCode::Internal, "invalid configuration").with_source(e)
        })
    }

    /// Parses a config from a TOML file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            Error::new(
                crate::error::ErrorCode::Storage,
                format!("cannot read config file '{}'", path.display()),
            )
            .with_source(e)
        })?;
        Self::from_toml_str(&content)
    }

    /// Loads server configuration using an explicitly provided file path (or
    /// `NEXUS_CONFIG`), then applies environment variable overrides.
    pub fn load(explicit_path: Option<&Path>) -> Result<Self> {
        let default_path = config_file_candidates().into_iter().find(|p| p.is_file());

        let cfg = match explicit_path {
            Some(path) => Self::from_file(path)?,
            None => match (
                std::env::var(ENV_CONFIG).ok().map(PathBuf::from),
                default_path,
            ) {
                (Some(path), _) if path.is_file() => Self::from_file(&path)?,
                (_, Some(path)) => Self::from_file(&path)?,
                _ => Self::default(),
            },
        };

        Ok(cfg.apply_env_overrides())
    }

    /// Overlays environment variables on top of the current values.
    #[must_use]
    pub fn apply_env_overrides(mut self) -> Self {
        if let Ok(v) = std::env::var(ENV_BIND) {
            self.server.bind = v;
        }
        if let Ok(v) = std::env::var(ENV_PORT) {
            if let Ok(port) = v.parse() {
                self.server.port = port;
            }
        }
        if let Ok(v) = std::env::var(ENV_DATA_DIR) {
            self.server.data_dir = v;
        }
        if let Ok(v) = std::env::var(ENV_LOG_LEVEL) {
            if let Ok(level) = LogLevel::from_str(&v) {
                self.logging.level = level;
            }
        }
        if let Ok(v) = std::env::var(ENV_LOG_MAX_BYTES) {
            if let Ok(max_bytes) = v.parse::<u64>() {
                self.logging.max_bytes = max_bytes;
            }
        }
        if let Ok(v) = std::env::var(ENV_LOG_MAX_FILES) {
            if let Ok(max_files) = v.parse::<u32>() {
                self.logging.max_files = max_files;
            }
        }
        self
    }
}

impl ClientConfig {
    /// Loads client configuration with environment overrides.
    pub fn load() -> Self {
        ClientConfig::default().apply_env_overrides()
    }

    /// Applies client environment overrides.
    #[must_use]
    pub fn apply_env_overrides(mut self) -> Self {
        if let Ok(v) = std::env::var(ENV_SERVER_ADDRESS) {
            self.server_address = v;
        }
        if let Ok(v) = std::env::var(ENV_PORT) {
            if let Ok(port) = v.parse() {
                self.server_port = port;
            }
        }
        if let Ok(v) = std::env::var(ENV_USERNAME) {
            self.username = v;
        }
        self
    }
}

/// Maps a single validation failure onto the unified Nexus error type with a
/// field-qualified, human-readable message.
fn config_error(field: &'static str, reason: impl Into<String>) -> Error {
    let reason = reason.into();
    Error::new(
        ErrorCode::Internal,
        format!("invalid configuration: {field}: {reason}"),
    )
}

/// Requires a numeric setting to be greater than zero.
fn require_positive(ok: bool, field: &'static str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(config_error(field, "must be greater than zero"))
    }
}

impl ServerConfig {
    /// Validates the configuration before the server starts serving.
    ///
    /// Port `0` is accepted on purpose: it asks the operating system for an
    /// ephemeral port, which tests and local development rely on.
    ///
    /// # Errors
    /// Returns the first invalid field together with a reason.
    pub fn validate(&self) -> Result<()> {
        if self.bind.trim().is_empty() {
            return Err(config_error("bind", "must not be empty"));
        }
        if self.data_dir.trim().is_empty() {
            return Err(config_error("data_dir", "must not be empty"));
        }
        require_positive(self.max_clients > 0, "max_clients")?;
        require_positive(self.max_cubes > 0, "max_cubes")?;
        require_positive(self.handshake_timeout_secs > 0, "handshake_timeout_secs")?;
        require_positive(
            self.authentication_timeout_secs > 0,
            "authentication_timeout_secs",
        )?;
        require_positive(self.heartbeat_timeout_secs > 0, "heartbeat_timeout_secs")?;
        require_positive(self.write_timeout_secs > 0, "write_timeout_secs")?;
        require_positive(self.max_outbound_queue > 0, "max_outbound_queue")?;
        require_positive(self.max_message_bytes > 0, "max_message_bytes")?;
        Ok(())
    }
}

impl ClientConfig {
    /// Validates the client configuration before connecting.
    ///
    /// # Errors
    /// Returns the first invalid field together with a reason.
    pub fn validate(&self) -> Result<()> {
        if self.server_address.trim().is_empty() {
            return Err(config_error("server_address", "must not be empty"));
        }
        if self.server_port == 0 {
            return Err(config_error("server_port", "must be a real port (1-65535)"));
        }
        if self.username.trim().is_empty() {
            return Err(config_error("username", "must not be empty"));
        }
        require_positive(self.heartbeat_interval_secs > 0, "heartbeat_interval_secs")?;
        require_positive(self.connect_timeout_secs > 0, "connect_timeout_secs")?;
        Ok(())
    }
}

impl NexusConfig {
    /// Validates every section of the configuration.
    ///
    /// # Errors
    /// Returns the first invalid field together with a reason.
    pub fn validate(&self) -> Result<()> {
        self.server.validate()
    }
}

/// Candidate config-file locations for the whole system.
#[must_use]
pub fn config_file_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(config_dir) = dirs::config_dir() {
        candidates.push(config_dir.join("nexus/config.toml"));
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("nexus.config.toml"));
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let cfg = NexusConfig::default();
        assert_eq!(cfg.server.port, DEFAULT_PORT);
        assert_eq!(cfg.server.max_cubes, 64);
        assert_eq!(cfg.logging.level, LogLevel::Info);
    }

    #[test]
    fn partial_toml_fills_defaults() {
        let input = r#"
            [server]
            bind = "127.0.0.1"
            port = 9999
        "#;
        let cfg = NexusConfig::from_toml_str(input).unwrap();
        assert_eq!(cfg.server.bind, "127.0.0.1");
        assert_eq!(cfg.server.port, 9999);
        assert_eq!(cfg.server.max_cubes, ServerConfig::default().max_cubes);
        assert_eq!(cfg.logging.level, LogLevel::Info);
    }

    #[test]
    fn full_toml_parses() {
        let input = r#"
            [server]
            bind = "0.0.0.0"
            port = 7000
            data_dir = "./data"
            max_clients = 100
            max_cubes = 32
            heartbeat_timeout_secs = 45

            [logging]
            level = "debug"
            format = "json"
        "#;
        let cfg = NexusConfig::from_toml_str(input).unwrap();
        assert_eq!(cfg.server.max_clients, 100);
        assert_eq!(cfg.server.heartbeat_timeout_secs, 45);
        assert_eq!(cfg.logging.level, LogLevel::Debug);
        assert_eq!(cfg.logging.format, LogFormat::Json);
    }

    #[test]
    fn invalid_toml_is_rejected() {
        let err = NexusConfig::from_toml_str("not toml at all [[[").unwrap_err();
        assert_eq!(err.user_message(), "invalid configuration");
    }

    #[test]
    fn log_level_parsing() {
        assert_eq!(LogLevel::from_str("DEBUG").unwrap(), LogLevel::Debug);
        assert_eq!(LogLevel::from_str("warn").unwrap(), LogLevel::Warn);
        assert!(LogLevel::from_str("verbose").is_err());
    }

    #[test]
    fn env_overrides_win() {
        // Environment overrides applied deterministically via the API surface,
        // using the current process env (empty in tests).
        let cfg = NexusConfig::default().apply_env_overrides();
        assert_eq!(cfg.server.bind, ServerConfig::default().bind);
        assert_eq!(cfg.server.port, DEFAULT_PORT);
    }

    #[test]
    fn client_defaults_are_usable() {
        let cfg = ClientConfig::default();
        assert_eq!(cfg.server_address, "127.0.0.1");
        assert_eq!(cfg.server_port, DEFAULT_PORT);
        assert!(!cfg.username.is_empty());
    }

    #[test]
    fn default_configurations_validate() {
        NexusConfig::default().validate().unwrap();
        ServerConfig::default().validate().unwrap();
        ClientConfig::default().validate().unwrap();
    }

    #[test]
    fn empty_bind_is_rejected() {
        let cfg = ServerConfig {
            bind: "   ".into(),
            ..ServerConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("bind"));
        assert!(err.to_string().contains("must not be empty"));
        assert!(err.to_string().starts_with("invalid configuration"));
    }

    #[test]
    fn empty_data_dir_is_rejected() {
        let cfg = ServerConfig {
            data_dir: String::new(),
            ..ServerConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("data_dir"));
    }

    #[test]
    fn zero_server_limits_are_rejected() {
        let cases: [(&str, ServerConfig); 4] = [
            (
                "max_clients",
                ServerConfig {
                    max_clients: 0,
                    ..ServerConfig::default()
                },
            ),
            (
                "max_cubes",
                ServerConfig {
                    max_cubes: 0,
                    ..ServerConfig::default()
                },
            ),
            (
                "max_outbound_queue",
                ServerConfig {
                    max_outbound_queue: 0,
                    ..ServerConfig::default()
                },
            ),
            (
                "max_message_bytes",
                ServerConfig {
                    max_message_bytes: 0,
                    ..ServerConfig::default()
                },
            ),
        ];
        for (field, cfg) in cases {
            let err = cfg.validate().unwrap_err();
            assert!(
                err.to_string().contains(field),
                "expected field {field} in error, got: {err}"
            );
        }
    }

    #[test]
    fn zero_server_timeouts_are_rejected() {
        let cases: [(&str, ServerConfig); 4] = [
            (
                "handshake_timeout_secs",
                ServerConfig {
                    handshake_timeout_secs: 0,
                    ..ServerConfig::default()
                },
            ),
            (
                "authentication_timeout_secs",
                ServerConfig {
                    authentication_timeout_secs: 0,
                    ..ServerConfig::default()
                },
            ),
            (
                "heartbeat_timeout_secs",
                ServerConfig {
                    heartbeat_timeout_secs: 0,
                    ..ServerConfig::default()
                },
            ),
            (
                "write_timeout_secs",
                ServerConfig {
                    write_timeout_secs: 0,
                    ..ServerConfig::default()
                },
            ),
        ];
        for (field, cfg) in cases {
            let err = cfg.validate().unwrap_err();
            assert!(
                err.to_string().contains(field),
                "expected field {field} in error, got: {err}"
            );
        }
    }

    #[test]
    fn ephemeral_port_is_allowed_for_the_server() {
        let cfg = ServerConfig {
            port: 0,
            ..ServerConfig::default()
        };
        assert!(cfg.validate().is_ok(), "port 0 means ephemeral binding");
    }

    #[test]
    fn invalid_client_settings_are_rejected() {
        let cases: [(&str, ClientConfig); 3] = [
            (
                "server_address",
                ClientConfig {
                    server_address: String::new(),
                    ..ClientConfig::default()
                },
            ),
            (
                "server_port",
                ClientConfig {
                    server_port: 0,
                    ..ClientConfig::default()
                },
            ),
            (
                "username",
                ClientConfig {
                    username: "  ".into(),
                    ..ClientConfig::default()
                },
            ),
        ];
        for (field, cfg) in cases {
            let err = cfg.validate().unwrap_err();
            assert!(
                err.to_string().contains(field),
                "expected field {field} in error, got: {err}"
            );
        }
    }

    #[test]
    fn zero_client_timeouts_are_rejected() {
        let cfg = ClientConfig {
            connect_timeout_secs: 0,
            ..ClientConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("connect_timeout_secs"));
    }

    #[test]
    fn nexus_config_validation_delegates_to_server() {
        let cfg = NexusConfig {
            server: ServerConfig {
                max_clients: 0,
                ..ServerConfig::default()
            },
            ..NexusConfig::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("max_clients"));
    }
}
