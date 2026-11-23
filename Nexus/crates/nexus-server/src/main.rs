use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use nexus_core::config::LogFormat;
use nexus_core::logging;
use nexus_server::{FilesystemStorage, Server};

/// Nexus server: authoritative source of truth for collaboration state.
#[derive(Debug, Parser)]
#[command(name = "nexus-server", about = "Nexus collaboration server")]
struct Cli {
    /// Path to a TOML configuration file.
    #[arg(short, long)]
    config: Option<PathBuf>,
    /// Override the bind address (equivalent to NEXUS_BIND).
    #[arg(long)]
    bind: Option<String>,
    /// Override the listen port (equivalent to NEXUS_SERVER_PORT).
    #[arg(long)]
    port: Option<u16>,
    /// Log output format.
    #[arg(long, value_enum, default_value_t = LogFormatArg::Auto)]
    log_format: LogFormatArg,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum LogFormatArg {
    Auto,
    Text,
    Json,
}

impl From<LogFormatArg> for Option<LogFormat> {
    fn from(value: LogFormatArg) -> Self {
        match value {
            LogFormatArg::Auto => None,
            LogFormatArg::Text => Some(LogFormat::Text),
            LogFormatArg::Json => Some(LogFormat::Json),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let mut config = nexus_core::config::NexusConfig::load(cli.config.as_deref())
        .context("failed to load configuration")?;

    if let Some(bind) = cli.bind {
        config.server.bind = bind;
    }
    if let Some(port) = cli.port {
        config.server.port = port;
    }

    let mut logging_config = config.logging.clone();
    if let Some(format) = Option::<LogFormat>::from(cli.log_format) {
        logging_config.format = format;
    }
    logging::init(&logging_config);

    // Production deployments persist workspace data under `data_dir`. The
    // directory must exist before the storage layer canonicalizes it.
    let data_dir = std::path::Path::new(&config.server.data_dir).to_path_buf();
    tokio::fs::create_dir_all(&data_dir)
        .await
        .with_context(|| format!("cannot create data dir '{}'", data_dir.display()))?;
    let storage: Arc<_> = Arc::new(FilesystemStorage::new(data_dir.clone()));

    let server =
        Server::with_storage(config.server, storage).with_event_log(data_dir.join("events"));
    let mut running = server.start().await.context("failed to start server")?;

    tracing::info!("ready at {}", running.local_addr);

    // Ctrl+C / SIGTERM -> graceful shutdown.
    let shutdown = Arc::new(nexus_server::Shutdown::new());
    nexus_server::spawn_signal_task(shutdown.clone());
    shutdown.delay().await;
    running.request_shutdown();
    running.wait().await;
    tracing::info!("server stopped");
    Ok(())
}
