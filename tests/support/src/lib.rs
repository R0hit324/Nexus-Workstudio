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
//!
//! Nothing here touches the user's real files, credentials, or environment
//! (spec section 85).

#![cfg_attr(not(test), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use nexus_core::event::{Event, EventPayload};
use nexus_core::ids::{CubeId, UserId};
use nexus_core::timestamp::Timestamp;

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
}
