//! Per-Cube workspace registry, path validation and workspace-aware storage.

pub mod manager;
pub mod paths;

pub use manager::{CubeWorkspace, WorkspaceManager};
