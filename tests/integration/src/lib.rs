//! End-to-end integration tests.
//!
//! These start a real [`Server`] on an ephemeral port, connect a real
//! [`Client`], and exercise the complete operational surface that Part 1
//! delivers: handshake, authentication, Cube lifecycle, listing, membership,
//! and heartbeats.

#![cfg_attr(not(test), allow(dead_code, unused_imports))]

use nexus_core::config::ServerConfig;
use nexus_core::ids::UserId;
use nexus_server::Server;

fn server_config() -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1".into(),
        port: 0,
        ..ServerConfig::default()
    }
}

async fn spawn_server() -> (nexus_server::RunningServer, std::net::SocketAddr) {
    let running = Server::new(server_config()).start().await.unwrap();
    let addr = running.local_addr;
    (running, addr)
}

fn client_config(addr: std::net::SocketAddr, username: &str) -> nexus_core::config::ClientConfig {
    nexus_core::config::ClientConfig {
        server_address: addr.ip().to_string(),
        server_port: addr.port(),
        username: username.into(),
        ..nexus_core::config::ClientConfig::default()
    }
}

#[tokio::test]
async fn client_connects_authenticates_and_echoes_version() {
    let (_running, addr) = spawn_server().await;
    let config = client_config(addr, "alice");

    let client = nexus_client::Client::connect(&config).await.unwrap();
    let handshake = client.handshake();
    assert_eq!(handshake.server_name, "nexus-server");
    assert!(handshake
        .server_capabilities
        .contains(&nexus_protocol::Capability::Presence));

    let session = client.authenticate("alice").await.unwrap();
    assert_eq!(session.username, "alice");
    assert_eq!(client.session().await, Some(session.clone()));

    client.close().await.unwrap();
}

#[tokio::test]
async fn create_list_join_leave_over_the_wire() {
    let (_running, addr) = spawn_server().await;
    let config = client_config(addr, "bob");
    let alice_config = client_config(addr, "alice");

    let bob = nexus_client::Client::connect(&config).await.unwrap();
    bob.authenticate("bob").await.unwrap();
    let alice = nexus_client::Client::connect(&alice_config).await.unwrap();
    alice.authenticate("alice").await.unwrap();

    let cube = bob.create_cube("shared-demo").await.unwrap();
    assert_eq!(cube.name, "shared-demo");
    assert_eq!(cube.members.len(), 1);

    // Second user joins.
    let joined = alice.join_cube(cube.id).await.unwrap();
    assert_eq!(joined.members.len(), 2);

    // Both clients see the same list.
    let bob_list = bob.list_cubes().await.unwrap();
    let alice_list = alice.list_cubes().await.unwrap();
    assert_eq!(bob_list.len(), 1);
    assert_eq!(alice_list.len(), 1);
    assert_eq!(bob_list[0].id, cube.id);

    alice.leave_cube(cube.id).await.unwrap();
    let after = bob.list_cubes().await.unwrap();
    assert_eq!(after[0].members.len(), 1);

    // Anonymous authentication fails.
    let anon = nexus_client::Client::connect(&config).await.unwrap();
    let err = anon.authenticate("").await;
    assert!(err.is_err());

    bob.close().await.unwrap();
    alice.close().await.unwrap();
}

#[tokio::test]
async fn plugin_status_reports_builtin_roster_over_the_wire() {
    let (_running, addr) = spawn_server().await;
    let config = client_config(addr, "erika");

    let client = nexus_client::Client::connect(&config).await.unwrap();
    client.authenticate("erika").await.unwrap();

    let records = client.plugin_status().await.unwrap();
    let ids: Vec<&str> = records.iter().map(|record| record.id.as_str()).collect();
    assert!(
        ids.contains(&"org.nexus.git"),
        "missing builtin git: {ids:?}"
    );
    assert!(
        ids.contains(&"org.nexus.rust"),
        "missing builtin rust: {ids:?}"
    );
    let git = records.iter().find(|r| r.id == "org.nexus.git").unwrap();
    assert_eq!(git.name, "Git Integration");
    assert_eq!(git.state, "enabled");
    assert!(git.error.is_none());

    client.close().await.unwrap();
}

#[tokio::test]
async fn heartbeats_are_acknowledged() {
    let (_running, addr) = spawn_server().await;
    let config = client_config(addr, "carol");

    let client = nexus_client::Client::connect(&config).await.unwrap();
    client.authenticate("carol").await.unwrap();

    for _ in 0..3 {
        client.send_heartbeat().await.unwrap();
    }

    client.close().await.unwrap();
}

#[tokio::test]
async fn duplicate_cube_names_and_missing_cubes_are_handled() {
    let (_running, addr) = spawn_server().await;
    let config = client_config(addr, "dana");

    let client = nexus_client::Client::connect(&config).await.unwrap();
    client.authenticate("dana").await.unwrap();

    let first = client.create_cube("unique").await.unwrap();
    let second = client.create_cube("unique").await.unwrap();
    // Names are just labels; ids are unique.
    assert_ne!(first.id, second.id);

    // Leaving a Cube the client never joined raises a domain error.
    let ghost = nexus_core::ids::CubeId::new();
    let err = client.leave_cube(ghost).await;
    assert!(err.is_err());

    client.close().await.unwrap();
}

#[tokio::test]
async fn server_state_tracks_connections() {
    let running = Server::new(server_config()).start().await.unwrap();
    let state = running.state().clone();
    let addr = running.local_addr;
    let config = client_config(addr, "erin");

    let client = nexus_client::Client::connect(&config).await.unwrap();
    client.authenticate("erin").await.unwrap();

    // Let the accept loop settle.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if state
            .active_connections
            .load(std::sync::atomic::Ordering::Relaxed)
            >= 1
        {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("server never counted a connection");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    client.close().await.unwrap();

    // Close is graceful: the server must observe the session drop promptly.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if state
            .active_sessions
            .load(std::sync::atomic::Ordering::Relaxed)
            == 0
        {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("server never dropped the session after disconnect");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    // The connection counter may briefly be 1 while the handler tidies up.
}

#[test]
fn user_ids_are_unique() {
    let a = UserId::new();
    let b = UserId::new();
    assert_ne!(a, b);
}

/// Headless Part 2 scenario: drives `App` — the terminal-free application
/// layer — without a screen, just as the live event loop would, to prove the
/// UI state machine is independent of ratatui/crossterm.
#[test]
fn headless_session_connects_joins_opens_and_edits() {
    use nexus_client::SessionService;
    use nexus_core::ids::{CubeId, FileId, WorkspaceId};
    use nexus_tui::actions::Action;
    use nexus_tui::app::App;
    use nexus_tui::input::{Key, UiEvent};
    let mut app = App::new(SessionService::new("ws://headless"));

    // Connect, then join a cube with a two-file listing.
    app.session.connect("alice");
    app.tick();
    assert!(app.session.state.is_connected());
    assert_eq!(app.session.state.username.as_deref(), Some("alice"));

    let cube = CubeId::from_u64(7);
    let files = vec![
        (FileId::from_u64(1), std::path::PathBuf::from("src/main.rs")),
        (FileId::from_u64(2), std::path::PathBuf::from("README.md")),
    ];
    app.session
        .join_cube(cube, "api", WorkspaceId::from_u64(9), files);
    app.tick();
    assert_eq!(
        app.session.state.active_cube.as_ref().map(|c| c.id),
        Some(cube)
    );
    assert_eq!(app.editor.active_doc().map(|d| d.file_id), None);

    // Finger opens a cached file and the editor shows it.
    app.session.open_file(
        FileId::from_u64(1),
        std::path::PathBuf::from("src/main.rs"),
        "fn main() {".into(),
    );
    app.tick();
    app.apply_action(Action::OpenFile(FileId::from_u64(1)));
    let doc = app.editor.active_doc().expect("a document is open");
    assert_eq!(doc.path, std::path::PathBuf::from("src/main.rs"));
    assert!(app.session.workspace.cached(FileId::from_u64(1)).is_some());

    // Insert mode: typing marks the document (and the workspace) dirty.
    app.handle_event(UiEvent::Key(Key::Char('i')));
    app.handle_event(UiEvent::Key(Key::Char('x')));
    let doc = app.editor.active_doc().expect("document stays open");
    assert!(doc.dirty);
    assert!(app.session.workspace.is_dirty(FileId::from_u64(1)));

    // Presence from another member surfaces via the users panel.
    app.session
        .set_presence(vec![nexus_client::app::state::MemberPresence {
            user_id: nexus_core::ids::UserId::from_u64(9),
            name: "bob".into(),
            status: nexus_client::app::state::MemberStatus::Online,
        }]);
    app.tick();
    assert!(app.users.members().iter().any(|m| m.name == "bob"));
}

/// Network fault: an abruptly dropped client (no graceful close) must not
/// corrupt server state; the server cleans the session up and keeps serving.
#[tokio::test]
async fn abrupt_client_disconnect_is_cleaned_up_without_corrupting_server() {
    let running = Server::new(server_config()).start().await.unwrap();
    let state = running.state().clone();
    let addr = running.local_addr;
    let config = client_config(addr, "frank");

    let client = nexus_client::Client::connect(&config).await.unwrap();
    client.authenticate("frank").await.unwrap();
    let _cube = client.create_cube("doomed").await.unwrap();

    // Abruptly drop the client without a graceful close.
    drop(client);

    // The server must observe the session drop and stay healthy.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if state
            .active_sessions
            .load(std::sync::atomic::Ordering::Relaxed)
            == 0
        {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("server never dropped the session after abrupt disconnect");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    // A fresh client must be able to authenticate and operate normally.
    let fresh = nexus_client::Client::connect(&config).await.unwrap();
    let session = fresh.authenticate("frank").await.unwrap();
    assert_eq!(session.username, "frank");
    let recreated = fresh.create_cube("after-abuse").await.unwrap();
    assert_eq!(recreated.name, "after-abuse");
    fresh.close().await.unwrap();
}
