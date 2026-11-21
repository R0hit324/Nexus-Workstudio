use tracing_subscriber::EnvFilter;

use crate::config::{LogFormat, LoggingConfig};

/// Initializes the process-wide tracing subscriber from the given config.
///
/// Respects a pre-existing `RUST_LOG` environment variable when present.
/// `try_init` is used so that calling this function multiple times (e.g. from
/// tests) never panics.
pub fn init(config: &LoggingConfig) {
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

/// Initializes logging with built-in defaults (info, text format).
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
        };
        let encoded = serde_json::to_string(&config).unwrap();
        let decoded: LoggingConfig = serde_json::from_str(&encoded).unwrap();
        assert_eq!(config, decoded);
        assert_eq!(decoded.level.as_filter_str(), "warn");
    }
}
