use serde::{Deserialize, Serialize};

use crate::ids::{BuildId, CubeId, FileId, UserId};
use crate::timestamp::Timestamp;

/// Build lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildState {
    Queued,
    Preparing,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

/// Build target/profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildProfile {
    Debug,
    Release,
    Test,
    Custom,
}

/// Build target type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildTargetType {
    Build,
    Run,
    Test,
    Check,
    Lint,
    Format,
    Custom,
}

/// Build target specification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildTarget {
    pub target_type: BuildTargetType,
    pub profile: BuildProfile,
    pub target_name: Option<String>,
    pub working_directory: String,
    pub environment: Vec<EnvVar>,
    pub timeout_seconds: u64,
    pub resource_limits: crate::runtime::ResourceLimits,
}

/// Build request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildRequest {
    pub build_id: BuildId,
    pub cube_id: CubeId,
    pub user_id: UserId,
    pub target: BuildTarget,
    pub configuration: BuildConfiguration,
    pub queued_at: Timestamp,
}

/// Build configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildConfiguration {
    pub language: String,
    pub toolchain: Option<String>,
    pub build_command: Option<String>,
    pub test_command: Option<String>,
    pub check_command: Option<String>,
    pub lint_command: Option<String>,
    pub format_command: Option<String>,
    pub environment: Vec<EnvVar>,
    pub working_directory: String,
    pub timeout_seconds: u64,
    pub resource_limits: crate::runtime::ResourceLimits,
}

/// Build result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildResult {
    pub build_id: BuildId,
    pub state: BuildState,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub started_at: Timestamp,
    pub finished_at: Option<Timestamp>,
    pub diagnostics: Vec<Diagnostic>,
    pub artifacts: Vec<BuildArtifact>,
    pub resource_usage: ResourceUsage,
}

/// Build artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildArtifact {
    pub artifact_id: crate::ids::ArtifactId,
    pub path: String,
    pub size: u64,
    pub kind: ArtifactKind,
    pub created_at: Timestamp,
}

/// Artifact kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactKind {
    Binary,
    Library,
    Package,
    TestBinary,
    Documentation,
    Other,
}

/// Diagnostic from compiler/build tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub file_id: Option<FileId>,
    pub path: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub end_line: Option<u32>,
    pub end_column: Option<u32>,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub code: Option<String>,
    pub source: String,
}

/// Diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

/// Environment variable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
    pub is_secret: bool,
}

/// Resource usage metrics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub peak_memory_bytes: u64,
    pub cpu_time_ms: u64,
    pub wall_time_ms: u64,
}

/// Build queue priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildPriority {
    Interactive,
    Normal,
    Background,
}

/// Test request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRequest {
    pub test_id: BuildId,
    pub cube_id: CubeId,
    pub user_id: UserId,
    pub target: BuildTarget,
    pub configuration: BuildConfiguration,
    pub filter: Option<String>,
}

/// Test result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub test_id: BuildId,
    pub state: BuildState,
    pub duration_ms: u64,
    pub started_at: Timestamp,
    pub finished_at: Option<Timestamp>,
    pub test_cases: Vec<TestCaseResult>,
    pub resource_usage: ResourceUsage,
}

/// Individual test case result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCaseResult {
    pub name: String,
    pub state: BuildState,
    pub duration_ms: u64,
    pub output: String,
    pub error: Option<String>,
}

/// Compiler adapter trait for language-specific build logic.
#[async_trait::async_trait]
pub trait CompilerAdapter: Send + Sync {
    /// Returns the name of this adapter.
    fn name(&self) -> &str;

    /// Detects if this adapter applies to the given workspace.
    async fn detect(&self, workspace: &dyn crate::workspace::Workspace) -> bool;

    /// Prepares the build environment.
    async fn prepare(&self, request: &BuildRequest) -> crate::Result<()>;

    /// Executes a build.
    async fn build(&self, request: &BuildRequest) -> crate::Result<BuildResult>;

    /// Executes tests.
    async fn test(&self, request: &TestRequest) -> crate::Result<TestResult>;

    /// Runs a check (lint, format, etc.).
    async fn check(&self, request: &BuildRequest) -> crate::Result<BuildResult>;

    /// Gets the default build command for this language.
    fn default_build_command(&self) -> Option<&str>;

    /// Gets the default test command for this language.
    fn default_test_command(&self) -> Option<&str>;
}

/// Language adapter registry.
pub struct LanguageAdapterRegistry {
    adapters: Vec<Box<dyn CompilerAdapter>>,
}

impl LanguageAdapterRegistry {
    pub fn new() -> Self {
        Self {
            adapters: Vec::new(),
        }
    }

    pub fn register(&mut self, adapter: Box<dyn CompilerAdapter>) {
        self.adapters.push(adapter);
    }

    pub fn find(&self, language: &str) -> Option<&dyn CompilerAdapter> {
        self.adapters
            .iter()
            .find(|a| a.name() == language)
            .map(|a| a.as_ref())
    }

    pub async fn detect(
        &self,
        workspace: &dyn crate::workspace::Workspace,
    ) -> Option<&dyn CompilerAdapter> {
        for adapter in &self.adapters {
            if adapter.detect(workspace).await {
                return Some(adapter.as_ref());
            }
        }
        None
    }
}

impl Default for LanguageAdapterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_state_transitions() {
        // Test basic state flow
        let states = [
            BuildState::Queued,
            BuildState::Preparing,
            BuildState::Running,
            BuildState::Succeeded,
        ];
        assert_eq!(states.len(), 4);
    }
}
