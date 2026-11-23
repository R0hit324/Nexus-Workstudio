use anyhow::Context;
use clap::{Parser, Subcommand};
use nexus_client::Client;
use nexus_core::config::ClientConfig;

/// Nexus client: connect to a server and exercise its operational surface.
#[derive(Debug, Parser)]
#[command(name = "nexus-client", about = "Nexus collaboration client")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Connect, authenticate, create a Cube, send a heartbeat, then exit.
    Create {
        /// Cube name to create.
        name: String,
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        port: Option<u16>,
        #[arg(long, default_value = "developer")]
        user: String,
    },
    /// Connect, authenticate, list Cubes, then exit.
    List {
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        port: Option<u16>,
        #[arg(long, default_value = "developer")]
        user: String,
    },
}

async fn connect(config: &ClientConfig) -> anyhow::Result<Client> {
    let client = Client::connect(config).await.context("failed to connect")?;
    let handshake = client.handshake();
    tracing::info!(
        server = %handshake.server_name,
        version = %handshake.server_version,
        "connected"
    );
    let session = client
        .authenticate(&config.username)
        .await
        .context("failed to authenticate")?;
    tracing::info!(session_id = %session.id, user = %session.username, "authenticated");
    Ok(client)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let (server, port, user) = match &cli.command {
        Command::Create {
            server, port, user, ..
        } => (server, port, user),
        Command::List {
            server, port, user, ..
        } => (server, port, user),
    };

    let mut config = ClientConfig::load();
    if let Some(server) = server {
        config.server_address = server.clone();
    }
    if let Some(port) = port {
        config.server_port = *port;
    }
    config.username = user.clone();

    let client = connect(&config).await?;

    match cli.command {
        Command::Create { name, .. } => {
            let cube = client
                .create_cube(&name)
                .await
                .context("failed to create Cube")?;
            println!("created Cube {} ({})", cube.name, cube.id);
            client.send_heartbeat().await.context("heartbeat failed")?;
            println!("heartbeat acknowledged");
        }
        Command::List { .. } => {
            let cubes = client.list_cubes().await.context("failed to list Cubes")?;
            println!("{} Cube(s):", cubes.len());
            for cube in cubes {
                println!(
                    "  {}  {}  {:?}  ({} members)",
                    cube.id,
                    cube.name,
                    cube.status,
                    cube.members.len()
                );
            }
        }
    }

    client.close().await.context("failed to close cleanly")?;
    Ok(())
}
