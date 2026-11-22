//! Concurrency tests for the Part 4 event engine.
//!
//! The engine's core promise: sequences are assigned exactly-once and gap-free
//! under parallel publishers. These tests hammer the bus with real OS threads
//! and verify the persisted log satisfies the invariant.

use std::sync::Arc;

use nexus_core::event::{DeliveryClass, Event, EventPayload, EventSequence};
use nexus_core::ids::{CubeId, UserId};
use nexus_events::bus::EventBus;
use nexus_events::metrics::EventMetrics;

fn durable(cube: CubeId) -> Event {
    Event::new(
        cube,
        UserId::new(),
        EventPayload::PermissionChanged {
            user_id: UserId::new(),
        },
    )
}

fn assert_gap_free(events: &[Event]) {
    for (i, event) in events.iter().enumerate() {
        assert_eq!(
            event.sequence().value() as usize,
            i + 1,
            "sequence {} expected at index {}, got {:?}",
            i + 1,
            i,
            event.sequence()
        );
    }
}

/// Distinct cubes publishing in parallel must each see a monotonic, gap-free,
/// exactly-once sequence. The per-cube gates must not leak order across cubes.
#[test]
fn parallel_cubes_stay_independent_and_contiguous() {
    const CUBES: usize = 8;
    const PER_CUBE: usize = 250;

    let bus = Arc::new(EventBus::new(
        Arc::new(nexus_events::store::MemoryEventStore::new()),
        None,
        Arc::new(EventMetrics::new()),
        Arc::new(nexus_core::PerformanceMetrics::new()),
    ));
    let cubes: Vec<CubeId> = (0..CUBES).map(|_| CubeId::new()).collect();

    let handles: Vec<_> = cubes
        .iter()
        .map(|cube| {
            let bus = bus.clone();
            let cube = *cube;
            std::thread::spawn(move || {
                for _ in 0..PER_CUBE {
                    let receipt = bus.publish(durable(cube)).unwrap();
                    assert_eq!(receipt.delivered_to, 0);
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }

    for cube in &cubes {
        let events = bus.replay(*cube, EventSequence::UNASSIGNED).unwrap();
        assert_eq!(events.len(), PER_CUBE, "cube {cube} lost events");
        assert_gap_free(&events);
        assert_eq!(bus.latest_sequence(*cube).value(), PER_CUBE as u64);
    }
}

/// Publishers fighting over a single cube must be serialized: total order is
/// contiguous 1..=N with no duplicates and no gaps.
#[test]
fn single_cube_parallel_publishers_are_serialized() {
    const THREADS: usize = 8;
    const PER_THREAD: usize = 200;

    let bus = Arc::new(EventBus::new(
        Arc::new(nexus_events::store::MemoryEventStore::new()),
        None,
        Arc::new(EventMetrics::new()),
        Arc::new(nexus_core::PerformanceMetrics::new()),
    ));
    let cube = CubeId::new();

    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let bus = bus.clone();
            std::thread::spawn(move || {
                for _ in 0..PER_THREAD {
                    let receipt = bus.publish(durable(cube)).unwrap();
                    assert!(receipt.persisted);
                    assert!(receipt.event.is_sequenced());
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }

    let expected = (THREADS * PER_THREAD) as u64;
    assert_eq!(bus.latest_sequence(cube).value(), expected);

    let events = bus.replay(cube, EventSequence::UNASSIGNED).unwrap();
    assert_eq!(events.len() as u64, expected);
    assert_gap_free(&events);

    // No duplicates: each sequence appears exactly once.
    let mut seen = std::collections::HashSet::new();
    for event in &events {
        assert!(seen.insert(event.sequence().value()), "duplicate sequence");
    }
}

/// Delivery class influences persistence but never the sequence total order:
/// transient events consume a slot yet are not persisted, so replay skips them
/// while `latest_sequence` tracks every slot ever issued.
#[test]
fn transient_writes_consume_but_do_not_persist() {
    let bus = Arc::new(EventBus::default());
    let cube = CubeId::new();

    let transient = Event::new(
        cube,
        UserId::new(),
        EventPayload::CursorMoved {
            path: "a.rs".into(),
            line: 1,
            column: 1,
        },
    );
    assert_eq!(transient.delivery(), DeliveryClass::Transient);

    let t1 = bus.publish(transient.clone()).unwrap();
    let d2 = bus.publish(durable(cube)).unwrap();
    let t3 = bus.publish(transient).unwrap();

    assert!(!t1.persisted);
    assert!(d2.persisted);
    assert!(!t3.persisted);

    // Sequences advance for every event...
    assert_eq!(t1.event.sequence().value(), 1);
    assert_eq!(d2.event.sequence().value(), 2);
    assert_eq!(t3.event.sequence().value(), 3);

    // ...but the persisted watermark only tracks durable slots.
    assert_eq!(bus.latest_sequence(cube).value(), 2);

    // ...but only the durable slot is replayed.
    let events = bus.replay(cube, EventSequence::UNASSIGNED).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].sequence().value(), 2);
}
