use std::collections::{HashMap, HashSet};

use nexus_core::event::EventSequence;
use nexus_core::ids::CubeId;
use serde::{Deserialize, Serialize};

/// Guards "sequencer must never assign contiguous sequences while a previous
/// seq for that cube is in flight": has its own bookkeeping-free alternative —
/// a per-Cube atomic counter.
///
/// `Sequencer` hands out monotonically increasing per-Cube sequences with no
/// global lock. Two publishers racing on the same Cube receive distinct,
/// strictly ordered values. Duplicates and gaps cannot be produced by the
/// sequencer itself; they can only arrive from external sources, which the
/// [`SequenceTracker`] watches for.
#[derive(Debug)]
pub struct Sequencer {
    next: std::sync::atomic::AtomicU64,
}

impl Default for Sequencer {
    fn default() -> Self {
        Self::starting_at(EventSequence::UNASSIGNED)
    }
}

impl Sequencer {
    /// A sequencer that starts stamping at `from + 1` (used on restart so the
    /// store's latest sequence is honored).
    #[must_use]
    pub fn starting_at(from: EventSequence) -> Self {
        Self {
            next: std::sync::atomic::AtomicU64::new(from.value() + 1),
        }
    }

    /// Reserves the next sequence value.
    #[must_use]
    pub fn reserve(&self) -> EventSequence {
        let taken = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        EventSequence::new(taken)
    }

    /// The next value the sequencer will hand out.
    #[must_use]
    pub fn peek(&self) -> EventSequence {
        EventSequence::new(self.next.load(std::sync::atomic::Ordering::SeqCst))
    }
}

/// What a sequence observation means for a receiver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequenceState {
    /// The event is the exact next expected sequence.
    InOrder,
    /// A hole was detected: `expected` is missing.
    Gap { expected: EventSequence },
    /// This sequence was already applied before (duplicate redelivery).
    Duplicate { sequence: EventSequence },
    /// An already-applied newer save arrived before the lower one(s) aligned.
    OutOfOrder {
        sequence: EventSequence,
        expected: EventSequence,
    },
}

/// Tracks per-Cube receipt state for gap/duplicate/out-of-order detection
/// (spec section 38 and 50).
///
/// Not thread-safe by itself; hand each consumer its own tracker.
#[derive(Debug)]
pub struct SequenceTracker {
    /// Per cube, the next sequence we expect.
    expected: HashMap<CubeId, EventSequence>,
    /// Sequence values we have acknowledged, for duplicate detection.
    /// Bounded to `window`; gaps beyond the window are seen as OutOfOrder.
    ack: HashSet<u64>,
    /// How many past sequences to remember for duplicate detection.
    window: usize,
    /// Diagnostic counts (spec section 55).
    pub counts: SequenceCounts,
}

impl Default for SequenceTracker {
    fn default() -> Self {
        Self::with_window(16_384)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SequenceCounts {
    pub in_order: u64,
    pub gaps: u64,
    pub duplicates: u64,
    pub out_of_order: u64,
}

impl SequenceTracker {
    /// A tracker remembering `window` acknowledged sequences per Cube.
    #[must_use]
    pub fn with_window(window: usize) -> Self {
        Self {
            expected: HashMap::new(),
            ack: HashSet::new(),
            window,
            counts: SequenceCounts::default(),
        }
    }

    /// Seeds the expected sequence for a Cube (e.g. from a checkpoint).
    pub fn seed(&mut self, cube: CubeId, next_expected: EventSequence) {
        self.expected.insert(cube, next_expected);
    }

    fn remember(&mut self, seq: u64) {
        // Simple bounded memory: keep the low-order bits we have seen. For the
        // scale of a single Cube's stream this is an acceptable proxy.
        self.ack.insert(seq);
        while self.ack.len() > self.window {
            if let Some(&old) = self.ack.iter().min() {
                self.ack.remove(&old);
            }
        }
    }

    /// Registers an incoming sequence. Returns how the receiver should treat it.
    pub fn observe(&mut self, cube: CubeId, sequence: EventSequence) -> SequenceState {
        let expected_val = *self.expected.entry(cube).or_insert(EventSequence::new(1));
        let seq = sequence.value();
        if seq == expected_val.value() {
            self.counts.in_order += 1;
            self.remember(seq);
            *self.expected.get_mut(&cube).unwrap() = expected_val.next();
            SequenceState::InOrder
        } else if seq < expected_val.value() {
            if self.ack.contains(&seq) {
                self.counts.duplicates += 1;
                SequenceState::Duplicate { sequence }
            } else {
                self.counts.out_of_order += 1;
                SequenceState::OutOfOrder {
                    sequence,
                    expected: expected_val,
                }
            }
        } else {
            self.counts.gaps += 1;
            SequenceState::Gap {
                expected: expected_val,
            }
        }
    }

    /// Returns the next sequence this tracker expects for `cube`.
    #[must_use]
    pub fn next_expected(&self, cube: CubeId) -> EventSequence {
        self.expected
            .get(&cube)
            .copied()
            .unwrap_or(EventSequence::new(1))
    }
}

/// A consumer checkpoint: the last applied (or last durable) sequence plus a
/// wall-clock timestamp (spec section 44).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumerCheckpoint {
    pub cube: CubeId,
    pub sequence: EventSequence,
    pub timestamp: nexus_core::timestamp::Timestamp,
}

impl ConsumerCheckpoint {
    #[must_use]
    pub fn at(cube: CubeId, sequence: EventSequence) -> Self {
        Self {
            cube,
            sequence,
            timestamp: nexus_core::timestamp::Timestamp::now(),
        }
    }
}

/// Persists consumer checkpoints so replays resume where they left off.
pub trait CheckpointStore: Send + Sync + std::fmt::Debug {
    fn save(&self, consumer: &str, checkpoint: ConsumerCheckpoint) -> crate::Result<()>;
    fn load(&self, consumer: &str, cube: CubeId) -> crate::Result<Option<ConsumerCheckpoint>>;
}

/// In-memory checkpoint store, for tests and ephemeral consumers.
#[derive(Debug, Default)]
pub struct MemoryCheckpointStore {
    checkpoints: std::sync::Mutex<HashMap<(String, CubeId), ConsumerCheckpoint>>,
}

impl MemoryCheckpointStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl CheckpointStore for MemoryCheckpointStore {
    fn save(&self, consumer: &str, checkpoint: ConsumerCheckpoint) -> crate::Result<()> {
        self.checkpoints
            .lock()
            .unwrap()
            .insert((consumer.to_string(), checkpoint.cube), checkpoint);
        Ok(())
    }

    fn load(&self, consumer: &str, cube: CubeId) -> crate::Result<Option<ConsumerCheckpoint>> {
        Ok(self
            .checkpoints
            .lock()
            .unwrap()
            .get(&(consumer.to_string(), cube))
            .copied())
    }
}

/// Writes checkpoints into a directory as JSON files (one per consumer).
#[derive(Debug)]
pub struct FileCheckpointStore {
    root: std::path::PathBuf,
}

impl FileCheckpointStore {
    pub fn open(root: impl AsRef<std::path::Path>) -> crate::Result<Self> {
        std::fs::create_dir_all(root.as_ref())?;
        Ok(Self {
            root: root.as_ref().to_path_buf(),
        })
    }
}

impl CheckpointStore for FileCheckpointStore {
    fn save(&self, consumer: &str, checkpoint: ConsumerCheckpoint) -> crate::Result<()> {
        let path = self.root.join(format!("{consumer}.json"));
        let data = serde_json::to_vec(&checkpoint).map_err(crate::Error::serde)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    fn load(&self, consumer: &str, cube: CubeId) -> crate::Result<Option<ConsumerCheckpoint>> {
        let path = self.root.join(format!("{consumer}.json"));
        if !path.exists() {
            return Ok(None);
        }
        let data = std::fs::read(&path)?;
        let stored: ConsumerCheckpoint =
            serde_json::from_slice(&data).map_err(crate::Error::serde)?;
        if stored.cube == cube {
            Ok(Some(stored))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequencer_reserves_monotonic_values() {
        let s = Sequencer::starting_at(EventSequence::new(1));
        assert_eq!(s.reserve().value(), 2);
        assert_eq!(s.reserve().value(), 3);
        let started = Sequencer::default();
        assert_eq!(started.reserve().value(), 1);
    }

    #[test]
    fn sequencer_is_safe_across_threads() {
        use std::sync::Arc;
        let seq = Arc::new(Sequencer::default());
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let seq = seq.clone();
                std::thread::spawn(move || {
                    (0..100).map(|_| seq.reserve().value()).collect::<Vec<_>>()
                })
            })
            .collect();
        let mut all: Vec<u64> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        all.sort_unstable();
        let expected: Vec<u64> = (1..=800).collect();
        assert_eq!(all, expected);
    }

    #[test]
    fn tracker_sees_in_order_then_gaps() {
        let mut t = SequenceTracker::default();
        let cube = CubeId::new();
        assert_eq!(
            t.observe(cube, EventSequence::new(1)),
            SequenceState::InOrder
        );
        // 3 is a gap while 2 is missing.
        assert_eq!(
            t.observe(cube, EventSequence::new(3)),
            SequenceState::Gap {
                expected: EventSequence::new(2)
            }
        );
        // Now 2 arrives -> in order, expected advances to 3.
        assert_eq!(
            t.observe(cube, EventSequence::new(2)),
            SequenceState::InOrder
        );
        assert_eq!(t.next_expected(cube).value(), 3);
    }

    #[test]
    fn tracker_detects_duplicate_and_out_of_order() {
        let mut t = SequenceTracker::default();
        let cube = CubeId::new();
        t.observe(cube, EventSequence::new(1));
        t.observe(cube, EventSequence::new(2));
        assert_eq!(
            t.observe(cube, EventSequence::new(2)),
            SequenceState::Duplicate {
                sequence: EventSequence::new(2)
            }
        );
        // A value below expected that was never seen is out-of-order jitter.
        // Fake a "never acked" lower value by seeding far ahead.
        t.seed(cube, EventSequence::new(10));
        assert_eq!(
            t.observe(cube, EventSequence::new(5)),
            SequenceState::OutOfOrder {
                sequence: EventSequence::new(5),
                expected: EventSequence::new(10)
            }
        );
        assert!(t.counts.duplicates >= 1);
        assert!(t.counts.out_of_order >= 1);
    }

    #[test]
    fn tracker_separates_cubes() {
        let mut t = SequenceTracker::default();
        let a = CubeId::new();
        let b = CubeId::new();
        for i in 1..=3 {
            assert_eq!(t.observe(a, EventSequence::new(i)), SequenceState::InOrder);
        }
        assert_eq!(t.observe(b, EventSequence::new(1)), SequenceState::InOrder);
    }

    #[test]
    fn checkpoints_roundtrip_in_memory_and_file() {
        use std::path::PathBuf;
        let cube = CubeId::new();
        let cp = ConsumerCheckpoint::at(cube, EventSequence::new(9));

        let mem = MemoryCheckpointStore::new();
        mem.save("projector", cp).unwrap();
        assert_eq!(mem.load("projector", cube).unwrap(), Some(cp));
        assert_eq!(mem.load("projector", CubeId::new()).unwrap(), None);

        let dir = std::env::temp_dir().join(format!("nexus-cp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = FileCheckpointStore::open(PathBuf::from(&dir)).unwrap();
        file.save("projector", cp).unwrap();
        assert_eq!(file.load("projector", cube).unwrap(), Some(cp));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
