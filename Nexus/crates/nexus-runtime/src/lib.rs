//! Runtime orchestration crate.
//!
//! Part 9 implements isolated Cube runtimes: lifecycle management, Docker backend,
//! resource limits, port allocation, process orchestration, and workspace mounting.

pub mod backend;
pub mod events;
pub mod manager;
pub mod ports;
pub mod processes;
pub mod watcher;

pub use backend::{
    BackendFactory, DockerBackend, DockerConfig, LocalProcessBackend, LocalProcessConfig,
    RuntimeBackend,
};
pub use events::{FileChangeKind, HealthEvent, ProcessEvent, RuntimeEvent, WorkspaceChangeEvent};
pub use manager::{RuntimeManager, RuntimeManagerConfig};
pub use ports::PortAllocator;
pub use processes::{ProcessManager, ProcessManagerConfig, TrackedProcess};
pub use watcher::{FileChange, WatcherConfig, WorkspaceWatcher};

/// Phase marker for version detection.
pub const PHASE: &str = "part9-runtime";

#[cfg(test)]
mod tests {
    #[test]
    fn phase_is_correct() {
        assert_eq!(super::PHASE, "part9-runtime");
    }
}
