//! Workspace model crate.
//!
//! Part 1 reserves this crate and the foundational vocabulary lives in
//! `nexus-core::workspace`. The full workspace model, conflict handling, and
//! state synchronization arrive in Part 2.

/// Reserved crate marker. Workspace synchronization is implemented in Part 2.
pub const PHASE: &str = "part1-placeholder";

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_is_static() {
        assert_eq!(super::PHASE, "part1-placeholder");
    }
}
