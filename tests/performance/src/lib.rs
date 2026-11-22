//! Performance smoke tests.
//!
//! Part 1 ships a lightweight throughput sanity check and a coarse latency
//! bound. These are deliberately loose (CI machines vary wildly); strict SLOs
//! are defined in the Part 13 performance spec, and this crate grows dedicated
//! harnesses there.

#![cfg_attr(not(test), allow(dead_code, unused_imports))]

use std::sync::Arc;
use std::time::{Duration, Instant};

use nexus_core::config::ServerConfig;
use nexus_core::timestamp::Timestamp;
use nexus_protocol::{FrameCodec, Message};
use nexus_server::Server;
use tokio_util::codec::Encoder;

fn server_config() -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1".into(),
        port: 0,
        ..ServerConfig::default()
    }
}

fn client_config(addr: std::net::SocketAddr) -> nexus_core::config::ClientConfig {
    nexus_core::config::ClientConfig {
        server_address: addr.ip().to_string(),
        server_port: addr.port(),
        username: "perf".into(),
        ..nexus_core::config::ClientConfig::default()
    }
}

/// Encodes `heartbeats` messages back-to-back and measures encode+length-scan
/// throughput in messages/second. A pure CPU benchmark with no I/O.
#[test]
fn frame_encoding_throughput_is_reasonable() {
    let heartbeat = Message::Heartbeat {
        at: Timestamp::now(),
    };
    const N: usize = 50_000;

    let mut codec = FrameCodec::default();
    let mut dst = bytes::BytesMut::with_capacity(64 * N);
    let start = Instant::now();
    for _ in 0..N {
        codec.encode(heartbeat.clone(), &mut dst).unwrap();
    }
    let elapsed = start.elapsed();
    let per_second = N as f64 / elapsed.as_secs_f64();
    assert!(per_second > 100_000.0, "encode too slow: {per_second:.0}/s");
}

/// A full server round trip (connect, authenticate, create, close) must fit in
/// a generous budget. Guards against pathological regressions; CI noise is
/// absorbed by the loose bound.
#[tokio::test]
async fn full_round_trip_stays_within_budget() {
    let running = Server::new(server_config()).start().await.unwrap();
    let config = client_config(running.local_addr);

    let start = Instant::now();
    let client = nexus_client::Client::connect(&config).await.unwrap();
    client.authenticate("perf").await.unwrap();
    let _cube = client.create_cube("roundtrip").await.unwrap();
    client.close().await.unwrap();

    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "round trip too slow: {elapsed:?}"
    );
}

/// A batch of heartbeats must complete within a bound.
#[tokio::test]
async fn heartbeat_burst_within_budget() {
    let running = Server::new(server_config()).start().await.unwrap();
    let config = client_config(running.local_addr);

    let client = nexus_client::Client::connect(&config).await.unwrap();
    client.authenticate("perf").await.unwrap();

    let start = Instant::now();
    for _ in 0..25 {
        client.send_heartbeat().await.unwrap();
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "heartbeat burst too slow: {elapsed:?}"
    );
    client.close().await.unwrap();
}

/// Part 4 event engine throughput: sequencing + persistence + fan-out of
/// durable events must stay at or above the Part 1 baseline (frame encoding
/// clears 100k/s in debug builds). The engine's per-Cube gate is deliberately
/// contended here by using a fixed set of cubes.
#[test]
fn event_publish_throughput_matches_part1_baseline() {
    use nexus_core::bench::{is_regressed, BenchmarkReport};
    use nexus_core::event::EventPayload;
    use nexus_core::ids::UserId;
    use nexus_events::bus::EventBus;
    use nexus_events::metrics::EventMetrics;
    use nexus_events::store::MemoryEventStore;

    const CUBES: usize = 4;
    const N: usize = 20_000;

    let bus = EventBus::new(
        Arc::new(MemoryEventStore::new()),
        None,
        Arc::new(EventMetrics::new()),
        Arc::new(nexus_core::PerformanceMetrics::new()),
    );
    let cubes: Vec<_> = (0..CUBES).map(|_| nexus_core::ids::CubeId::new()).collect();

    let start = Instant::now();
    for i in 0..N {
        let cube = cubes[i % CUBES];
        let event = nexus_core::event::Event::new(
            cube,
            UserId::new(),
            EventPayload::PermissionChanged {
                user_id: UserId::new(),
            },
        );
        let receipt = bus.publish(event).unwrap();
        assert!(receipt.persisted);
    }
    let elapsed = start.elapsed();
    let per_second = N as f64 / elapsed.as_secs_f64();
    let current = BenchmarkReport {
        name: String::from("nexus.event_publish_throughput"),
        iterations: N as u64,
        total_duration_ms: elapsed.as_millis() as u64,
        ops_per_second: per_second,
    };
    let baseline = BenchmarkReport {
        name: String::from("nexus.event_publish_throughput"),
        iterations: N as u64,
        total_duration_ms: 200,
        ops_per_second: 100_000.0,
    };
    assert!(
        !is_regressed(&baseline, &current, 0.0),
        "event publish regressed: baseline 100k ops/s, current {per_second:.0}/s ({elapsed:?} for {N})"
    );
}
