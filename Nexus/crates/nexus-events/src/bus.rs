use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use nexus_core::event::{Event, EventSequence};
use nexus_core::ids::CubeId;
use nexus_core::PerformanceMetrics;
use tokio::sync::mpsc;

use crate::batch::EventBatch;
use crate::consumer::ConsumerRunner;
use crate::filter::EventFilter;
use crate::metrics::EventMetrics;
use crate::seq::Sequencer;
use crate::store::{EventStore, MemoryEventStore};
use crate::trace::trace_event;
use crate::{Error, Result};

pub type SubscriptionId = u64;

/// One subscription attached to a Cube's stream through a bounded queue.
///
/// The subscriber drains through `wait` (async) or `drain` (sync, non-FIFO
/// batch). Publishing never blocks on a subscriber: if its queue is full the
/// event is dropped (transient) or counted as a slow-subscriber drop, which
/// is exactly the isolation contract (spec section 32).
#[derive(Debug)]
pub struct Subscription {
    pub id: SubscriptionId,
    pub filter: EventFilter,
    rx: mpsc::Receiver<Event>,
    bus: Arc<Inner>,
}

impl Subscription {
    /// Waits for the next matching event. Returns `None` when unsubscribed.
    pub async fn wait(&mut self) -> Option<Event> {
        self.rx.recv().await
    }

    /// Synchronously drains everything currently buffered, newest preserved,
    /// oldest-first order.
    pub fn drain(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            out.push(event);
        }
        out
    }

    /// Removes this subscription from the bus (drops the backing sender).
    pub fn unsubscribe(self) {
        self.bus.unsubscribe(self.id);
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.bus.unsubscribe(self.id);
    }
}

/// Result of publishing a single event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReceipt {
    pub event: Event,
    pub delivered_to: usize,
    pub persisted: bool,
}

/// Result of publishing a whole batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchReceipt {
    pub count: usize,
    pub delivered: usize,
    pub to: EventSequence,
}

#[derive(Debug)]
struct SubscriberEntry {
    id: SubscriptionId,
    filter: EventFilter,
    tx: mpsc::Sender<Event>,
}

#[derive(Debug)]
struct CubeState {
    sequencer: Arc<Sequencer>,
    gate: Arc<Mutex<()>>,
}

#[derive(Debug)]
struct Inner {
    store: Arc<dyn EventStore>,
    cubes: Mutex<HashMap<CubeId, CubeState>>,
    subscribers: Mutex<Vec<SubscriberEntry>>,
    metrics: Arc<EventMetrics>,
    perf: Arc<PerformanceMetrics>,
    consumers: Mutex<Option<ConsumerRunner>>,
    next_id: AtomicU64,
}

impl Inner {
    fn unsubscribe(&self, id: SubscriptionId) {
        let mut subs = self.subscribers.lock().unwrap();
        subs.retain(|entry| entry.id != id);
        self.metrics.observe_subscribers(subs.len() as u64);
    }

    fn cube_state(&self, cube: CubeId) -> (Arc<Sequencer>, Arc<Mutex<()>>) {
        let mut guard = self.cubes.lock().unwrap();
        let state = guard.entry(cube).or_insert_with(|| {
            let latest = self.store.latest_sequence(cube);
            CubeState {
                sequencer: Arc::new(Sequencer::starting_at(latest)),
                gate: Arc::new(Mutex::new(())),
            }
        });
        (state.sequencer.clone(), state.gate.clone())
    }

    fn fan_out(&self, event: &Event) -> usize {
        let mut delivered = 0usize;
        let mut prune: Vec<SubscriptionId> = Vec::new();
        let mut subs = self.subscribers.lock().unwrap();
        for entry in subs.iter() {
            if !entry.filter.matches(event) {
                continue;
            }
            match entry.tx.try_send(event.clone()) {
                Ok(()) => {
                    delivered += 1;
                    self.metrics.record_delivered();
                }
                Err(mpsc::error::TrySendError::Full(_)) => match event.delivery() {
                    nexus_core::event::DeliveryClass::Transient => {
                        self.metrics.record_dropped_transient();
                    }
                    _ => {
                        self.metrics.record_slow_subscriber_drop();
                    }
                },
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    prune.push(entry.id);
                }
            }
        }
        if !prune.is_empty() {
            subs.retain(|entry| !prune.contains(&entry.id));
            self.metrics.observe_subscribers(subs.len() as u64);
        }
        delivered
    }
}

/// The internal event backbone (spec section 20, 21, 22, 31, 32).
///
/// The bus owns the creation pipeline:
///   1. assign a per-Cube sequence (each Cube has its own sequencer + gate,
///      so independent Cubes publish in parallel; no global lock),
///   2. persist durable/reliable events to the [`EventStore`],
///   3. fan out to internal subscribers through bounded queues (isolation),
///   4. apply the registered [`ConsumerRunner`]s, and
///   5. record metrics and structured traces.
#[derive(Debug, Clone)]
pub struct EventBus {
    inner: Arc<Inner>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(
            Arc::new(MemoryEventStore::new()),
            None,
            Arc::new(EventMetrics::new()),
            Arc::new(nexus_core::PerformanceMetrics::new()),
        )
    }
}

impl EventBus {
    /// A bus over `store`, optionally driving `consumers`.
    pub fn new(
        store: Arc<dyn EventStore>,
        consumers: Option<ConsumerRunner>,
        metrics: Arc<EventMetrics>,
        perf: Arc<PerformanceMetrics>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                cubes: Mutex::new(HashMap::new()),
                subscribers: Mutex::new(Vec::new()),
                metrics,
                perf,
                consumers: Mutex::new(consumers),
                next_id: AtomicU64::new(1),
            }),
        }
    }

    /// The backing store (for replay, recovery, and inspection).
    #[must_use]
    pub fn store(&self) -> &Arc<dyn EventStore> {
        &self.inner.store
    }

    /// Global metrics handle.
    #[must_use]
    pub fn metrics(&self) -> &Arc<EventMetrics> {
        &self.inner.metrics
    }

    /// Subscribes with a bounded queue of `capacity` events.
    pub fn subscribe(&self, filter: EventFilter, capacity: usize) -> Subscription {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel(capacity.saturating_add(1));
        let mut subs = self.inner.subscribers.lock().unwrap();
        subs.push(SubscriberEntry {
            id,
            filter: filter.clone(),
            tx,
        });
        self.inner.metrics.observe_subscribers(subs.len() as u64);
        drop(subs);
        Subscription {
            id,
            filter,
            rx,
            bus: self.inner.clone(),
        }
    }

    /// Removes a subscription by id.
    pub fn unsubscribe(&self, id: SubscriptionId) {
        let mut subs = self.inner.subscribers.lock().unwrap();
        subs.retain(|entry| entry.id != id);
        self.inner.metrics.observe_subscribers(subs.len() as u64);
    }

    /// Number of live internal subscriptions.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        self.inner.subscribers.lock().unwrap().len()
    }

    /// Publishes an event: assigns a sequence (if unassigned), persists when
    /// its delivery class requires, then fans out. The returned event is the
    /// authoritative, sequenced record.
    pub fn publish(&self, event: Event) -> Result<PublishReceipt> {
        let cube = event.cube_id();
        if event.sequence() != EventSequence::UNASSIGNED {
            // Already sequenced: persist + fan out as-is.
            return self.publish_sequenced(event);
        }
        let (sequencer, gate) = self.inner.cube_state(cube);
        let _gate = gate.lock().unwrap();
        let seq = sequencer.reserve();
        let stamped = event.with_sequence(seq);
        self.persist_and_fan_out(stamped, &_gate)
    }

    /// Publishes an already-sequenced event (must not collide with the
    /// sequencer's cursor; used for re-publishing recovered records).
    pub fn publish_sequenced(&self, event: Event) -> Result<PublishReceipt> {
        if event.sequence() == EventSequence::UNASSIGNED {
            return Err(Error::AlreadySequenced);
        }
        let (_sequencer, gate) = self.inner.cube_state(event.cube_id());
        let _gate = gate.lock().unwrap();
        self.persist_and_fan_out(event, &_gate)
    }

    fn persist_and_fan_out<'a>(
        &self,
        event: Event,
        _gate: &std::sync::MutexGuard<'a, ()>,
    ) -> Result<PublishReceipt> {
        let perf_started = Instant::now();
        let persisted = event.delivery().is_persistent();
        if persisted {
            let started = event.timestamp().as_millis();
            if let Err(err) = self.inner.store.append(&event) {
                self.inner.metrics.record_failed();
                return Err(err);
            }
            self.inner.metrics.record_persist_delay_ms(
                nexus_core::timestamp::Timestamp::now()
                    .as_millis()
                    .saturating_sub(started),
            );
            self.inner.metrics.record_persisted();
        }
        let delivered = self.inner.fan_out(&event);
        if let Some(runner) = self.inner.consumers.lock().unwrap().as_mut() {
            let _ = runner.apply(&event);
        }
        trace_event(&event, None);
        self.inner
            .perf
            .record_latency_ms(perf_started.elapsed().as_millis() as u64);
        self.inner.perf.record_operation();
        self.inner.metrics.record_published();
        Ok(PublishReceipt {
            event,
            delivered_to: delivered,
            persisted,
        })
    }

    /// Publishes a validated batch under a single per-Cube sequence window,
    /// guaranteeing contiguous persisted sequences.
    pub fn publish_batch(&self, batch: EventBatch) -> Result<BatchReceipt> {
        batch.validate()?;
        let (sequencer, gate) = self.inner.cube_state(batch.cube);
        let _gate = gate.lock().unwrap();
        let mut delivered = 0usize;
        let mut count = 0usize;
        let mut to = EventSequence::UNASSIGNED;
        for mut event in batch.events {
            if event.sequence() == EventSequence::UNASSIGNED {
                let seq = sequencer.reserve();
                event = event.with_sequence(seq);
            }
            let receipt = self.persist_and_fan_out(event, &_gate)?;
            delivered += receipt.delivered_to;
            to = receipt.event.sequence();
            count += 1;
        }
        Ok(BatchReceipt {
            count,
            delivered,
            to,
        })
    }

    /// Replays `cube` from `from` (inclusive) out of the backing store.
    pub fn replay(&self, cube: CubeId, from: EventSequence) -> Result<Vec<Event>> {
        let events = self.inner.store.replay(cube, from)?;
        self.inner.metrics.record_replayed(events.len());
        Ok(events)
    }

    /// Latest persisted sequence for `cube`.
    #[must_use]
    pub fn latest_sequence(&self, cube: CubeId) -> EventSequence {
        self.inner.store.latest_sequence(cube)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SegmentedEventStore;
    use nexus_core::event::EventPayload;
    use nexus_core::ids::UserId;

    fn cube_event(cube: CubeId, payload: EventPayload) -> Event {
        Event::new(cube, UserId::new(), payload)
    }

    fn bus_durable(cube: CubeId, path: String) -> Event {
        cube_event(cube, EventPayload::FileCreated { path })
    }

    fn durable(cube: CubeId) -> Event {
        cube_event(cube, EventPayload::FileCreated { path: "/f".into() })
    }

    fn transient(cube: CubeId) -> Event {
        cube_event(
            cube,
            EventPayload::CursorMoved {
                path: "a".into(),
                line: 1,
                column: 1,
            },
        )
    }

    #[tokio::test]
    async fn publish_assigns_monotonic_sequences_per_cube() {
        let bus = EventBus::default();
        let cube = CubeId::new();
        for expected in 1..=5 {
            let receipt = bus.publish(durable(cube)).unwrap();
            assert_eq!(receipt.event.sequence().value(), expected);
            assert!(receipt.persisted);
        }
        assert_eq!(bus.latest_sequence(cube).value(), 5);
    }

    #[tokio::test]
    async fn cubes_are_independent_for_sequences() {
        let bus = EventBus::default();
        let a = CubeId::new();
        let b = CubeId::new();
        let ra = bus.publish(durable(a)).unwrap();
        let rb = bus.publish(durable(b)).unwrap();
        assert_eq!(ra.event.sequence().value(), 1);
        assert_eq!(rb.event.sequence().value(), 1);
    }

    #[tokio::test]
    async fn only_durable_events_persist() {
        let bus = EventBus::default();
        let cube = CubeId::new();
        // A transient still consumes a sequence number...
        let receipt = bus.publish(transient(cube)).unwrap();
        assert_eq!(receipt.event.sequence().value(), 1);
        assert!(!receipt.persisted);
        assert_eq!(bus.latest_sequence(cube).value(), 0);
        // ...but only durable events are persisted.
        let receipt = bus.publish(durable(cube)).unwrap();
        assert_eq!(receipt.event.sequence().value(), 2);
        assert!(receipt.persisted);
        assert_eq!(bus.latest_sequence(cube).value(), 2);
        assert_eq!(
            bus.replay(cube, EventSequence::UNASSIGNED).unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn subscribers_get_matching_events_only() {
        let bus = EventBus::default();
        let cube = CubeId::new();
        let mut cursors = bus.subscribe(
            EventFilter::types([nexus_core::event::EventType::CursorMoved]),
            64,
        );

        bus.publish(durable(cube)).unwrap();
        bus.publish(cursor_event(&bus, cube)).unwrap();

        assert_eq!(cursors.drain().len(), 1);
    }

    fn cursor_event(bus: &EventBus, cube: CubeId) -> Event {
        let _ = bus;
        cube_event(
            cube,
            EventPayload::CursorMoved {
                path: "a".into(),
                line: 2,
                column: 3,
            },
        )
    }

    #[tokio::test]
    async fn slow_subscriber_is_isolated() {
        let bus = EventBus::default();
        let cube = CubeId::new();
        let mut slow = bus.subscribe(EventFilter::all(), 1);
        let mut received = bus.subscribe(EventFilter::all(), 1024);

        // Fill the slow queue (capacity 1 + one buffered) with transients.
        for _ in 0..5 {
            bus.publish(transient(cube)).unwrap();
        }
        // The fast subscriber saw everything.
        assert_eq!(received.drain().len(), 5);
        // The slow one saw at most its capacity, others were dropped,
        // and publishing never failed (isolation).
        let slow_got = slow.drain().len();
        assert!(
            slow_got <= 2,
            "slow subscriber bounded by capacity, got {slow_got}"
        );
        let snap = bus.metrics().snapshot();
        assert!(snap.dropped_transient >= 3, "drops recorded: {snap}");
        assert_eq!(snap.failed, 0);
    }

    #[tokio::test]
    async fn unsubscribe_stops_delivery() {
        let bus = EventBus::default();
        let cube = CubeId::new();
        let mut sub = bus.subscribe(EventFilter::all(), 16);
        bus.publish(transient(cube)).unwrap();
        assert_eq!(sub.drain().len(), 1);
        bus.unsubscribe(sub.id);
        assert_eq!(bus.subscriber_count(), 0);
        bus.publish(transient(cube)).unwrap();
        assert_eq!(sub.drain().len(), 0);
    }

    #[tokio::test]
    async fn batch_publishes_contiguously() {
        let bus = EventBus::default();
        let cube = CubeId::new();
        let events: Vec<Event> = (0..3)
            .map(|_| cube_event(cube, EventPayload::FileCreated { path: "/f".into() }))
            .collect();
        let batch = EventBatch { cube, events };
        let receipt = bus.publish_batch(batch).unwrap();
        assert_eq!(receipt.count, 3);
        assert_eq!(receipt.to.value(), 3);
        assert_eq!(
            bus.replay(cube, EventSequence::UNASSIGNED).unwrap().len(),
            3
        );
    }

    #[tokio::test]
    async fn reopens_over_persisted_store() {
        use std::fs;
        let dir = std::env::temp_dir().join(format!("nexus-bus-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let cube = CubeId::new();
        {
            let store: Arc<dyn EventStore> = Arc::new(SegmentedEventStore::open(&dir).unwrap());
            let bus = EventBus::new(
                store,
                None,
                Arc::new(EventMetrics::new()),
                Arc::new(nexus_core::PerformanceMetrics::new()),
            );
            bus.publish(durable(cube)).unwrap();
            bus.publish(durable(cube)).unwrap();
        }
        {
            let store: Arc<dyn EventStore> = Arc::new(SegmentedEventStore::open(&dir).unwrap());
            let bus = EventBus::new(
                store,
                None,
                Arc::new(EventMetrics::new()),
                Arc::new(nexus_core::PerformanceMetrics::new()),
            );
            // Continues sequencing past the latest persisted value.
            let receipt = bus.publish(durable(cube)).unwrap();
            assert_eq!(receipt.event.sequence().value(), 3);
            assert_eq!(bus.replay(cube, EventSequence::new(2)).unwrap().len(), 2);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn publish_fails_when_store_appends_out_of_order() {
        #[derive(Debug)]
        struct TrapStore;
        impl EventStore for TrapStore {
            fn append(&self, _e: &Event) -> Result<()> {
                Err(Error::SequenceMismatch {
                    expected: 1,
                    got: 2,
                })
            }
            fn get(&self, _c: CubeId, _s: EventSequence) -> Result<Option<Event>> {
                Ok(None)
            }
            fn replay(&self, _c: CubeId, _f: EventSequence) -> Result<Vec<Event>> {
                Ok(vec![])
            }
            fn latest_sequence(&self, _c: CubeId) -> EventSequence {
                EventSequence::UNASSIGNED
            }
            fn remove_cube(&self, _c: CubeId) {}
        }
        let bus = EventBus::new(
            Arc::new(TrapStore),
            None,
            Arc::new(EventMetrics::new()),
            Arc::new(nexus_core::PerformanceMetrics::new()),
        );
        let cube = CubeId::new();
        assert!(bus.publish(durable(cube)).is_err());
        assert!(!bus.publish(transient(cube)).unwrap().persisted);
    }

    #[tokio::test]
    async fn publish_failure_is_recorded_in_metrics() {
        #[derive(Debug)]
        struct FailingStore;
        impl EventStore for FailingStore {
            fn append(&self, _e: &Event) -> Result<()> {
                Err(Error::Serialization("injected store failure".into()))
            }
            fn get(&self, _c: CubeId, _s: EventSequence) -> Result<Option<Event>> {
                Ok(None)
            }
            fn replay(&self, _c: CubeId, _f: EventSequence) -> Result<Vec<Event>> {
                Ok(vec![])
            }
            fn latest_sequence(&self, _c: CubeId) -> EventSequence {
                EventSequence::UNASSIGNED
            }
            fn remove_cube(&self, _c: CubeId) {}
        }
        let bus = EventBus::new(
            Arc::new(FailingStore),
            None,
            Arc::new(EventMetrics::new()),
            Arc::new(nexus_core::PerformanceMetrics::new()),
        );
        let cube = CubeId::new();
        assert!(bus.publish(durable(cube)).is_err());
        let snap = bus.metrics().snapshot();
        assert_eq!(snap.failed, 1, "failure must be accounted: {snap}");
        assert_eq!(snap.persisted, 0);
        // Transient events bypass the store and still publish.
        assert!(!bus.publish(transient(cube)).unwrap().persisted);
        assert_eq!(bus.metrics().snapshot().failed, 1);
    }

    #[tokio::test]
    async fn transient_delivery_is_dropped_on_full_queue() {
        let bus = EventBus::default();
        let cube = CubeId::new();
        let mut slow = bus.subscribe(EventFilter::all(), 0);
        for _ in 0..10 {
            bus.publish(transient(cube)).unwrap();
        }
        assert!(slow.drain().len() <= 2);
        // Non-transient still reaches a healthy subscriber.
        let mut healthy = bus.subscribe(EventFilter::all(), 64);
        bus.publish(cube_event(
            cube,
            EventPayload::ChatMessageSent { text: "hi".into() },
        ))
        .unwrap();
        assert_eq!(healthy.drain().len(), 1);
    }

    #[tokio::test]
    async fn submit_unsequenced_twice_is_legal_fresh_events() {
        let bus = EventBus::default();
        let cube = CubeId::new();
        // Two logically equal payloads get two distinct ids and sequences.
        let a = bus.publish(durable(cube)).unwrap();
        let b = bus.publish(durable(cube)).unwrap();
        assert_ne!(a.event.sequence(), b.event.sequence());
        assert_ne!(a.event.id(), b.event.id());
    }

    #[tokio::test]
    async fn concurrent_publishers_produce_gapless_persisted_stream() {
        use std::sync::Arc;
        let bus = Arc::new(EventBus::default());
        let cube = CubeId::new();
        let publishers: Vec<_> = (0..8)
            .map(|p| {
                let bus = bus.clone();
                tokio::spawn(async move {
                    for i in 0..100u64 {
                        let path = format!("/p{p}-{i}.rs");
                        let _ = bus.publish(bus_durable(cube, path).clone());
                    }
                })
            })
            .collect();
        for handle in publishers {
            handle.await.unwrap();
        }
        let events = bus.replay(cube, EventSequence::UNASSIGNED).unwrap();
        assert_eq!(events.len(), 800, "every durability publish persisted");
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e.sequence().value(), (i + 1) as u64, "contiguous ordering");
        }
    }
}
