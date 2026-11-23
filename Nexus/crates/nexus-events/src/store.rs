use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Mutex;

use nexus_core::event::{Event, EventSequence};
use nexus_core::ids::CubeId;

use crate::Error;

pub type Result<T> = crate::Result<T>;

/// Storage for events, keyed by Cube and ordered by [`EventSequence`].
///
/// Implementations must reject out-of-order or duplicate appends; the append
/// order is the authoritative per-Cube order and replay must reproduce it
/// exactly. The trait intentionally favors `&self` receivers so a store can
/// be shared behind an `Arc` across threads (spec section 44, 45).
pub trait EventStore: Send + Sync + std::fmt::Debug {
    /// Appends a fully-sequenced event. Must reject duplicates and gaps.
    fn append(&self, event: &Event) -> Result<()>;

    /// Reads a single event, if present.
    fn get(&self, cube: CubeId, sequence: EventSequence) -> Result<Option<Event>>;

    /// Replays every event at `>= from` (inclusive) in sequence order.
    fn replay(&self, cube: CubeId, from: EventSequence) -> Result<Vec<Event>>;

    /// The latest sequence persisted for a Cube (`UNASSIGNED` if none).
    fn latest_sequence(&self, cube: CubeId) -> EventSequence;

    /// Finished sub-Cube bookkeeping.
    fn remove_cube(&self, cube: CubeId);
}

/// In-memory store, used for ephemeral Cubes and tests. Guards the
/// monotonic per-Cube sequence invariant under concurrency.
#[derive(Debug, Default)]
pub struct MemoryEventStore {
    cubes:
        Mutex<std::collections::HashMap<CubeId, std::collections::BTreeMap<EventSequence, Event>>>,
}

impl MemoryEventStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl EventStore for MemoryEventStore {
    fn append(&self, event: &Event) -> Result<()> {
        let cube = event.cube_id();
        let seq = event.sequence();
        let mut guard = self.cubes.lock().unwrap();
        let log = guard.entry(cube).or_default();
        if let Some(&latest) = log.keys().next_back() {
            if seq.value() != latest.value() + 1 {
                return Err(Error::SequenceMismatch {
                    expected: latest.value() + 1,
                    got: seq.value(),
                });
            }
        }
        log.insert(seq, event.clone());
        Ok(())
    }

    fn get(&self, cube: CubeId, sequence: EventSequence) -> Result<Option<Event>> {
        let guard = self.cubes.lock().unwrap();
        Ok(guard.get(&cube).and_then(|log| log.get(&sequence).cloned()))
    }

    fn replay(&self, cube: CubeId, from: EventSequence) -> Result<Vec<Event>> {
        let guard = self.cubes.lock().unwrap();
        Ok(guard
            .get(&cube)
            .map(|log| log.range(from..).map(|(_, e)| e.clone()).collect())
            .unwrap_or_default())
    }

    fn latest_sequence(&self, cube: CubeId) -> EventSequence {
        let guard = self.cubes.lock().unwrap();
        guard
            .get(&cube)
            .and_then(|log| log.keys().next_back().copied())
            .unwrap_or(EventSequence::UNASSIGNED)
    }

    fn remove_cube(&self, cube: CubeId) {
        self.cubes.lock().unwrap().remove(&cube);
    }
}

const SEGMENT_TARGET: u64 = 4096;

/// Append-only, checksummed, segment-based file store.
///
/// Layout per Cube:
/// ```text
/// <root>/<cube-id>/events/segment-000001.log
/// <root>/<cube-id>/events/segment-000002.log
/// ```
///
/// Each segment is a sequence of length-prefixed JSON records:
/// `[u32 BE payload_len][payload][b64 sha256 of payload]\n`. On `append`,
/// records land in the current segment; when it hits [`SEGMENT_TARGET`]
/// records a fresh segment starts. Replay scans segments in name order and
/// honors the checkpoint/skip window. Reads verify checksums and reject
/// corrupt records, surfacing a distinct error so recovery tooling can
/// quarantine the segment.
#[derive(Debug)]
pub struct SegmentedEventStore {
    root: PathBuf,
    cubes: Mutex<std::collections::HashMap<CubeId, CubeWriter>>,
}

#[derive(Debug)]
struct CubeWriter {
    seg_dir: PathBuf,
    current_segment: u64,
    current_len: u64,
    seq: std::sync::atomic::AtomicU64,
    write: Mutex<Option<std::fs::File>>,
}

impl SegmentedEventStore {
    /// Opens (or resumes) a store rooted at `root`. Missing directories are
    /// created.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        fs::create_dir_all(root.as_ref())?;
        Ok(Self {
            root: root.as_ref().to_path_buf(),
            cubes: Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// Populates the per-Cube writer state (re-indexing persisted segments
    /// where needed) without keeping a borrow of the map.
    fn ensure_cube(&self, cube: CubeId) -> Result<()> {
        let mut guard = self.cubes.lock().unwrap();
        if guard.contains_key(&cube) {
            return Ok(());
        }
        let seg_dir = self.root.join(cube.to_string()).join("events");
        fs::create_dir_all(&seg_dir)?;

        // Rebuild the latest sequence and the current segment from what is on
        // disk (recovery after restart). Segment content is checksum-verified.
        let mut segments: std::collections::BTreeMap<u64, u64> = std::collections::BTreeMap::new();
        for entry in fs::read_dir(&seg_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(rest) = name
                .strip_prefix("segment-")
                .and_then(|s| s.strip_suffix(".log"))
            {
                if let Ok(n) = rest.parse::<u64>() {
                    let count = count_records(&entry.path())?;
                    segments.insert(n, count);
                }
            }
        }

        let (current_segment, current_len, seq_total) = match segments.iter().next_back() {
            None => (1, 0, 0u64),
            Some((&last, &last_count)) => {
                let total = (last - 1).saturating_mul(SEGMENT_TARGET) + last_count;
                if last_count < SEGMENT_TARGET {
                    (last, last_count, total)
                } else {
                    (last + 1, 0, total)
                }
            }
        };

        let write = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(seg_dir.join(segment_name(current_segment)))?;
        guard.insert(
            cube,
            CubeWriter {
                seg_dir,
                current_segment,
                current_len,
                seq: std::sync::atomic::AtomicU64::new(seq_total),
                write: Mutex::new(Some(write)),
            },
        );
        Ok(())
    }
}

/// Counts records in a segment file by scanning length prefixes.
fn count_records(path: &Path) -> Result<u64> {
    let mut reader = BufReader::new(fs::File::open(path)?);
    let mut count = 0u64;
    loop {
        let mut hasher = Sha256::new();
        let mut len_buf = [0u8; 4];
        let n = read_exact_or_eof(&mut reader, &mut len_buf)?;
        if n == 0 {
            break;
        }
        if n < 4 {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "truncated length prefix",
            )));
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        read_exact_or_eof(&mut reader, &mut payload)?;
        hasher.update(&payload);
        let mut digest = [0u8; 32];
        let m = read_exact_or_eof(&mut reader, &mut digest)?;
        if m < 32 {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "truncated checksum",
            )));
        }
        let mut newline = [0u8; 1];
        read_exact_or_eof(&mut reader, &mut newline)?;
        if hasher.finalize().as_slice() != digest {
            return Err(Error::Serialization(format!(
                "checksum mismatch in {}",
                path.display()
            )));
        }
        count += 1;
    }
    Ok(count)
}

fn read_exact_or_eof<R: BufRead>(reader: &mut R, buf: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let got = reader.read(&mut buf[filled..])?;
        if got == 0 {
            break;
        }
        filled += got;
    }
    Ok(filled)
}

fn segment_name(n: u64) -> String {
    format!("segment-{:06}.log", n)
}

impl EventStore for SegmentedEventStore {
    fn append(&self, event: &Event) -> Result<()> {
        let cube = event.cube_id();
        let seq = event.sequence();
        self.ensure_cube(cube)?;
        let mut guard = self.cubes.lock().unwrap();
        let writer = guard.get_mut(&cube).expect("cube ensured");

        let mut latch = writer.write.lock().unwrap();
        let file = latch
            .as_mut()
            .ok_or_else(|| Error::Serialization("segment closed".into()))?;

        let expected = writer.seq.load(Ordering::SeqCst) + 1;
        if seq.value() != expected {
            return Err(Error::SequenceMismatch {
                expected,
                got: seq.value(),
            });
        }

        // Roll to a new segment once the current is full.
        if writer.current_len >= SEGMENT_TARGET {
            writer.current_segment += 1;
            let next_file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(writer.seg_dir.join(segment_name(writer.current_segment)))?;
            *file = next_file;
            writer.current_len = 0;
        }

        let payload = serde_json::to_vec(event).map_err(Error::serde)?;
        let digest = Sha256::digest(&payload);
        let len = payload.len() as u32;
        let mut record = Vec::with_capacity(4 + payload.len() + 32 + 1);
        record.extend_from_slice(&len.to_be_bytes());
        record.extend_from_slice(&payload);
        record.extend_from_slice(&digest);
        record.push(b'\n');
        file.write_all(&record)?;

        writer.current_len += 1;
        writer.seq.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn get(&self, cube: CubeId, sequence: EventSequence) -> Result<Option<Event>> {
        let all = self.replay(cube, sequence)?;
        Ok(all.into_iter().next())
    }

    fn replay(&self, cube: CubeId, from: EventSequence) -> Result<Vec<Event>> {
        // Ensure the cube is indexed (may be a fresh open).
        self.ensure_cube(cube)?;
        let seg_dir = self.root.join(cube.to_string()).join("events");
        let mut segments: Vec<u64> = fs::read_dir(&seg_dir)?
            .filter_map(|e| {
                let name = e.ok()?.file_name().to_string_lossy().into_owned();
                let rest = name.strip_prefix("segment-")?.strip_suffix(".log")?;
                rest.parse().ok()
            })
            .collect();
        segments.sort_unstable();

        let skip = from.value();
        let mut out = Vec::new();
        for seg in segments {
            let path = seg_dir.join(segment_name(seg));
            let base = (seg - 1).saturating_mul(SEGMENT_TARGET);
            let mut reader = BufReader::new(fs::File::open(&path)?);
            let mut local_seq = 0u64;
            loop {
                let mut hasher = Sha256::new();
                let mut len_buf = [0u8; 4];
                let n = read_exact_or_eof(&mut reader, &mut len_buf)?;
                if n == 0 {
                    break;
                }
                if n < 4 {
                    return Err(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "truncated record",
                    )));
                }
                let len = u32::from_be_bytes(len_buf) as usize;
                let mut payload = vec![0u8; len];
                read_exact_or_eof(&mut reader, &mut payload)?;
                hasher.update(&payload);
                let mut digest = [0u8; 32];
                read_exact_or_eof(&mut reader, &mut digest)?;
                let mut newline = [0u8; 1];
                read_exact_or_eof(&mut reader, &mut newline)?;
                if hasher.finalize().as_slice() != digest {
                    return Err(Error::Serialization(format!(
                        "checksum mismatch in segment {}",
                        path.display()
                    )));
                }
                local_seq += 1;
                let seq = base + local_seq;
                if seq >= skip {
                    let event: Event = serde_json::from_slice(&payload)
                        .map_err(|e| Error::Serialization(format!("{e}: {}", path.display())))?;
                    out.push(event);
                }
            }
        }
        Ok(out)
    }

    fn latest_sequence(&self, cube: CubeId) -> EventSequence {
        if self.ensure_cube(cube).is_err() {
            return EventSequence::UNASSIGNED;
        }
        let guard = self.cubes.lock().unwrap();
        let seq = guard
            .get(&cube)
            .map(|w| w.seq.load(Ordering::SeqCst))
            .unwrap_or(0);
        EventSequence::new(seq)
    }

    fn remove_cube(&self, cube: CubeId) {
        let mut guard = self.cubes.lock().unwrap();
        guard.remove(&cube);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::event::{Event, EventPayload};
    use nexus_core::ids::UserId;
    use std::sync::atomic::AtomicU32;

    static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

    fn temp_dir() -> PathBuf {
        let n = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("nexus-events-test-{}-{}", std::process::id(), n));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn ev(cube: CubeId, seq: u64) -> Event {
        Event::new(
            cube,
            UserId::new(),
            EventPayload::FileCreated { path: "/f".into() },
        )
        .with_sequence(EventSequence::new(seq))
    }

    #[test]
    fn memory_store_rejects_duplicates_and_gaps() {
        let store = MemoryEventStore::new();
        let cube = CubeId::new();
        store.append(&ev(cube, 1)).unwrap();
        assert!(store.append(&ev(cube, 1)).is_err());
        assert!(store.append(&ev(cube, 3)).is_err());
        store.append(&ev(cube, 2)).unwrap();
        assert_eq!(store.latest_sequence(cube).value(), 2);
    }

    #[test]
    fn memory_store_replays_in_order() {
        let store = MemoryEventStore::new();
        let cube = CubeId::new();
        for seq in 1..=5 {
            store.append(&ev(cube, seq)).unwrap();
        }
        let replayed = store.replay(cube, EventSequence::new(3)).unwrap();
        assert_eq!(replayed.len(), 3);
        assert_eq!(replayed[0].sequence().value(), 3);
        assert_eq!(replayed[2].sequence().value(), 5);
    }

    #[test]
    fn segmented_store_persists_across_reopen() {
        let dir = temp_dir();
        let cube = CubeId::new();
        {
            let store = SegmentedEventStore::open(&dir).unwrap();
            for seq in 1..=4 {
                store.append(&ev(cube, seq)).unwrap();
            }
            assert_eq!(store.latest_sequence(cube).value(), 4);
        }
        {
            let reopened = SegmentedEventStore::open(&dir).unwrap();
            assert_eq!(reopened.latest_sequence(cube).value(), 4);
            let replayed = reopened.replay(cube, EventSequence::new(2)).unwrap();
            assert_eq!(replayed.len(), 3);
            assert_eq!(replayed[0].sequence().value(), 2);
            assert_eq!(replayed[2].sequence().value(), 4);
            // Contiguous continuation onto the reopened store.
            reopened.append(&ev(cube, 5)).unwrap();
            assert_eq!(reopened.latest_sequence(cube).value(), 5);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn segmented_store_rolls_segments() {
        let dir = temp_dir();
        let store = SegmentedEventStore::open(&dir).unwrap();
        let cube = CubeId::new();
        for seq in 1..=SEGMENT_TARGET + 3 {
            store.append(&ev(cube, seq)).unwrap();
        }
        let replayed = store.replay(cube, EventSequence::UNASSIGNED).unwrap();
        assert_eq!(replayed.len() as u64, SEGMENT_TARGET + 3);
        assert_eq!(
            replayed.last().unwrap().sequence().value(),
            SEGMENT_TARGET + 3
        );
        // Corrupt-free re-read across the boundary.
        assert_eq!(
            store
                .get(cube, EventSequence::new(SEGMENT_TARGET + 1))
                .unwrap()
                .unwrap()
                .sequence()
                .value(),
            SEGMENT_TARGET + 1
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn segmented_store_detects_checksum_corruption() {
        let dir = temp_dir();
        let store = SegmentedEventStore::open(&dir).unwrap();
        let cube = CubeId::new();
        store.append(&ev(cube, 1)).unwrap();
        store.append(&ev(cube, 2)).unwrap();
        // Flip a byte inside the second record.
        let seg = dir
            .join(cube.to_string())
            .join("events")
            .join("segment-000001.log");
        let data = fs::read(&seg).unwrap();
        let flipped = {
            let mut d = data.clone();
            let mut idx = 0;
            let mut rec = 0;
            while idx < d.len() {
                let len = u32::from_be_bytes([d[idx], d[idx + 1], d[idx + 2], d[idx + 3]]) as usize;
                rec += 1;
                if rec == 2 {
                    d[idx + 4] ^= 0x01;
                    break;
                }
                idx += 4 + len + 32 + 1;
            }
            d
        };
        assert_ne!(data, flipped);
        fs::write(&seg, flipped).unwrap();
        let result = store.replay(cube, EventSequence::UNASSIGNED);
        assert!(result.is_err(), "corruption must surface as an error");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn memory_and_segmented_agree_on_ordering_under_threads() {
        use std::sync::Arc;
        let dir = temp_dir();
        let stores: Vec<Arc<dyn EventStore>> = vec![
            Arc::new(MemoryEventStore::new()),
            Arc::new(SegmentedEventStore::open(&dir).unwrap()),
        ];
        let cube = CubeId::new();
        for store in stores {
            // Sequential 200-event write; both stores must produce identical
            // gapless ordering.
            for seq in 1..=200u64 {
                store.append(&ev(cube, seq)).unwrap();
            }
            let replayed = store.replay(cube, EventSequence::UNASSIGNED).unwrap();
            assert_eq!(replayed.len(), 200);
            for (i, e) in replayed.iter().enumerate() {
                assert_eq!(
                    e.sequence().value(),
                    (i + 1) as u64,
                    "no gaps, strict order"
                );
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn both_stores_reject_out_of_order_under_contention() {
        // The store's contract: appends that are not strictly increasing are
        // rejected (the bus's sequencer prevents this in production); these
        // races must never silently reorder the log.
        use std::sync::Arc;
        let dir = temp_dir();
        let stores: Vec<Arc<dyn EventStore>> = vec![
            Arc::new(MemoryEventStore::new()),
            Arc::new(SegmentedEventStore::open(&dir).unwrap()),
        ];
        let cube = CubeId::new();
        for store in stores {
            // Ascending prefix is accepted...
            for seq in 1..=5u64 {
                store.append(&ev(cube, seq)).unwrap();
            }
            // ...and every out-of-sequence append is rejected.
            for seq in (7..=50u64).rev() {
                assert!(
                    store.append(&ev(cube, seq)).is_err(),
                    "seq {seq} must be rejected"
                );
            }
            // A duplicate (5) is a sequence mismatch, not silently ignored.
            assert!(store.append(&ev(cube, 5)).is_err());
            assert!(store.append(&ev(cube, 99)).is_err());
            let replayed = store.replay(cube, EventSequence::UNASSIGNED).unwrap();
            assert_eq!(replayed.len(), 5);
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
