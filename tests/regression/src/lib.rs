//! Permanent regression suite (Part 14, spec sections 70 and 84).
//!
//! Every serious defect found in Nexus gets a numbered, stable regression
//! test here before (or together with) its fix. Tests are grouped by
//! subsystem and keep their identifier forever:
//!
//! | ID | Subsystem | Behavior protected |
//! |----|-----------|--------------------|
//! | REG-001 | Performance | Benchmark regression detection semantics |
//! | REG-002 | Events | Failed publishes are accounted, never persisted |
//! | REG-003 | Events | Stores reject duplicate and out-of-order appends |
//! | REG-004 | Support | Temporary environments never leak state |
//!
//! Adding a test: pick the next free `REG-NNN` identifier, name the test
//! `reg_NNN_<behavior>`, and link the issue or root cause in a comment.

#![cfg_attr(not(test), allow(dead_code, unused_imports))]

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nexus_core::bench::{is_regressed, BenchmarkReport};
    use nexus_core::event::{Event, EventPayload};
    use nexus_core::ids::{CubeId, UserId};
    use nexus_core::PerformanceMetrics;
    use nexus_events::bus::EventBus;
    use nexus_events::metrics::EventMetrics;
    use nexus_events::store::{EventStore, MemoryEventStore};
    use nexus_tests_support::{cube_id, user_id, FaultInjector, FaultyEventStore, TempEnv};

    fn report(name: &str, ops_per_second: f64) -> BenchmarkReport {
        BenchmarkReport {
            name: name.into(),
            iterations: 1_000,
            total_duration_ms: 10,
            ops_per_second,
        }
    }

    /// REG-001: regression detection must flag a throughput drop larger than
    /// the configured budget and ignore noise inside it.
    #[test]
    fn reg_001_benchmark_regression_detection() {
        let baseline = report("regression.throughput", 100_000.0);
        let within_budget = report("regression.throughput", 95_000.0);
        let regressed = report("regression.throughput", 50_000.0);

        assert!(!is_regressed(&baseline, &within_budget, 0.10));
        assert!(is_regressed(&baseline, &regressed, 0.10));
        // A zero baseline cannot regress.
        assert!(!is_regressed(&report("x", 0.0), &regressed, 0.0));
    }

    /// REG-002: a store rejection must be counted as a failed publish and
    /// must never persist the event; transient events still flow.
    #[tokio::test]
    async fn reg_002_failed_publish_is_accounted_and_not_persisted() {
        let faults = Arc::new(FaultInjector::new());
        let store: Arc<dyn EventStore> = Arc::new(FaultyEventStore::new(
            Arc::new(MemoryEventStore::new()),
            faults.clone(),
        ));
        let bus = EventBus::new(
            store,
            None,
            Arc::new(EventMetrics::new()),
            Arc::new(PerformanceMetrics::new()),
        );

        let cube = cube_id(1);
        let actor = user_id(1);

        faults.fail_next(1);
        let durable = Event::new(
            cube,
            actor,
            EventPayload::FileCreated {
                path: "a.rs".into(),
            },
        );
        assert!(bus.publish(durable).is_err());

        let snapshot = bus.metrics().snapshot();
        assert_eq!(snapshot.failed, 1);
        assert_eq!(snapshot.persisted, 0);

        // Transient events bypass the store and must still publish.
        let transient = Event::new(
            cube,
            actor,
            EventPayload::CursorMoved {
                path: "a.rs".into(),
                line: 1,
                column: 1,
            },
        );
        assert!(!bus.publish(transient).unwrap().persisted);
        assert_eq!(bus.metrics().snapshot().failed, 1);
    }

    /// REG-003: stores must reject duplicate and out-of-order appends so
    /// replay order is authoritative.
    #[tokio::test]
    async fn reg_003_store_rejects_duplicate_and_out_of_order_appends() {
        use nexus_core::event::EventSequence;

        let store = MemoryEventStore::new();
        let cube = CubeId::from_u64(3);
        let actor = UserId::from_u64(3);
        let event = |sequence: u64| {
            Event::new(
                cube,
                actor,
                EventPayload::FileCreated {
                    path: format!("f{sequence}.rs"),
                },
            )
            .with_sequence(EventSequence::new(sequence))
        };

        assert!(store.append(&event(1)).is_ok());
        // Duplicate sequence.
        assert!(store.append(&event(1)).is_err());
        // Gap (2 missing).
        assert!(store.append(&event(3)).is_err());
        // In-order continuation works.
        assert!(store.append(&event(2)).is_ok());
        assert_eq!(
            store.replay(cube, EventSequence::UNASSIGNED).unwrap().len(),
            2
        );
    }

    /// REG-004: temporary test environments must clean up after themselves.
    #[test]
    fn reg_004_temp_env_does_not_leak_state() {
        let path;
        {
            let env = TempEnv::new("reg-004");
            path = env.path().to_path_buf();
            assert!(path.is_dir());
            std::fs::write(env.join("artifact.txt"), b"data").unwrap();
        }
        assert!(!path.exists(), "temp env leaked: {}", path.display());
    }
}
