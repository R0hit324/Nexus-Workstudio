//! Deterministic fixtures and isolated environments for Nexus test suites
//! (Part 14).
//!
//! Implements the reusable-test-setup rules from the Part 14 specification:
//!
//! - **Fixtures** (spec section 5): deterministic users, cubes, timestamps,
//!   and events so setup logic is not duplicated across tests.
//! - **Temporary environments** (spec section 6): every test can own an
//!   isolated directory that is removed when it goes out of scope.
//! - **Deterministic randomness** (spec section 73): an injectable seeded
//!   generator for reproducible test data.
//! - **Failure injection** (spec section 71): configurable fault injectors
//!   and an [`EventStore`] wrapper for deliberate storage failures.
//!
//! Nothing here touches the user's real files, credentials, or environment
//! (spec section 85).

#![cfg_attr(not(test), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use nexus_core::event::{Event, EventPayload};
use nexus_core::ids::{CubeId, UserId};
use nexus_core::timestamp::Timestamp;
use nexus_events::store::EventStore;
use nexus_events::Error;

static ENV_COUNTER: AtomicU64 = AtomicU64::new(1);

/// An isolated, self-cleaning temporary directory for a single test.
///
/// The directory lives under the OS temp root and is removed when the value
/// is dropped, so tests never leave state behind (spec section 6).
#[derive(Debug)]
pub struct TempEnv {
    root: PathBuf,
}

impl TempEnv {
    /// Creates a fresh environment directory labelled `label`.
    ///
    /// # Panics
    /// Panics if the directory cannot be created.
    #[must_use]
    pub fn new(label: &str) -> Self {
        let n = ENV_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("nexus-test-{label}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create temporary test environment");
        Self { root }
    }

    /// The environment root.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// A path inside the environment.
    #[must_use]
    pub fn join(&self, relative: impl AsRef<Path>) -> PathBuf {
        self.root.join(relative)
    }
}

impl Drop for TempEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A deterministic pseudo-random source for reproducible test data.
///
/// Implements xorshift64*, which is adequate for fixtures and keeps tests
/// reproducible from a fixed seed (spec section 73).
#[derive(Debug, Clone)]
pub struct TestRng {
    state: u64,
}

impl TestRng {
    /// Creates a generator from `seed`. A zero seed is normalized.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    /// The next raw value.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// The next value below `upper` (`0` when `upper` is zero).
    pub fn next_bounded(&mut self, upper: u64) -> u64 {
        if upper == 0 {
            0
        } else {
            self.next_u64() % upper
        }
    }

    /// Fills `buffer` with deterministic bytes.
    pub fn fill(&mut self, buffer: &mut [u8]) {
        for chunk in buffer.chunks_mut(8) {
            let bytes = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
    }
}

/// A deterministic cube id fixture.
#[must_use]
pub fn cube_id(n: u64) -> CubeId {
    CubeId::from_u64(n)
}

/// A deterministic user id fixture.
#[must_use]
pub fn user_id(n: u64) -> UserId {
    UserId::from_u64(n)
}

/// A deterministic timestamp fixture (milliseconds since the epoch).
#[must_use]
pub fn timestamp(millis: u64) -> Timestamp {
    Timestamp::from_millis(millis)
}

/// A deterministic durable file-created event fixture.
#[must_use]
pub fn file_created(cube: CubeId, actor: UserId, path: &str) -> Event {
    Event::new(cube, actor, EventPayload::FileCreated { path: path.into() })
}

/// A deterministic transient cursor-moved event fixture.
#[must_use]
pub fn cursor_moved(cube: CubeId, actor: UserId, path: &str, line: u64, column: u64) -> Event {
    Event::new(
        cube,
        actor,
        EventPayload::CursorMoved {
            path: path.into(),
            line,
            column,
        },
    )
}

/// A deterministic chat message event fixture.
#[must_use]
pub fn chat_message(cube: CubeId, actor: UserId, text: &str) -> Event {
    Event::new(
        cube,
        actor,
        EventPayload::ChatMessageSent { text: text.into() },
    )
}

/// Configurable failure injection for test doubles (spec section 71).
///
/// Tests arm the injector to fail the next `N` calls or every `N`-th call,
/// then verify that the system under test degrades gracefully instead of
/// crashing or corrupting state.
#[derive(Debug, Default)]
pub struct FaultInjector {
    calls: AtomicU64,
    fail_next: AtomicU64,
    fail_every: AtomicU64,
}

impl FaultInjector {
    /// Creates a disarmed injector.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Arms the injector to fail the next `count` calls.
    pub fn fail_next(&self, count: u64) {
        self.fail_next.store(count, Ordering::SeqCst);
    }

    /// Arms the injector to fail every `every`-th call (`0` disables).
    pub fn fail_every(&self, every: u64) {
        self.fail_every.store(every, Ordering::SeqCst);
    }

    /// Disarms all injection.
    pub fn disarm(&self) {
        self.fail_next.store(0, Ordering::SeqCst);
        self.fail_every.store(0, Ordering::SeqCst);
    }

    /// Records one call and reports whether it should fail.
    pub fn should_fail(&self) -> bool {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_next.load(Ordering::SeqCst) > 0 {
            self.fail_next.fetch_sub(1, Ordering::SeqCst);
            return true;
        }
        let every = self.fail_every.load(Ordering::SeqCst);
        every > 0 && call % every == 0
    }

    /// The number of calls observed so far.
    #[must_use]
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }
}

/// An [`EventStore`] wrapper that injects append failures (spec section 71).
///
/// Every other operation delegates to the wrapped store, so tests can keep
/// using a real store while making selected appends fail.
#[derive(Debug)]
pub struct FaultyEventStore {
    inner: Arc<dyn EventStore>,
    faults: Arc<FaultInjector>,
}

impl FaultyEventStore {
    /// Wraps `inner`, injecting failures according to `faults`.
    #[must_use]
    pub fn new(inner: Arc<dyn EventStore>, faults: Arc<FaultInjector>) -> Self {
        Self { inner, faults }
    }

    /// The wrapped store.
    #[must_use]
    pub fn inner(&self) -> &Arc<dyn EventStore> {
        &self.inner
    }
}

impl EventStore for FaultyEventStore {
    fn append(&self, event: &Event) -> std::result::Result<(), Error> {
        if self.faults.should_fail() {
            return Err(Error::Serialization("injected store failure".into()));
        }
        self.inner.append(event)
    }

    fn get(
        &self,
        cube: CubeId,
        sequence: nexus_core::event::EventSequence,
    ) -> std::result::Result<Option<Event>, Error> {
        self.inner.get(cube, sequence)
    }

    fn replay(
        &self,
        cube: CubeId,
        from: nexus_core::event::EventSequence,
    ) -> std::result::Result<Vec<Event>, Error> {
        self.inner.replay(cube, from)
    }

    fn latest_sequence(&self, cube: CubeId) -> nexus_core::event::EventSequence {
        self.inner.latest_sequence(cube)
    }

    fn remove_cube(&self, cube: CubeId) {
        self.inner.remove_cube(cube);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_env_creates_and_cleans_its_directory() {
        let path;
        {
            let env = TempEnv::new("cleanup");
            path = env.path().to_path_buf();
            assert!(path.is_dir());
            std::fs::write(env.join("file.txt"), b"x").unwrap();
            assert!(env.join("file.txt").is_file());
        }
        assert!(!path.exists());
    }

    #[test]
    fn rng_is_reproducible_from_a_seed() {
        let mut a = TestRng::new(42);
        let mut b = TestRng::new(42);
        for _ in 0..16 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn bounded_values_stay_in_range() {
        let mut rng = TestRng::new(7);
        for _ in 0..100 {
            assert!(rng.next_bounded(10) < 10);
        }
        assert_eq!(rng.next_bounded(0), 0);
    }

    #[test]
    fn fixtures_are_deterministic() {
        let cube = cube_id(7);
        let user = user_id(9);
        let first = file_created(cube, user, "src/main.rs");
        let second = file_created(cube, user, "src/main.rs");
        assert_eq!(first.cube_id(), cube);
        assert_eq!(first.actor(), user);
        assert_eq!(first.payload(), second.payload());
    }

    #[test]
    fn injector_fails_the_armed_number_of_calls() {
        let faults = FaultInjector::new();
        faults.fail_next(2);
        assert!(faults.should_fail());
        assert!(faults.should_fail());
        assert!(!faults.should_fail());
        assert_eq!(faults.calls(), 3);
    }

    #[test]
    fn injector_can_fail_every_nth_call() {
        let faults = FaultInjector::new();
        faults.fail_every(2);
        assert!(!faults.should_fail());
        assert!(faults.should_fail());
        assert!(!faults.should_fail());
        assert!(faults.should_fail());
        faults.disarm();
        assert!(!faults.should_fail());
    }

    #[test]
    fn faulty_store_delegates_and_injects_failures() {
        use nexus_core::event::EventSequence;
        use nexus_events::store::MemoryEventStore;

        let faults = Arc::new(FaultInjector::new());
        let inner: Arc<dyn EventStore> = Arc::new(MemoryEventStore::new());
        let store = FaultyEventStore::new(inner, faults.clone());
        let cube = cube_id(1);
        let user = user_id(1);

        let first = file_created(cube, user, "a.rs").with_sequence(EventSequence::new(1));
        assert!(store.append(&first).is_ok());

        faults.fail_next(1);
        let second = file_created(cube, user, "b.rs").with_sequence(EventSequence::new(2));
        assert!(store.append(&second).is_err());

        // The failed append must not have consumed the sequence.
        assert!(store.append(&second).is_ok());
        assert_eq!(
            store.replay(cube, EventSequence::UNASSIGNED).unwrap().len(),
            2
        );
    }
}
