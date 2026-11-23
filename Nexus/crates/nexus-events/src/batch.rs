use nexus_core::event::{Event, EventSequence};
use nexus_core::ids::CubeId;

/// A contiguous run of events for one Cube (spec section 7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventBatch {
    pub cube: CubeId,
    pub events: Vec<Event>,
}

impl EventBatch {
    /// The first sequence in the batch.
    #[must_use]
    pub fn from(&self) -> Option<EventSequence> {
        self.events.first().map(Event::sequence)
    }

    /// The last sequence in the batch.
    #[must_use]
    pub fn to(&self) -> Option<EventSequence> {
        self.events.last().map(Event::sequence)
    }

    /// Number of events in the batch.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the batch is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Validates the batch: every event must belong to the declared Cube;
    /// if events are sequenced, their sequences must be contiguous.
    ///
    /// Unsequenced batches (fresh events awaiting the sequencer) pass
    /// validation; the bus assigns all of them contiguously under one
    /// per-Cube critical section.
    pub fn validate(&self) -> crate::Result<()> {
        for event in &self.events {
            if event.cube_id() != self.cube {
                return Err(crate::Error::InvalidCube);
            }
        }
        if self.events.len() < 2 || self.events.iter().any(|e| !e.is_sequenced()) {
            return Ok(());
        }
        let mut expected = self.events[0].sequence().value();
        for event in &self.events[1..] {
            expected += 1;
            if event.sequence().value() != expected {
                return Err(crate::Error::SequenceMismatch {
                    expected,
                    got: event.sequence().value(),
                });
            }
        }
        Ok(())
    }
}

/// Collects events into [`EventBatch`]es for efficient, order-respecting
/// fan-out and persistence (spec section 22). Callers must flush manually
/// (`flush`) or after a time budget (`flush_if_due`); the collector never
/// holds events beyond `max_size`.
#[derive(Debug)]
pub struct BatchCollector {
    cube: Option<CubeId>,
    events: Vec<Event>,
    max_size: usize,
}

impl BatchCollector {
    /// A collector that flushes after `max_size` events.
    #[must_use]
    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            cube: None,
            events: Vec::with_capacity(max_size.min(1024)),
            max_size,
        }
    }

    /// Feeds an event. When the batch's target cube differs from an existing
    /// pending batch, the pending batch is returned for flushing first.
    pub fn push(&mut self, event: Event) -> Option<EventBatch> {
        match self.cube {
            None => {
                let event = event.clone();
                self.cube = Some(event.cube_id());
                self.events.push(event);
                if self.events.len() >= self.max_size {
                    self.flush()
                } else {
                    None
                }
            }
            Some(cube) if cube == event.cube_id() => {
                self.events.push(event);
                if self.events.len() >= self.max_size {
                    self.flush()
                } else {
                    None
                }
            }
            Some(_) => {
                let pending = self.flush();
                let event = event.clone();
                self.cube = Some(event.cube_id());
                self.events.push(event);
                pending
            }
        }
    }

    /// Extracts the pending batch, if any.
    pub fn flush(&mut self) -> Option<EventBatch> {
        if self.events.is_empty() {
            self.cube = None;
            return None;
        }
        let batch = EventBatch {
            cube: self.cube.expect("cube set when nonempty"),
            events: std::mem::take(&mut self.events),
        };
        self.cube = None;
        Some(batch)
    }

    /// Flushes when a wall-clock budget (in ms) has elapsed since the first
    /// pending event.
    pub fn flush_if_due(&mut self, now_ms: u64, budget_ms: u64) -> Option<EventBatch> {
        match self.events.first() {
            Some(first) => {
                let age = now_ms.saturating_sub(first.timestamp().as_millis());
                if age >= budget_ms {
                    self.flush()
                } else {
                    None
                }
            }
            None => None,
        }
    }

    /// Whether the collector currently holds events.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::event::EventPayload;
    use nexus_core::ids::UserId;

    fn ev(cube: CubeId, seq: u64) -> Event {
        Event::with_timestamp(
            cube,
            UserId::new(),
            EventPayload::FileCreated {
                path: format!("/f-{seq}"),
            },
            nexus_core::timestamp::Timestamp::from_millis(1000 + seq),
        )
        .with_sequence(EventSequence::new(seq))
    }

    #[test]
    fn batch_validates_contiguity() {
        let cube = CubeId::new();
        let batch = EventBatch {
            cube,
            events: vec![ev(cube, 1), ev(cube, 2), ev(cube, 3)],
        };
        batch.validate().unwrap();
        assert_eq!(batch.from(), Some(EventSequence::new(1)));
        assert_eq!(batch.to(), Some(EventSequence::new(3)));

        let broken = EventBatch {
            cube,
            events: vec![ev(cube, 1), ev(cube, 3)],
        };
        assert!(broken.validate().is_err());
    }

    #[test]
    fn collector_agglomerates_per_cube() {
        let cube_a = CubeId::new();
        let cube_b = CubeId::new();
        let mut collector = BatchCollector::with_max_size(3);
        assert!(collector.push(ev(cube_a, 1)).is_none());
        assert!(collector.push(ev(cube_a, 2)).is_none());

        // First 3rd fills the batch and returns it.
        let batch = collector.push(ev(cube_a, 3)).unwrap();
        batch.validate().unwrap();
        assert_eq!(batch.len(), 3);

        // Switching cubes flushes nothing (empty) and starts cube_b.
        assert!(collector.push(ev(cube_b, 1)).is_none());
        let flushed = collector.flush().unwrap();
        assert_eq!(flushed.cube, cube_b);
        assert!(collector.is_empty());
        assert!(collector.flush().is_none());
    }

    #[test]
    fn collector_flushes_when_due() {
        let cube = CubeId::new();
        let mut collector = BatchCollector::with_max_size(1000);
        collector.push(ev(cube, 1));
        assert!(collector.flush_if_due(1050, 1000).is_none());
        let batch = collector.flush_if_due(2500, 1000).unwrap();
        assert_eq!(batch.len(), 1);
    }
}
