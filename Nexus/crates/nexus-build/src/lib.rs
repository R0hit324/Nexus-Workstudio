//! Build system crate.
//!
//! Part 10 implements the build system: build queue, compiler adapters,
//! diagnostics parsing, artifact tracking, and build execution.

pub mod adapters;
pub mod artifacts;
pub mod diagnostics;
pub mod manager;
pub mod queue;

pub use adapters::{
    CppAdapter, GoAdapter, JavaAdapter, JavaScriptAdapter, PythonAdapter, RustAdapter,
};
pub use artifacts::ArtifactManager;
pub use diagnostics::{DiagnosticParser, RustDiagnosticParser};
pub use manager::{BuildEvent, BuildManager, BuildManagerConfig};
pub use queue::{BuildQueue, BuildQueueConfig, BuildQueueEntry};

// Re-exports of core build types so downstream crates can use them together.
pub use nexus_core::{
    ArtifactKind, BuildArtifact, BuildConfiguration, BuildPriority, BuildProfile, BuildRequest,
    BuildResult, BuildState, BuildTarget, BuildTargetType, CompilerAdapter, Diagnostic,
    DiagnosticSeverity, LanguageAdapterRegistry, ResourceUsage, TestRequest, TestResult,
};

/// Phase marker for version detection.
pub const PHASE: &str = "part10-build";

#[cfg(test)]
mod tests {
    #[test]
    fn phase_is_correct() {
        assert_eq!(super::PHASE, "part10-build");
    }
}
