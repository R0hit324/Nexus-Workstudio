use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::writer::MakeWriter;
use tracing_subscriber::EnvFilter;

use crate::config::{LogFormat, LoggingConfig};

/// A writer that appends to a log file and rotates it once it exceeds a size
/// budget, retaining at most a fixed number of rotated files.
#[derive(Clone)]
pub struct RotatingFileWriter {
    inner: Arc<Mutex<RotatingFileInner>>,
    dir: PathBuf,
    max_files: u32,
}

#[derive(Debug)]
struct RotatingFileInner {
    file: std::fs::File,
    bytes: u64,
    max_bytes: u64,
    sequence: u32,
}

fn log_file_name(sequence: u32) -> String {
    if sequence == 0 {
        "nexus.log".to_string()
    } else {
        format!("nexus-{sequence}.log")
    }
}

impl RotatingFileWriter {
    /// Creates a rotating file writer in `dir` with the given size budget and
    /// retention window.
    ///
    /// The active file is `nexus.log`; each rollover produces `nexus-<n>.log`
    /// and files beyond the retention window are deleted.
    ///
    /// # Errors
    /// Returns an error when the log directory or file cannot be opened.
    pub fn new(dir: impl AsRef<Path>, max_bytes: u64, max_files: u32) -> io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open(dir.as_ref().join(log_file_name(0)))?;
        Ok(Self {
            inner: Arc::new(Mutex::new(RotatingFileInner {
                file,
                bytes: 0,
                max_bytes,
                sequence: 0,
            })),
            dir: dir.as_ref().to_path_buf(),
            max_files,
        })
    }

    fn path_for(&self, sequence: u32) -> PathBuf {
        self.dir.join(log_file_name(sequence))
    }

    fn rotate(&self, guard: &mut RotatingFileInner) -> io::Result<()> {
        let sequence = guard.sequence + 1;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(true)
            .open(self.path_for(sequence))?;
        *guard = RotatingFileInner {
            file,
            bytes: 0,
            max_bytes: guard.max_bytes,
            sequence,
        };
        if self.max_files > 0 && sequence > self.max_files {
            let _ = std::fs::remove_file(self.path_for(sequence - self.max_files));
        }
        Ok(())
    }
}

impl Write for RotatingFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "log writer lock poisoned"))?;
        if guard.bytes > 0 && guard.bytes + buf.len() as u64 > guard.max_bytes {
            self.rotate(&mut *guard)?;
        }
        let written = guard.file.write(buf)?;
        guard.bytes += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "log writer lock poisoned"))?;
        guard.file.flush()
    }
}

impl<'a> MakeWriter<'a> for RotatingFileWriter {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

/// Initializes the process-wide tracing subscriber from the given config.
///
/// Respects a pre-existing `RUST_LOG` environment variable when present.
/// `try_init` is used so that calling this function multiple times (e.g. from
/// tests) never panics. When `max_bytes`/`max_files` are set the subscriber
/// writes through a rotating file appender, otherwise it writes to the
/// standard streams.
pub fn init(config: &LoggingConfig) {
    if config.max_bytes > 0 && config.max_files > 0 {
        let log_dir = std::env::temp_dir().join("nexus-logs");
        let _ = init_rotating(config, &log_dir);
        return;
    }

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(config.level.as_filter_str()));

    let builder = tracing_subscriber::fmt().with_env_filter(filter);

    match config.format {
        LogFormat::Json => {
            let _ = builder.json().try_init();
        }
        LogFormat::Text => {
            let _ = builder.try_init();
        }
    }
}

/// Creates a rotating file writer and installs it as the tracing writer.
///
/// # Errors
/// Returns an error when the log directory or file cannot be opened.
pub fn init_rotating(
    config: &LoggingConfig,
    log_dir: &Path,
) -> std::io::Result<RotatingFileWriter> {
    let writer = RotatingFileWriter::new(log_dir, config.max_bytes, config.max_files)?;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(config.level.as_filter_str()));
    let builder = tracing_subscriber::fmt()
        .with_writer(writer)
        .with_env_filter(filter);
    match config.format {
        LogFormat::Json => {
            let _ = builder.json().try_init();
        }
        LogFormat::Text => {
            let _ = builder.try_init();
        }
    }
    Ok(RotatingFileWriter::new(
        log_dir,
        config.max_bytes,
        config.max_files,
    )?)
}

/// Initializes logging with built-in defaults (info, text format, no rotation).
pub fn init_default() {
    init(&LoggingConfig::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initializing_logging_does_not_panic() {
        // This can be called repeatedly even though the subscriber is global.
        init(&LoggingConfig::default());
        init(&LoggingConfig {
            format: LogFormat::Json,
            ..LoggingConfig::default()
        });
    }

    #[test]
    fn json_config_roundtrips() {
        let config = LoggingConfig {
            format: LogFormat::Json,
            level: crate::config::LogLevel::Warn,
            ..LoggingConfig::default()
        };
        let encoded = serde_json::to_string(&config).unwrap();
        let decoded: LoggingConfig = serde_json::from_str(&encoded).unwrap();
        assert_eq!(config, decoded);
        assert_eq!(decoded.level.as_filter_str(), "warn");
    }

    #[test]
    fn rotation_is_disabled_by_default() {
        let config = LoggingConfig::default();
        assert_eq!(config.max_bytes, 0);
        assert_eq!(config.max_files, 0);
    }

    #[test]
    fn rotation_policy_roundtrips_through_serde() {
        let config = LoggingConfig {
            max_bytes: 10 * 1024 * 1024,
            max_files: 7,
            ..LoggingConfig::default()
        };
        let encoded = serde_json::to_string(&config).unwrap();
        let decoded: LoggingConfig = serde_json::from_str(&encoded).unwrap();
        assert_eq!(config, decoded);
        assert_eq!(decoded.max_bytes, 10 * 1024 * 1024);
        assert_eq!(decoded.max_files, 7);
    }

    #[test]
    fn rotating_writer_writes_and_rotates() {
        let dir = std::env::temp_dir().join(format!("nexus-log-{}", uuid::Uuid::new_v4()));
        let mut writer = RotatingFileWriter::new(&dir, 8, 3).expect("rotating writer");
        writer.write_all(b"12345678").expect("write");
        writer.write_all(b"abcd").expect("write after rotation");
        writer.flush().expect("flush");

        let active = std::fs::read(dir.join("nexus.log")).expect("read active log");
        assert_eq!(&active[..], b"12345678");
        let rotated = std::fs::read(dir.join("nexus-1.log")).expect("read rotated log");
        assert_eq!(&rotated[..], b"abcd");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotating_writer_under_budget_does_not_rotate() {
        let dir = std::env::temp_dir().join(format!("nexus-log-{}", uuid::Uuid::new_v4()));
        let mut writer = RotatingFileWriter::new(&dir, 1024, 3).expect("rotating writer");
        writer.write_all(b"hello").expect("write");
        writer.flush().expect("flush");
        assert!(dir.join("nexus.log").exists());
        assert!(!dir.join("nexus-1.log").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotating_init_accepts_policy_and_dir() {
        let dir = std::env::temp_dir().join(format!("nexus-log-{}", uuid::Uuid::new_v4()));
        let config = LoggingConfig {
            max_bytes: 1024,
            max_files: 2,
            ..LoggingConfig::default()
        };
        let result = init_rotating(&config, &dir);
        assert!(result.is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
