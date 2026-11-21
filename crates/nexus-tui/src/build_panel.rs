//! Build panel: diagnostics and build output for the active cube.
//!
//! The panel is pure state; scheduling builds and piping output happens in the
//! session service. Diagnostics are bounded so pathological builds cannot
//! exhaust memory.

use std::collections::VecDeque;
use std::path::PathBuf;

/// Severity of a build diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Error,
    Warning,
    Info,
}

/// One diagnostic produced by a build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

/// Final outcome of a finished build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildOutcome {
    Success,
    Failed,
}

/// Bounded build state shown in the build panel.
#[derive(Debug, Clone, Default)]
pub struct BuildState {
    pub running: bool,
    pub queued: bool,
    pub command: Option<String>,
    pub outcome: Option<BuildOutcome>,
    /// stdout/stderr lines from the most recent build.
    pub output: VecDeque<String>,
    /// Sorted diagnostics (highest severity first).
    pub diagnostics: Vec<Diagnostic>,
}

/// Maximum diagnostics kept per build.
pub const MAX_DIAGNOSTICS: usize = 512;

/// Maximum output lines kept per build.
pub const MAX_BUILD_OUTPUT: usize = 2_000;

impl BuildState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            running: false,
            queued: false,
            command: None,
            outcome: None,
            output: VecDeque::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Marks a build as started by `command`.
    pub fn start(&mut self, command: impl Into<String>) {
        self.running = true;
        self.queued = false;
        self.command = Some(command.into());
        self.outcome = None;
        self.output.clear();
        self.diagnostics.clear();
    }

    /// Appends raw output, keeping the queue bounded.
    pub fn push_output(&mut self, text: impl Into<String>) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        self.output.push_back(text);
        while self.output.len() > MAX_BUILD_OUTPUT {
            self.output.pop_front();
        }
    }

    /// Records one diagnostic (bounded, most severe first).
    pub fn add_diagnostic(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
        // Keep the list bounded and sorted so the panel can render top-down.
        self.diagnostics.sort_by_key(|d| severity_rank(d.level));
        if self.diagnostics.len() > MAX_DIAGNOSTICS {
            self.diagnostics.truncate(MAX_DIAGNOSTICS);
        }
    }

    /// Finishes a build with its final outcome.
    pub fn finish(&mut self, outcome: BuildOutcome) {
        self.running = false;
        self.outcome = Some(outcome);
    }

    pub fn clear(&mut self) {
        self.output.clear();
        self.diagnostics.clear();
        self.outcome = None;
        self.running = false;
        self.queued = false;
        self.command = None;
    }
}

fn severity_rank(level: DiagnosticLevel) -> u8 {
    match level {
        DiagnosticLevel::Error => 0,
        DiagnosticLevel::Warning => 1,
        DiagnosticLevel::Info => 2,
    }
}

// ---- Protocol glue (Part 10) ----------------------------------------------

impl Diagnostic {
    /// Maps a server-side `nexus_core` diagnostic onto the panel's compact
    /// form, dropping severity levels the panel does not display hints for.
    #[must_use]
    pub fn from_core(diagnostic: nexus_core::Diagnostic) -> Self {
        Self {
            level: match diagnostic.severity {
                nexus_core::DiagnosticSeverity::Error => DiagnosticLevel::Error,
                nexus_core::DiagnosticSeverity::Warning => DiagnosticLevel::Warning,
                nexus_core::DiagnosticSeverity::Info | nexus_core::DiagnosticSeverity::Hint => {
                    DiagnosticLevel::Info
                }
            },
            message: diagnostic.message,
            file: diagnostic.path.map(PathBuf::from),
            line: diagnostic.line.map(|l| l as usize),
            column: diagnostic.column.map(|c| c as usize),
        }
    }
}

impl BuildState {
    /// Consumes a streamed [`nexus_protocol::Message::BuildOutput`] chunk.
    pub fn push_protocol_output(
        &mut self,
        stream: nexus_protocol::message::BuildOutputStream,
        data: impl Into<String>,
    ) {
        // Output and stderr are rendered identically by the build panel; the
        // distinction is recorded nowhere else today.
        let _ = stream;
        self.push_output(data);
    }

    /// Consumes a [`nexus_protocol::Message::BuildDiagnostics`] batch.
    pub fn push_protocol_diagnostics(
        &mut self,
        diagnostics: impl IntoIterator<Item = nexus_core::Diagnostic>,
    ) {
        for diagnostic in diagnostics {
            self.add_diagnostic(Diagnostic::from_core(diagnostic));
        }
    }

    /// Consumes a [`nexus_protocol::Message::BuildFinished`] result.
    pub fn apply_protocol_result(&mut self, result: &nexus_core::BuildResult) {
        if result.state == nexus_core::BuildState::Succeeded {
            self.finish(BuildOutcome::Success);
        } else {
            self.finish(BuildOutcome::Failed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_resets_output_and_diagnostics() {
        let mut b = BuildState::new();
        b.push_output("old");
        b.add_diagnostic(Diagnostic {
            level: DiagnosticLevel::Warning,
            message: "w1".into(),
            file: None,
            line: None,
            column: None,
        });
        b.start("cargo build");
        assert!(b.running);
        assert!(b.output.is_empty());
        assert!(b.diagnostics.is_empty());
        assert_eq!(b.command.as_deref(), Some("cargo build"));
    }

    #[test]
    fn diagnostics_sort_and_bound() {
        let mut b = BuildState::new();
        for i in 0..(MAX_DIAGNOSTICS + 5) {
            b.add_diagnostic(Diagnostic {
                level: if i % 2 == 0 {
                    DiagnosticLevel::Error
                } else {
                    DiagnosticLevel::Info
                },
                message: format!("d{i}"),
                file: None,
                line: None,
                column: None,
            });
        }
        assert!(b.diagnostics.len() <= MAX_DIAGNOSTICS);
        assert_eq!(b.diagnostics[0].level, DiagnosticLevel::Error);
    }

    #[test]
    fn output_is_bounded() {
        let mut b = BuildState::new();
        for i in 0..(MAX_BUILD_OUTPUT + 10) {
            b.push_output(i.to_string());
        }
        assert_eq!(b.output.len(), MAX_BUILD_OUTPUT);
    }

    #[test]
    fn lifecycle_finish_and_clear() {
        let mut b = BuildState::new();
        b.start("make");
        b.finish(BuildOutcome::Failed);
        assert_eq!(b.outcome, Some(BuildOutcome::Failed));
        assert!(!b.running);
        b.clear();
        assert!(b.command.is_none());
        assert!(b.output.is_empty());
    }

    #[test]
    fn protocol_glue_maps_diagnostics_and_results() {
        let mut b = BuildState::new();
        b.start("cargo build");
        b.push_protocol_diagnostics([nexus_core::Diagnostic {
            file_id: None,
            path: Some("src/main.rs".into()),
            line: Some(7),
            column: Some(3),
            end_line: None,
            end_column: None,
            severity: nexus_core::DiagnosticSeverity::Error,
            message: "boom".into(),
            code: Some("E1".into()),
            source: "rustc".into(),
        }]);
        assert_eq!(b.diagnostics.len(), 1);
        assert_eq!(b.diagnostics[0].level, DiagnosticLevel::Error);
        assert_eq!(b.diagnostics[0].line, Some(7));

        let mut result = nexus_core::BuildResult {
            build_id: nexus_core::ids::BuildId::new(),
            state: nexus_core::BuildState::Succeeded,
            exit_code: Some(0),
            duration_ms: 1,
            started_at: nexus_core::Timestamp::now(),
            finished_at: Some(nexus_core::Timestamp::now()),
            diagnostics: vec![],
            artifacts: vec![],
            resource_usage: nexus_core::ResourceUsage {
                peak_memory_bytes: 0,
                cpu_time_ms: 0,
                wall_time_ms: 0,
            },
        };
        let _ = &mut result;
        b.apply_protocol_result(&result);
        assert_eq!(b.outcome, Some(BuildOutcome::Success));
        assert!(!b.running);

        b.start("cargo build");
        b.push_protocol_output(nexus_protocol::message::BuildOutputStream::Stderr, "warn");
        assert_eq!(b.output.back().map(String::as_str), Some("warn"));
    }
}
