//! Security boundary tests (Part 14, spec sections 79 and 95).
//!
//! These tests exercise the authentication, authorization, and protocol abuse
//! surfaces of a live server + client pair:
//!
//! - **Authentication**: the first message after the handshake must be
//!   `Authenticate`; empty and unsupported-character usernames never yield a
//!   session; requests that skip authentication are protocol violations.
//! - **Authorization**: workspace operations are denied for members of other
//!   Cubes, path traversal cannot escape the workspace root, and a joined
//!   member can still read shared data.
//! - **Protocol abuse**: malformed and oversized frames tear the connection
//!   down without harming the server, and protocol version mismatches fail
//!   closed.

#![cfg_attr(not(test), allow(dead_code, unused_imports))]

use std::time::Duration;

use nexus_core::config::{ClientConfig, ServerConfig};
use nexus_core::ids::RequestId;
use nexus_core::version::Version;
use nexus_protocol::message::{ErrorCode as ProtoError, Message};
use nexus_protocol::transport::{TcpTransport, Transport};
use nexus_server::Server;

use nexus_tests_support::TempEnv;

fn server_config(env: &TempEnv) -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1".into(),
        port: 0,
        data_dir: env.join("data").to_string_lossy().into_owned(),
        ..ServerConfig::default()
    }
}

async fn spawn_server(env: &TempEnv) -> (nexus_server::RunningServer, std::net::SocketAddr) {
    let running = Server::new(server_config(env)).start().await.unwrap();
    let addr = running.local_addr;
    (running, addr)
}

fn client_config(addr: std::net::SocketAddr, username: &str) -> ClientConfig {
    ClientConfig {
        server_address: addr.ip().to_string(),
        server_port: addr.port(),
        username: username.into(),
        ..ClientConfig::default()
    }
}

// ---------------------------------------------------------------------------
// Raw wire helpers (needed where error codes must be asserted).
// ---------------------------------------------------------------------------

/// Receives the next reply correlated to `want`, skipping broadcast traffic.
async fn next_reply(transport: &mut TcpTransport, want: RequestId) -> Message {
    for _ in 0..64 {
        let got = tokio::time::timeout(Duration::from_secs(5), transport.receive())
            .await
            .expect("timed out waiting for reply")
            .expect("transport error")
            .expect("connection closed before reply");
        if got.response_request_id() == Some(want) {
            return got;
        }
    }
    panic!("timed out waiting for reply to {want:?}");
}

/// Receives the first reply of a fresh connection (handshake phase).
async fn expect_first_reply(transport: &mut TcpTransport) -> Message {
    let got = tokio::time::timeout(Duration::from_secs(5), transport.receive())
        .await
        .expect("timed out waiting for reply")
        .expect("transport error")
        .expect("connection closed before reply");
    got
}

async fn handshake(addr: std::net::SocketAddr) -> TcpTransport {
    let mut transport = TcpTransport::connect(addr).await.unwrap();
    transport
        .send(&Message::client_hello(
            "security-probe",
            nexus_client::client_capabilities(),
        ))
        .await
        .unwrap();
    let reply = expect_first_reply(&mut transport).await;
    match reply {
        Message::ServerHello { .. } => {}
        other => panic!("unexpected handshake reply: {other:?}"),
    }
    transport
}

async fn authenticate(addr: std::net::SocketAddr, username: &str) -> TcpTransport {
    let mut transport = handshake(addr).await;
    let request_id = RequestId::new();
    transport
        .send(&Message::Authenticate {
            request_id,
            username: username.into(),
        })
        .await
        .unwrap();
    let reply = next_reply(&mut transport, request_id).await;
    assert!(
        matches!(reply, Message::AuthenticationResult { .. }),
        "authentication failed: {reply:?}"
    );
    transport
}

// ---------------------------------------------------------------------------
// Authentication boundaries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn message_must_follow_the_handshake() {
    let env = TempEnv::new("sec-handshake-order");
    let (_running, addr) = spawn_server(&env).await;

    // Sending `Authenticate` before any `ClientHello` is a protocol violation.
    let mut transport = TcpTransport::connect(addr).await.unwrap();
    let request_id = RequestId::new();
    transport
        .send(&Message::Authenticate {
            request_id,
            username: "alice".into(),
        })
        .await
        .unwrap();
    let reply = next_reply(&mut transport, request_id).await;
    let Message::Error { code, .. } = reply else {
        panic!("expected error reply, got {reply:?}");
    };
    assert_eq!(code, ProtoError::ProtocolViolation);
}

#[tokio::test]
async fn workspace_request_before_authenticate_is_rejected() {
    let env = TempEnv::new("sec-auth-before-user");
    let (_running, addr) = spawn_server(&env).await;

    let mut transport = handshake(addr).await;
    let request_id = RequestId::new();
    transport
        .send(&Message::ListCubes { request_id })
        .await
        .unwrap();
    let reply = next_reply(&mut transport, request_id).await;
    let Message::Error { code, .. } = reply else {
        panic!("expected error reply, got {reply:?}");
    };
    assert_eq!(code, ProtoError::ProtocolViolation);
}

#[tokio::test]
async fn empty_username_is_rejected_and_server_stays_healthy() {
    let env = TempEnv::new("sec-empty-user");
    let (_running, addr) = spawn_server(&env).await;

    let client = nexus_client::Client::connect(&client_config(addr, ""))
        .await
        .unwrap();
    let err = client.authenticate("").await;
    assert!(err.is_err(), "empty username must not authenticate");

    // The server must still accept a valid credential on a fresh session.
    let good = nexus_client::Client::connect(&client_config(addr, "alice"))
        .await
        .unwrap();
    let session = good.authenticate("alice").await.unwrap();
    assert_eq!(session.username, "alice");
    good.close().await.unwrap();
}

#[tokio::test]
async fn unsupported_username_characters_are_rejected() {
    let env = TempEnv::new("sec-bad-user");
    let (_running, addr) = spawn_server(&env).await;

    let client = nexus_client::Client::connect(&client_config(addr, "bad user"))
        .await
        .unwrap();
    let err = client.authenticate("bad user").await;
    assert!(
        err.is_err(),
        "username with unsupported characters must be rejected"
    );
    client.close().await.unwrap();
}

// ---------------------------------------------------------------------------
// Authorization boundaries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cross_cube_access_is_denied_for_unjoined_members() {
    let env = TempEnv::new("sec-cross-cube");
    let (_running, addr) = spawn_server(&env).await;

    let mut alice = authenticate(addr, "alice").await;
    let mut bob = authenticate(addr, "bob").await;

    let create_rid = RequestId::new();
    alice
        .send(&Message::CreateCube {
            request_id: create_rid,
            name: "target".into(),
        })
        .await
        .unwrap();
    let Message::CubeCreated { cube, .. } = next_reply(&mut alice, create_rid).await else {
        panic!("expected CubeCreated");
    };

    let write_rid = RequestId::new();
    alice
        .send(&Message::WriteFile {
            request_id: write_rid,
            cube_id: cube.id,
            path: "secret.txt".into(),
            content: b"classified".to_vec(),
        })
        .await
        .unwrap();
    let Message::Ack { .. } = next_reply(&mut alice, write_rid).await else {
        panic!("expected Ack");
    };

    // Bob is authenticated but has not joined Alice's Cube.
    let read_rid = RequestId::new();
    bob.send(&Message::ReadFile {
        request_id: read_rid,
        cube_id: cube.id,
        path: "secret.txt".into(),
    })
    .await
    .unwrap();
    match next_reply(&mut bob, read_rid).await {
        Message::Error { code, .. } => assert_eq!(
            code,
            ProtoError::PermissionDenied,
            "unjoined read must be permission denied"
        ),
        other => panic!("expected Error, got {other:?}"),
    }

    let write_rid = RequestId::new();
    bob.send(&Message::WriteFile {
        request_id: write_rid,
        cube_id: cube.id,
        path: "evil.txt".into(),
        content: b"boom".to_vec(),
    })
    .await
    .unwrap();
    match next_reply(&mut bob, write_rid).await {
        Message::Error { code, .. } => assert_eq!(
            code,
            ProtoError::PermissionDenied,
            "unjoined write must be permission denied"
        ),
        other => panic!("expected Error, got {other:?}"),
    }

    // After joining, Bob may read the shared file.
    let join_rid = RequestId::new();
    bob.send(&Message::JoinCube {
        request_id: join_rid,
        cube_id: cube.id,
    })
    .await
    .unwrap();
    let Message::CubeJoined { .. } = next_reply(&mut bob, join_rid).await else {
        panic!("expected CubeJoined");
    };

    let read_rid = RequestId::new();
    bob.send(&Message::ReadFile {
        request_id: read_rid,
        cube_id: cube.id,
        path: "secret.txt".into(),
    })
    .await
    .unwrap();
    match next_reply(&mut bob, read_rid).await {
        Message::FileContent { content, .. } => assert_eq!(content, b"classified".to_vec()),
        other => panic!("expected FileContent, got {other:?}"),
    }

    alice.close().await.unwrap();
    bob.close().await.unwrap();
}

#[tokio::test]
async fn joined_member_can_read_shared_file() {
    let env = TempEnv::new("sec-join-read");
    let (_running, addr) = spawn_server(&env).await;

    let alice = nexus_client::Client::connect(&client_config(addr, "alice"))
        .await
        .unwrap();
    alice.authenticate("alice").await.unwrap();
    let cube = alice.create_cube("shared").await.unwrap();
    alice
        .write_file(cube.id, "notes.md", b"team notes")
        .await
        .unwrap();

    let bob = nexus_client::Client::connect(&client_config(addr, "bob"))
        .await
        .unwrap();
    bob.authenticate("bob").await.unwrap();
    bob.join_cube(cube.id).await.unwrap();

    let (version, content) = bob.read_file(cube.id, "notes.md").await.unwrap();
    assert_eq!(content, b"team notes".to_vec());
    assert!(version.get() > 0);

    alice.close().await.unwrap();
    bob.close().await.unwrap();
}

#[tokio::test]
async fn path_traversal_is_rejected() {
    let env = TempEnv::new("sec-traverse");
    let (_running, addr) = spawn_server(&env).await;

    let mut alice = authenticate(addr, "alice").await;

    let create_rid = RequestId::new();
    alice
        .send(&Message::CreateCube {
            request_id: create_rid,
            name: "escape".into(),
        })
        .await
        .unwrap();
    let Message::CubeCreated { cube, .. } = next_reply(&mut alice, create_rid).await else {
        panic!("expected CubeCreated");
    };

    let write_rid = RequestId::new();
    alice
        .send(&Message::WriteFile {
            request_id: write_rid,
            cube_id: cube.id,
            path: "../outside.txt".into(),
            content: b"evil".to_vec(),
        })
        .await
        .unwrap();
    let reply = next_reply(&mut alice, write_rid).await;
    let Message::Error { code, .. } = reply else {
        panic!("expected Error, got {reply:?}");
    };
    assert_eq!(
        code,
        ProtoError::InvalidPath,
        "traversal write must be rejected"
    );

    let rename_rid = RequestId::new();
    alice
        .send(&Message::RenameFile {
            request_id: rename_rid,
            cube_id: cube.id,
            from: "../outside.txt".into(),
            to: "../outside2.txt".into(),
        })
        .await
        .unwrap();
    let reply = next_reply(&mut alice, rename_rid).await;
    let Message::Error { code, .. } = reply else {
        panic!("expected Error, got {reply:?}");
    };
    assert_eq!(
        code,
        ProtoError::InvalidPath,
        "traversal rename must be rejected"
    );

    alice.close().await.unwrap();
}

// ---------------------------------------------------------------------------
// Protocol abuse and transport robustness
// ---------------------------------------------------------------------------

/// Sends one raw frame and expects the server to tear the connection down.
async fn send_raw_frame_and_expect_close(addr: std::net::SocketAddr, frame: &[u8]) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(frame).await.unwrap();
    stream.flush().await.unwrap();
    let mut buf = [0u8; 8];
    let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf))
        .await
        .expect("timed out waiting for server close")
        .expect("read failed");
    assert_eq!(n, 0, "server must close after the protocol error");
}

#[tokio::test]
async fn malformed_frame_closes_connection_without_harming_server() {
    let env = TempEnv::new("sec-malformed");
    let (_running, addr) = spawn_server(&env).await;

    // Frame header claims a 4-byte payload; the bytes are not a valid postcard
    // encoding of a `Message`, so the decoder must fail and close the peer.
    let mut frame = vec![0x00, 0x00, 0x00, 4];
    frame.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    send_raw_frame_and_expect_close(addr, &frame).await;

    // The server must still serve well-formed clients afterwards.
    let good = nexus_client::Client::connect(&client_config(addr, "alice"))
        .await
        .unwrap();
    let session = good.authenticate("alice").await.unwrap();
    assert_eq!(session.username, "alice");
    good.close().await.unwrap();
}

#[tokio::test]
async fn oversized_frame_is_rejected_then_server_recovers() {
    let env = TempEnv::new("sec-oversize");
    let (_running, addr) = spawn_server(&env).await;

    // A frame length above `MAX_FRAME_LENGTH` must be refused outright; the
    // decoder fails on the oversized header before any payload is read.
    let oversized = nexus_protocol::MAX_FRAME_LENGTH + 1;
    let frame = u32::to_be_bytes(oversized).to_vec();
    send_raw_frame_and_expect_close(addr, &frame).await;

    // The server must still serve well-formed clients afterwards.
    let good = nexus_client::Client::connect(&client_config(addr, "alice"))
        .await
        .unwrap();
    let session = good.authenticate("alice").await.unwrap();
    assert_eq!(session.username, "alice");
    good.close().await.unwrap();
}

#[tokio::test]
async fn protocol_version_mismatch_is_rejected() {
    let env = TempEnv::new("sec-version");
    let (_running, addr) = spawn_server(&env).await;

    // An ancient client must fail closed with a protocol mismatch.
    let mut transport = TcpTransport::connect(addr).await.unwrap();
    let reply = Message::ClientHello {
        client_name: "ancient-client".into(),
        protocol_version: Version::new(0),
        capabilities: nexus_client::client_capabilities(),
    };
    transport.send(&reply).await.unwrap();
    let reply = expect_first_reply(&mut transport).await;
    let Message::Error { code, .. } = reply else {
        panic!("expected error reply, got {reply:?}");
    };
    assert_eq!(
        code,
        ProtoError::ProtocolMismatch,
        "incompatible client version must be rejected"
    );
}
