use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};

/// Authoritative server lifecycle. Crossed only monotonically in the forward
/// direction; `Failed` and `Stopped` are terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ServerLifecycle {
    Starting,
    Initializing,
    Listening,
    Running,
    Stopping,
    Stopped,
    Failed,
}

const _STARTING: u8 = ServerLifecycle::Starting as u8;
const _INITIALIZING: u8 = ServerLifecycle::Initializing as u8;
const _LISTENING: u8 = ServerLifecycle::Listening as u8;
const _RUNNING: u8 = ServerLifecycle::Running as u8;
const _STOPPING: u8 = ServerLifecycle::Stopping as u8;
const _STOPPED: u8 = ServerLifecycle::Stopped as u8;
const _FAILED: u8 = ServerLifecycle::Failed as u8;

impl ServerLifecycle {
    /// Permitted forward transitions.
    ///
    /// - `Starting -> Initializing -> Listening -> Running`
    /// - `Running -> Stopping -> Stopped`
    /// - any active state -> `Failed` (on unrecoverable error)
    ///
    /// `Failed` and `Stopped` are terminal; nothing can follow them.
    #[must_use]
    pub fn can_transition_to(self, to: Self) -> bool {
        matches!(
            (self, to),
            (Self::Starting, Self::Initializing)
                | (Self::Initializing, Self::Listening)
                | (Self::Listening, Self::Running)
                | (Self::Listening, Self::Failed)
                | (Self::Running, Self::Stopping)
                | (Self::Running, Self::Failed)
                | (Self::Initializing, Self::Failed)
                | (Self::Starting, Self::Failed)
                | (Self::Stopping, Self::Stopped)
                | (Self::Stopping, Self::Failed)
        )
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Initializing => "initializing",
            Self::Listening => "listening",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

impl fmt::Display for ServerLifecycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug)]
pub struct LifecycleHandle {
    state: AtomicU8,
}

impl Default for LifecycleHandle {
    fn default() -> Self {
        Self::new_booting()
    }
}

impl LifecycleHandle {
    /// Starts in [`ServerLifecycle::Starting`].
    pub fn new_booting() -> Self {
        Self {
            state: AtomicU8::new(ServerLifecycle::Starting as u8),
        }
    }

    /// Attempts the forward transition to `to`. No-op when not permitted.
    #[must_use]
    pub fn transition_to(&self, to: ServerLifecycle) -> bool {
        let current = self.state();
        if !current.can_transition_to(to) {
            return false;
        }
        self.state.store(to as u8, Ordering::SeqCst);
        true
    }

    #[must_use]
    pub fn state(&self) -> ServerLifecycle {
        let value = self.state.load(Ordering::SeqCst);
        debug_assert!(value <= ServerLifecycle::Failed as u8);
        if value <= ServerLifecycle::Failed as u8 {
            // SAFETY: every allowed `u8` maps to a defined `repr(u8)` variant.
            unsafe { std::mem::transmute::<u8, ServerLifecycle>(value) }
        } else {
            ServerLifecycle::Failed
        }
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(
            self.state(),
            ServerLifecycle::Starting
                | ServerLifecycle::Initializing
                | ServerLifecycle::Listening
                | ServerLifecycle::Running
        )
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.state() == ServerLifecycle::Running
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_transitions_only() {
        let lifecycle = LifecycleHandle::new_booting();
        assert_eq!(lifecycle.state(), ServerLifecycle::Starting);
        assert!(lifecycle.transition_to(ServerLifecycle::Initializing));
        assert!(lifecycle.transition_to(ServerLifecycle::Listening));
        assert!(lifecycle.transition_to(ServerLifecycle::Running));
        assert!(lifecycle.transition_to(ServerLifecycle::Stopping));
        assert!(lifecycle.transition_to(ServerLifecycle::Stopped));
        assert_eq!(lifecycle.state(), ServerLifecycle::Stopped);
        assert!(!lifecycle.transition_to(ServerLifecycle::Running));
    }

    #[test]
    fn skips_are_rejected() {
        let lifecycle = LifecycleHandle::new_booting();
        assert!(
            !lifecycle.transition_to(ServerLifecycle::Running),
            "cannot skip initializing"
        );
        assert!(
            !lifecycle.transition_to(ServerLifecycle::Stopped),
            "cannot skip to stopped"
        );
    }

    #[test]
    fn failure_is_terminal() {
        let lifecycle = LifecycleHandle::new_booting();
        assert!(lifecycle.transition_to(ServerLifecycle::Failed));
        assert!(!lifecycle.transition_to(ServerLifecycle::Running));
        assert!(!lifecycle.transition_to(ServerLifecycle::Stopped));
        assert!(!lifecycle.is_running());
    }
}
