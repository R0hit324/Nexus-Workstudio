use serde::{Deserialize, Serialize};

/// A monotonic wall-clock timestamp expressed as milliseconds since the Unix
/// epoch.
///
/// A value type (rather than `std::time::Instant`) so timestamps can be
/// serialized, persisted, ordered, and shared over the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Timestamp(u64);

impl Timestamp {
    /// Returns the current wall-clock time in milliseconds since the epoch.
    #[must_use]
    pub fn now() -> Self {
        Self(current_time_ms())
    }

    /// Constructs a timestamp from milliseconds since the epoch.
    #[must_use]
    pub const fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    /// Returns the underlying milliseconds value.
    #[must_use]
    pub const fn as_millis(self) -> u64 {
        self.0
    }

    /// Returns seconds elapsed between this timestamp and another, saturating
    /// at zero.
    #[must_use]
    pub fn seconds_until(self, later: Timestamp) -> u64 {
        later.0.saturating_sub(self.0) / 1000
    }
}

impl std::fmt::Display for Timestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Returns the current wall-clock time in milliseconds since the Unix epoch.
#[must_use]
pub fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_after_epoch() {
        assert!(Timestamp::now().as_millis() > 1_000_000);
    }

    #[test]
    fn ordering_matches_milliseconds() {
        let early = Timestamp::from_millis(100);
        let late = Timestamp::from_millis(200);
        assert!(early < late);
        assert_eq!(early.seconds_until(late), 0);
        assert_eq!(
            Timestamp::from_millis(0).seconds_until(Timestamp::from_millis(5000)),
            5
        );
    }

    #[test]
    fn serializes_through_json() {
        let ts = Timestamp::from_millis(42);
        let encoded = serde_json::to_string(&ts).unwrap();
        let decoded: Timestamp = serde_json::from_str(&encoded).unwrap();
        assert_eq!(ts, decoded);
    }
}
