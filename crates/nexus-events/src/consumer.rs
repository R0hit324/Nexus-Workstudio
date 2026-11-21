use std::collections::HashMap;
use std::panic::AssertUnwindSafe;

use nexus_core::event::Event;
use nexus_core::ids::CubeId;

use crate::seq::{CheckpointStore, ConsumerCheckpoint};
use crate::{Error, Result};

/// A named, resumable event consumer (spec section 42, 43, 44).
///
/// Consumers hold derived state; they are fed events in strict per-Cube
/// order and may persist a checkpoint between runs so a restart resumes
/// exactly where the last run stopped (at-least-once with dedupe via the
/// ack tracker).
pub trait Consumer: Send + Sync + std::fmt::Debug {
    /// Stable consumer name, used as the checkpoint key.
    fn name(&self) -> &'static str;

    /// Applies one event to the consumer state.
    fn handle(&mut self, event: &Event) -> Result<()>;

    /// Optional: the label shown in operational dashboards.
    fn description(&self) -> &'static str {
        ""
    }
}

/// Runs consumers against an event stream with isolation guarantees:
/// a panicking or failing consumer is caught, logged, and skipped without
/// halting the remaining consumers (spec section 32, 49).
#[derive(Debug)]
pub struct ConsumerRunner {
    consumers: HashMap<String, Box<dyn Consumer>>,
    checkpoints: Box<dyn CheckpointStore>,
}

impl ConsumerRunner {
    pub fn new(checkpoints: Box<dyn CheckpointStore>) -> Self {
        Self {
            consumers: HashMap::new(),
            checkpoints,
        }
    }

    /// Registers a consumer unless a consumer with the same name exists.
    pub fn register(&mut self, consumer: impl Consumer + 'static) -> Result<()> {
        let name = consumer.name().to_string();
        if self.consumers.contains_key(&name) {
            return Err(Error::UnexpectedEventType(name));
        }
        self.consumers.insert(name, Box::new(consumer));
        Ok(())
    }

    /// All registered consumer names.
    pub fn names(&self) -> Vec<&'static str> {
        self.consumers.values().map(|c| c.name()).collect()
    }

    /// The last processed checkpoint for `consumer` in `cube`.
    pub fn checkpoint_of(&self, consumer: &str, cube: CubeId) -> Option<ConsumerCheckpoint> {
        self.checkpoints.load(consumer, cube).ok().flatten()
    }

    /// Feeds one event to every registered consumer.
    ///
    /// Consumers never see events that are not sequenced. A consumer that
    /// rejects the event (Err) is treated as a failure: it is logged,
    /// removed, and delivery to remaining consumers proceeds. The event is
    /// never skipped silently for other consumers.
    pub fn apply(&mut self, event: &Event) -> Result<()> {
        if !event.is_sequenced() {
            return Err(Error::AlreadySequenced);
        }
        self.apply_deliver(event);
        tracing::debug!(
            event.type = event.event_type().name(),
            event.seq = event.sequence().value(),
            "event applied to consumers"
        );
        Ok(())
    }

    fn apply_deliver(&mut self, event: &Event) {
        let names: Vec<String> = self.consumers.keys().cloned().collect();
        let mut failed = Vec::new();
        for name in names {
            let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let consumer = self.consumers.get_mut(&name).expect("consumer loaded");
                consumer.handle(event)
            }));
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    tracing::error!(consumer = %name, error = %err, "consumer rejected event; removing it");
                    failed.push(name);
                }
                Err(_) => {
                    tracing::error!(consumer = %name, "consumer panicked; removing it");
                    failed.push(name);
                }
            }
        }
        for name in failed {
            self.consumers.remove(&name);
        }
    }

    /// Persists a checkpoint for one consumer+cube.
    pub fn checkpoint(&self, consumer: &str, checkpoint: ConsumerCheckpoint) -> Result<()> {
        self.checkpoints.save(consumer, checkpoint)
    }

    /// Number of healthy consumers.
    pub fn count(&self) -> usize {
        self.consumers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::seq::MemoryCheckpointStore;
    use nexus_core::event::{Event, EventPayload, EventSequence};
    use nexus_core::ids::{CubeId, UserId};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[derive(Debug)]
    struct CountConsumer {
        name: &'static str,
        handled: Arc<AtomicUsize>,
        fail_at: Option<u64>,
    }

    impl Consumer for CountConsumer {
        fn name(&self) -> &'static str {
            self.name
        }
        fn handle(&mut self, event: &Event) -> Result<()> {
            if self.fail_at == Some(event.sequence().value()) {
                return Err(Error::FilteredOut);
            }
            self.handled.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    fn ev(seq: u64) -> Event {
        Event::with_timestamp(
            CubeId::new(),
            UserId::new(),
            EventPayload::UserJoined {
                user_id: UserId::new(),
            },
            nexus_core::timestamp::Timestamp::from_millis(seq),
        )
        .with_sequence(EventSequence::new(seq))
    }

    #[test]
    fn consumer_receives_sequenced_events_only() {
        let handled = Arc::new(AtomicUsize::new(0));
        let mut runner = ConsumerRunner::new(Box::new(MemoryCheckpointStore::new()));
        runner
            .register(CountConsumer {
                name: "counter",
                handled: handled.clone(),
                fail_at: None,
            })
            .unwrap();
        assert_eq!(runner.names(), vec!["counter"]);

        let unassigned = Event::new(CubeId::new(), UserId::new(), EventPayload::None);
        assert!(runner.apply(&unassigned).is_err());

        for seq in 1..=3 {
            runner.apply(&ev(seq)).unwrap();
        }
        assert_eq!(handled.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn failing_consumer_is_removed_not_crashing_others() {
        let healthy = Arc::new(AtomicUsize::new(0));
        let failing = Arc::new(AtomicUsize::new(0));
        let mut runner = ConsumerRunner::new(Box::new(MemoryCheckpointStore::new()));
        runner
            .register(CountConsumer {
                name: "failing",
                handled: failing.clone(),
                fail_at: Some(2),
            })
            .unwrap();
        runner
            .register(CountConsumer {
                name: "healthy",
                handled: healthy.clone(),
                fail_at: None,
            })
            .unwrap();
        assert_eq!(runner.count(), 2);

        runner.apply(&ev(1)).unwrap();
        assert_eq!(runner.count(), 2);

        runner.apply(&ev(2)).unwrap();
        // The failing consumer was dropped; the healthy one remains.
        assert_eq!(runner.count(), 1);
        assert_eq!(failing.load(Ordering::Relaxed), 1);
        assert_eq!(healthy.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn checkpoints_persist_and_resume() {
        let mut runner = ConsumerRunner::new(Box::new(MemoryCheckpointStore::new()));
        runner
            .register(CountConsumer {
                name: "counter",
                handled: Arc::new(AtomicUsize::new(0)),
                fail_at: None,
            })
            .unwrap();
        let cube = CubeId::new();
        runner
            .checkpoint(
                "counter",
                ConsumerCheckpoint::at(cube, EventSequence::new(5)),
            )
            .unwrap();
        let loaded = runner.checkpoint_of("counter", cube).unwrap();
        assert_eq!(loaded.sequence.value(), 5);
    }
}
