//! Compiler adapters for different languages.
//!
//! Implements the `CompilerAdapter` trait defined in `nexus-core`, reusing the
//! core `LanguageAdapterRegistry` rather than redefining it.

use std::time::Duration;

use async_trait::async_trait;

use nexus_core::workspace::Workspace;
use nexus_core::{
    BuildRequest, BuildResult, BuildState, CompilerAdapter, Diagnostic, EnvVar, Error, ErrorCode,
    ResourceUsage, Result, TestCaseResult, TestRequest, TestResult,
};

use crate::diagnostics::{try_parse_generic_line, try_parse_rust_line};

/// A captured process output.
struct CommandOutput {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
}

/// Zero-valued resource usage placeholder (adapters do not profile).
fn zero_usage() -> ResourceUsage {
    ResourceUsage {
        peak_memory_bytes: 0,
        cpu_time_ms: 0,
        wall_time_ms: 0,
    }
}

/// Runs a command with environment overrides and an optional timeout.
async fn run_command(
    program: &str,
    args: &[&str],
    cwd: &str,
    env: &[EnvVar],
    timeout_seconds: u64,
) -> Result<CommandOutput> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args);
    cmd.current_dir(cwd);
    for var in env {
        cmd.env(&var.key, &var.value);
    }

    let timeout = if timeout_seconds > 0 {
        Duration::from_secs(timeout_seconds)
    } else {
        Duration::from_secs(600)
    };

    match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(output) => {
            let output = output.map_err(|e| {
                Error::new(ErrorCode::Build, "failed to spawn build command").with_source(e)
            })?;
            Ok(CommandOutput {
                exit_code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                timed_out: false,
            })
        }
        Err(_) => Ok(CommandOutput {
            exit_code: None,
            stdout: String::new(),
            stderr: String::from("build timed out"),
            timed_out: true,
        }),
    }
}

/// Assembles a [`BuildResult`] from captured output.
fn finish_build(
    build_id: nexus_core::BuildId,
    language: &str,
    output: &CommandOutput,
) -> BuildResult {
    let state = if output.timed_out {
        BuildState::TimedOut
    } else if output.exit_code == Some(0) {
        BuildState::Succeeded
    } else {
        BuildState::Failed
    };

    let parser_output: Vec<Diagnostic> = if language == "rust" {
        output
            .stderr
            .lines()
            .chain(output.stdout.lines())
            .filter_map(try_parse_rust_line)
            .collect()
    } else {
        output
            .stderr
            .lines()
            .chain(output.stdout.lines())
            .filter_map(try_parse_generic_line)
            .collect()
    };

    BuildResult {
        build_id,
        state,
        exit_code: output.exit_code,
        duration_ms: 0,
        started_at: nexus_core::Timestamp::now(),
        finished_at: Some(nexus_core::Timestamp::now()),
        diagnostics: parser_output,
        artifacts: Vec::new(),
        resource_usage: zero_usage(),
    }
}

/// Runs the requested command kind for a language, resolving the command from
/// the configuration with a language-appropriate fallback.
async fn run_language_command(
    request: &BuildRequest,
    resolved: Option<&str>,
    fallback: &str,
) -> Result<BuildResult> {
    let cmd = resolved.unwrap_or(fallback);

    // Split "program extra args" naively on whitespace. Toolchains like
    // `cargo build --release` map cleanly; complex quoting is out of scope.
    let mut parts = cmd.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "empty build command"))?;
    let args: Vec<&str> = parts.collect();

    let cwd = if request.target.working_directory.is_empty() {
        request.configuration.working_directory.as_str()
    } else {
        request.target.working_directory.as_str()
    };

    let env: Vec<EnvVar> = request
        .target
        .environment
        .iter()
        .chain(request.configuration.environment.iter())
        .cloned()
        .collect();

    let output = run_command(program, &args, cwd, &env, request.target.timeout_seconds).await?;
    Ok(finish_build(
        request.build_id,
        request.configuration.language.as_str(),
        &output,
    ))
}

/// Runs a test command for a [`TestRequest`], handling the same resolution
/// logic as [`run_language_command`].
async fn run_test_command(
    request: &TestRequest,
    resolved: Option<&str>,
    fallback: &str,
) -> Result<TestResult> {
    let cmd = resolved.unwrap_or(fallback);

    let mut parts = cmd.split_whitespace();
    let program = parts
        .next()
        .ok_or_else(|| Error::new(ErrorCode::InvalidRequest, "empty test command"))?;
    let args: Vec<&str> = parts.collect();

    let cwd = if request.target.working_directory.is_empty() {
        request.configuration.working_directory.as_str()
    } else {
        request.target.working_directory.as_str()
    };

    let env: Vec<EnvVar> = request
        .target
        .environment
        .iter()
        .chain(request.configuration.environment.iter())
        .cloned()
        .collect();

    let output = run_command(program, &args, cwd, &env, request.target.timeout_seconds).await?;
    Ok(finish_test(request.test_id, &output))
}

fn default_build_command(kind: CommandKind, language: &str) -> &'static str {
    match (kind, language) {
        (CommandKind::Build, "rust") | (CommandKind::Test, "rust") => "cargo build",
        (CommandKind::Check, "rust") => "cargo check",
        (CommandKind::Build, "javascript") => "npm run build",
        (CommandKind::Test, "javascript") => "npm test",
        (CommandKind::Check, "javascript") => "npm run lint",
        (CommandKind::Build, "python") => "python -m pip install -e .",
        (CommandKind::Test, "python") => "python -m pytest",
        (CommandKind::Check, "python") => "python -m flake8",
        (CommandKind::Build, "go") => "go build ./...",
        (CommandKind::Test, "go") => "go test ./...",
        (CommandKind::Check, "go") => "go vet ./...",
        (CommandKind::Build, "java") => "mvn package",
        (CommandKind::Test, "java") => "mvn test",
        (CommandKind::Check, "java") => "mvn verify",
        (CommandKind::Build, "cpp") => "cmake --build build",
        (CommandKind::Test, "cpp") => "ctest",
        _ => "make",
    }
}

/// Command kinds used for language-appropriate defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandKind {
    Build,
    #[allow(dead_code)]
    Test,
    Check,
}

/// Builds a generic check-style test result from a command output.
fn finish_test(test_id: nexus_core::BuildId, output: &CommandOutput) -> TestResult {
    let state = if output.timed_out {
        BuildState::TimedOut
    } else if output.exit_code == Some(0) {
        BuildState::Succeeded
    } else {
        BuildState::Failed
    };

    // Parse test case names from a few common output styles.
    let test_cases: Vec<TestCaseResult> = output
        .stdout
        .lines()
        .filter_map(|line| {
            if line.starts_with("test ") {
                let rest = line.trim_start_matches("test ");
                let (name, outcome) = match rest.split_once("... ") {
                    Some((n, o)) => (n, o),
                    None => (rest, "ok"),
                };
                Some(TestCaseResult {
                    name: name.to_string(),
                    state: if outcome.starts_with("ok") {
                        BuildState::Succeeded
                    } else {
                        BuildState::Failed
                    },
                    duration_ms: 0,
                    output: line.to_string(),
                    error: None,
                })
            } else {
                None
            }
        })
        .collect();

    TestResult {
        test_id,
        state,
        duration_ms: 0,
        started_at: nexus_core::Timestamp::now(),
        finished_at: Some(nexus_core::Timestamp::now()),
        test_cases,
        resource_usage: zero_usage(),
    }
}

// ---------------------------------------------------------------------------
// Adapters
// ---------------------------------------------------------------------------

/// Rust adapter using cargo.
pub struct RustAdapter;

#[async_trait]
impl CompilerAdapter for RustAdapter {
    fn name(&self) -> &str {
        "rust"
    }

    async fn detect(&self, workspace: &dyn Workspace) -> bool {
        let files = workspace.list_files().await.unwrap_or_default();
        files.iter().any(|f| f.path.ends_with("Cargo.toml"))
    }

    async fn prepare(&self, _request: &BuildRequest) -> Result<()> {
        Ok(())
    }

    async fn build(&self, request: &BuildRequest) -> Result<BuildResult> {
        run_language_command(
            request,
            request.configuration.build_command.as_deref(),
            default_build_command(CommandKind::Build, "rust"),
        )
        .await
    }

    async fn test(&self, request: &TestRequest) -> Result<TestResult> {
        run_test_command(
            request,
            request.configuration.test_command.as_deref(),
            "cargo test",
        )
        .await
    }

    async fn check(&self, request: &BuildRequest) -> Result<BuildResult> {
        run_language_command(
            request,
            request.configuration.check_command.as_deref(),
            default_build_command(CommandKind::Check, "rust"),
        )
        .await
    }

    fn default_build_command(&self) -> Option<&str> {
        Some("cargo build")
    }

    fn default_test_command(&self) -> Option<&str> {
        Some("cargo test")
    }
}

/// JavaScript/TypeScript adapter using npm.
pub struct JavaScriptAdapter;

#[async_trait]
impl CompilerAdapter for JavaScriptAdapter {
    fn name(&self) -> &str {
        "javascript"
    }

    async fn detect(&self, workspace: &dyn Workspace) -> bool {
        let files = workspace.list_files().await.unwrap_or_default();
        files.iter().any(|f| f.path.ends_with("package.json"))
    }

    async fn prepare(&self, _request: &BuildRequest) -> Result<()> {
        Ok(())
    }

    async fn build(&self, request: &BuildRequest) -> Result<BuildResult> {
        run_language_command(
            request,
            request.configuration.build_command.as_deref(),
            default_build_command(CommandKind::Build, "javascript"),
        )
        .await
    }

    async fn test(&self, request: &TestRequest) -> Result<TestResult> {
        run_test_command(
            request,
            request.configuration.test_command.as_deref(),
            "npm test",
        )
        .await
    }

    async fn check(&self, request: &BuildRequest) -> Result<BuildResult> {
        run_language_command(
            request,
            request.configuration.check_command.as_deref(),
            default_build_command(CommandKind::Check, "javascript"),
        )
        .await
    }

    fn default_build_command(&self) -> Option<&str> {
        Some("npm run build")
    }

    fn default_test_command(&self) -> Option<&str> {
        Some("npm test")
    }
}

/// Python adapter.
pub struct PythonAdapter;

#[async_trait]
impl CompilerAdapter for PythonAdapter {
    fn name(&self) -> &str {
        "python"
    }

    async fn detect(&self, workspace: &dyn Workspace) -> bool {
        let files = workspace.list_files().await.unwrap_or_default();
        files
            .iter()
            .any(|f| f.path.ends_with("pyproject.toml") || f.path.ends_with("requirements.txt"))
    }

    async fn prepare(&self, _request: &BuildRequest) -> Result<()> {
        Ok(())
    }

    async fn build(&self, request: &BuildRequest) -> Result<BuildResult> {
        run_language_command(
            request,
            request.configuration.build_command.as_deref(),
            default_build_command(CommandKind::Build, "python"),
        )
        .await
    }

    async fn test(&self, request: &TestRequest) -> Result<TestResult> {
        run_test_command(
            request,
            request.configuration.test_command.as_deref(),
            "python -m pytest",
        )
        .await
    }

    async fn check(&self, request: &BuildRequest) -> Result<BuildResult> {
        run_language_command(
            request,
            request.configuration.check_command.as_deref(),
            default_build_command(CommandKind::Check, "python"),
        )
        .await
    }

    fn default_build_command(&self) -> Option<&str> {
        Some("python -m pip install -e .")
    }

    fn default_test_command(&self) -> Option<&str> {
        Some("python -m pytest")
    }
}

/// Go adapter.
pub struct GoAdapter;

#[async_trait]
impl CompilerAdapter for GoAdapter {
    fn name(&self) -> &str {
        "go"
    }

    async fn detect(&self, workspace: &dyn Workspace) -> bool {
        let files = workspace.list_files().await.unwrap_or_default();
        files.iter().any(|f| f.path.ends_with("go.mod"))
    }

    async fn prepare(&self, _request: &BuildRequest) -> Result<()> {
        Ok(())
    }

    async fn build(&self, request: &BuildRequest) -> Result<BuildResult> {
        run_language_command(
            request,
            request.configuration.build_command.as_deref(),
            default_build_command(CommandKind::Build, "go"),
        )
        .await
    }

    async fn test(&self, request: &TestRequest) -> Result<TestResult> {
        run_test_command(
            request,
            request.configuration.test_command.as_deref(),
            "go test ./...",
        )
        .await
    }

    async fn check(&self, request: &BuildRequest) -> Result<BuildResult> {
        run_language_command(
            request,
            request.configuration.check_command.as_deref(),
            default_build_command(CommandKind::Check, "go"),
        )
        .await
    }

    fn default_build_command(&self) -> Option<&str> {
        Some("go build ./...")
    }

    fn default_test_command(&self) -> Option<&str> {
        Some("go test ./...")
    }
}

/// Java adapter (Maven/Gradle).
pub struct JavaAdapter;

#[async_trait]
impl CompilerAdapter for JavaAdapter {
    fn name(&self) -> &str {
        "java"
    }

    async fn detect(&self, workspace: &dyn Workspace) -> bool {
        let files = workspace.list_files().await.unwrap_or_default();
        files
            .iter()
            .any(|f| f.path.ends_with("pom.xml") || f.path.ends_with("build.gradle"))
    }

    async fn prepare(&self, _request: &BuildRequest) -> Result<()> {
        Ok(())
    }

    async fn build(&self, request: &BuildRequest) -> Result<BuildResult> {
        run_language_command(
            request,
            request.configuration.build_command.as_deref(),
            default_build_command(CommandKind::Build, "java"),
        )
        .await
    }

    async fn test(&self, request: &TestRequest) -> Result<TestResult> {
        run_test_command(
            request,
            request.configuration.test_command.as_deref(),
            "mvn test",
        )
        .await
    }

    async fn check(&self, request: &BuildRequest) -> Result<BuildResult> {
        run_language_command(
            request,
            request.configuration.check_command.as_deref(),
            default_build_command(CommandKind::Check, "java"),
        )
        .await
    }

    fn default_build_command(&self) -> Option<&str> {
        Some("mvn package")
    }

    fn default_test_command(&self) -> Option<&str> {
        Some("mvn test")
    }
}

/// C/C++ adapter (CMake/Make).
pub struct CppAdapter;

#[async_trait]
impl CompilerAdapter for CppAdapter {
    fn name(&self) -> &str {
        "cpp"
    }

    async fn detect(&self, workspace: &dyn Workspace) -> bool {
        let files = workspace.list_files().await.unwrap_or_default();
        files
            .iter()
            .any(|f| f.path.ends_with("CMakeLists.txt") || f.path.ends_with("Makefile"))
    }

    async fn prepare(&self, _request: &BuildRequest) -> Result<()> {
        Ok(())
    }

    async fn build(&self, request: &BuildRequest) -> Result<BuildResult> {
        run_language_command(
            request,
            request.configuration.build_command.as_deref(),
            default_build_command(CommandKind::Build, "cpp"),
        )
        .await
    }

    async fn test(&self, request: &TestRequest) -> Result<TestResult> {
        run_test_command(
            request,
            request.configuration.test_command.as_deref(),
            "ctest",
        )
        .await
    }

    async fn check(&self, request: &BuildRequest) -> Result<BuildResult> {
        run_language_command(
            request,
            request.configuration.check_command.as_deref(),
            default_build_command(CommandKind::Check, "cpp"),
        )
        .await
    }

    fn default_build_command(&self) -> Option<&str> {
        Some("cmake --build build")
    }

    fn default_test_command(&self) -> Option<&str> {
        Some("ctest")
    }
}

// Internal helpers are defined above; adapters are stateless unit structs.

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::ids::BuildId;

    #[test]
    fn rust_language_uses_rust_parser_for_diagnostics() {
        let output = CommandOutput {
            exit_code: Some(101),
            stdout: String::new(),
            stderr: "src/main.rs:3:5: error[E0432]: unresolved import\n".to_string(),
            timed_out: false,
        };
        let result = finish_build(BuildId::new(), "rust", &output);
        assert_eq!(result.state, BuildState::Failed);
        assert!(!result.diagnostics.is_empty());
    }
}
