//! AI tool architecture (`§25`, `§26`, `§31`).
//!
//! Every AI interaction with Nexus flows through [`AiTool`]s executed by the
//! [`ToolRegistry`]. The registry is the single server-side enforcement
//! point: permission checks, destructive-approval gating, timeouts, and
//! output caps all live here so no individual tool can weaken them (`§85.5`).

use crate::context::ContextTier;
use crate::permission::{AiPermission, AiPermissionFailure, AiPermissionSet};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// What a tool needs to know about the caller.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// The AI session that initiated the call.
    pub session: crate::session::AiSessionId,
    /// The acting user (never "the AI itself").
    pub user: nexus_core::ids::UserId,
    /// The Cube this tool call is scoped to.
    pub cube: nexus_core::ids::CubeId,
}

/// Validated JSON arguments, as thin wrapping over `serde_json`.
#[derive(Debug, Clone)]
pub struct ToolArguments {
    value: serde_json::Value,
}

impl ToolArguments {
    /// Parses raw JSON; invalid payloads are rejected by the registry, tools
    /// may assume well-formedness.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let value =
            serde_json::from_str::<serde_json::Value>(raw).map_err(|err| err.to_string())?;
        if !value.is_object() {
            return Err(String::from("arguments must be a JSON object"));
        }
        Ok(Self { value })
    }

    #[must_use]
    pub fn string(&self, key: &str) -> Option<&str> {
        self.value.get(key).and_then(serde_json::Value::as_str)
    }

    #[must_use]
    pub fn u64(&self, key: &str) -> Option<u64> {
        self.value.get(key).and_then(serde_json::Value::as_u64)
    }
}

/// Tool execution outcome: text output capped by the registry, or a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub output: String,
    /// `true` when the result was truncated by the output cap (`§36`).
    pub truncated: bool,
}

/// How a tool registers itself (`§25`).
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    /// Stable machine name, e.g. `read_file`.
    pub name: &'static str,
    pub description: String,
    /// JSON-schema-ish argument shape, free-form for provider adaptation.
    pub parameters: String,
    /// The single permission this tool needs.
    pub permission: AiPermission,
    /// Hard wall after which the registry cancels the tool.
    pub timeout: Duration,
    /// Maximum returned text; the registry enforces this.
    pub output_limit: usize,
}

impl ToolDefinition {
    /// Read-only file tool defaults.
    fn readonly(name: &'static str, description: &str, parameters: &str) -> Self {
        Self {
            name,
            description: description.into(),
            parameters: parameters.into(),
            permission: AiPermission::ReadWorkspace,
            timeout: Duration::from_secs(5),
            output_limit: 8_192,
        }
    }
}

/// The tool surface every implementation provides.
#[async_trait]
pub trait AiTool: Send + Sync {
    fn definition(&self) -> ToolDefinition;

    /// Executes the tool. Permission/approval checks have already passed;
    /// the implementation only sees validated arguments.
    ///
    /// # Errors
    /// A human-readable failure string (tool-level errors are not typed —
    /// the orchestrator relays them to the model, `§37`).
    async fn execute(
        &self,
        context: &ToolContext,
        arguments: &ToolArguments,
    ) -> Result<String, String>;
}

/// What tools read: the Cube workspace as plain text (server-supplied).
pub trait WorkspaceSnapshot: Send + Sync {
    /// Full body of one file, when it exists.
    fn read(&self, path: &str) -> Option<String>;
    /// Workspace-relative paths.
    fn list(&self) -> Vec<String>;
}

/// `read_file` (`§26`).
pub struct ReadFileTool {
    workspace: Arc<dyn WorkspaceSnapshot>,
}

impl ReadFileTool {
    #[must_use]
    pub fn new(workspace: Arc<dyn WorkspaceSnapshot>) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl AiTool for ReadFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::readonly(
            "read_file",
            "read the full text of one workspace file",
            r#"{"path": "string"}"#,
        )
    }

    async fn execute(
        &self,
        _context: &ToolContext,
        arguments: &ToolArguments,
    ) -> Result<String, String> {
        let path = arguments.string("path").ok_or("missing `path`")?;
        self.workspace
            .read(path)
            .ok_or_else(|| format!("no such file: {path}"))
    }
}

/// `list_files` (`§26`).
pub struct ListFilesTool {
    workspace: Arc<dyn WorkspaceSnapshot>,
}

impl ListFilesTool {
    #[must_use]
    pub fn new(workspace: Arc<dyn WorkspaceSnapshot>) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl AiTool for ListFilesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::readonly("list_files", "list workspace file paths", r#"{}"#)
    }

    async fn execute(
        &self,
        _context: &ToolContext,
        _arguments: &ToolArguments,
    ) -> Result<String, String> {
        Ok(self.workspace.list().join("\n"))
    }
}

/// `search_workspace` (`§26`, `§19`): naive substring scan with tiered hits.
pub struct SearchWorkspaceTool {
    workspace: Arc<dyn WorkspaceSnapshot>,
}

impl SearchWorkspaceTool {
    #[must_use]
    pub fn new(workspace: Arc<dyn WorkspaceSnapshot>) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl AiTool for SearchWorkspaceTool {
    fn definition(&self) -> ToolDefinition {
        let mut def = ToolDefinition::readonly(
            "search_workspace",
            "substring search over workspace files",
            r#"{"query": "string"}"#,
        );
        def.output_limit = 2_048;
        def
    }

    async fn execute(
        &self,
        _context: &ToolContext,
        arguments: &ToolArguments,
    ) -> Result<String, String> {
        let query = arguments.string("query").ok_or("missing `query`")?;
        let matches = self
            .workspace
            .list()
            .into_iter()
            .filter(|path| {
                self.workspace
                    .read(path)
                    .is_some_and(|body| body.contains(query))
            })
            .collect::<Vec<_>>();
        let mut lines = matches
            .iter()
            .map(|path| format!("{path} [{:?}]", ContextTier::RelatedFiles))
            .collect::<Vec<_>>();
        if lines.len() > 32 {
            lines.truncate(32);
            lines.push(String::from("…"));
        }
        Ok(lines.join("\n"))
    }
}

// ---------------------------------------------------------------------------
// The registry (server-side enforcement, `§32`)
// ---------------------------------------------------------------------------

/// Enforcement decisions a host can veto through its own policy hook.
pub trait ApprovalGate: Send + Sync {
    /// Whether a destructive operation has an outstanding approval for this
    /// user/session. Defaults deny (`§39`).
    fn approved(&self, user: nexus_core::ids::UserId, permission: AiPermission) -> bool;
}

impl ApprovalGate for () {
    fn approved(&self, _user: nexus_core::ids::UserId, _permission: AiPermission) -> bool {
        false
    }
}

/// Failures reported by the registry, never by tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolFailure {
    UnknownTool(String),
    InvalidArguments(String),
    Permission(AiPermissionFailure),
    Timeout(String),
    ToolError(String),
}

impl core::fmt::Display for ToolFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownTool(name) => write!(f, "unknown tool: {name}"),
            Self::InvalidArguments(message) => write!(f, "invalid arguments: {message}"),
            Self::Permission(failure) => write!(f, "permission failure: {failure:?}"),
            Self::Timeout(name) => write!(f, "tool {name} timed out"),
            Self::ToolError(message) => write!(f, "tool failed: {message}"),
        }
    }
}

/// The single enforcement point for AI tool calls.
#[derive(Clone)]
pub struct ToolRegistry {
    tools: BTreeMap<&'static str, (ToolDefinition, Arc<dyn AiTool>)>,
    permissions: Arc<dyn Fn(&ToolContext) -> AiPermissionSet + Send + Sync>,
    approvals: Arc<dyn ApprovalGate>,
}

impl core::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ToolRegistry {
    /// A registry resolving permissions through `permissions` and destructive
    /// approvals through `approvals`. Both are server-side constructs; tools
    /// can never reach around them.
    pub fn new(
        permissions: Arc<dyn Fn(&ToolContext) -> AiPermissionSet + Send + Sync>,
        approvals: Arc<dyn ApprovalGate>,
    ) -> Self {
        Self {
            tools: BTreeMap::new(),
            permissions,
            approvals,
        }
    }

    /// Registers a tool; the same name replaces an earlier binding.
    pub fn register(&mut self, tool: Arc<dyn AiTool>) {
        let definition = tool.definition();
        self.tools.insert(definition.name, (definition, tool));
    }

    /// Registered tool names.
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        self.tools.keys().copied().collect()
    }

    /// Definitions in provider-facing shape.
    #[must_use]
    pub fn definitions(&self) -> Vec<crate::request::AiToolDefinition> {
        self.tools
            .values()
            .map(|(def, _)| crate::request::AiToolDefinition {
                name: def.name.into(),
                description: def.description.clone(),
                parameters: def.parameters.clone(),
            })
            .collect()
    }

    /// Executes `name` under full enforcement (`§25`): parse → permission →
    /// approval → timeout → output cap.
    ///
    /// # Errors
    /// See [`ToolFailure`]; tool-level errors are wrapped into
    /// `ToolFailure::ToolError`.
    pub async fn call(
        &self,
        name: &str,
        context: &ToolContext,
        raw_arguments: &str,
    ) -> Result<ToolResult, ToolFailure> {
        let Some((definition, tool)) = self.tools.get(name) else {
            return Err(ToolFailure::UnknownTool(name.into()));
        };
        let arguments =
            ToolArguments::parse(raw_arguments).map_err(ToolFailure::InvalidArguments)?;

        let set = (self.permissions)(context);
        match set.authorize(definition.permission) {
            Ok(()) => {}
            Err(AiPermissionFailure::NeedsApproval(permission)) => {
                if !self.approvals.approved(context.user, permission) {
                    return Err(ToolFailure::Permission(AiPermissionFailure::NeedsApproval(
                        permission,
                    )));
                }
            }
            Err(failure) => return Err(ToolFailure::Permission(failure)),
        }

        // Timeout + output cap (`§31`, `§36`).
        let (tx, mut rx) = mpsc::channel::<Result<String, String>>(1);
        let worker = tool.clone();
        let arguments = arguments.clone();
        let worker_context = context.clone();
        tokio::spawn(async move {
            let outcome = worker.execute(&worker_context, &arguments).await;
            let _ = tx.send(outcome).await;
        });
        let raw = tokio::time::timeout(definition.timeout, rx.recv())
            .await
            .map_err(|_| ToolFailure::Timeout(definition.name.to_owned()))?
            .ok_or_else(|| ToolFailure::ToolError(String::from("tool finished without a result")))?
            .map_err(ToolFailure::ToolError)?;

        let mut output = raw;
        let mut truncated = false;
        if output.len() > definition.output_limit {
            output.truncate(definition.output_limit);
            output.push_str("\n…[output truncated]");
            truncated = true;
        }
        Ok(ToolResult { output, truncated })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::ids::{CubeId, UserId};

    struct StaticWorkspace {
        files: BTreeMap<String, String>,
    }

    impl StaticWorkspace {
        fn standard() -> Self {
            let mut files = BTreeMap::new();
            files.insert(
                String::from("src/main.rs"),
                String::from("fn main() { println!(\"nexus\"); }"),
            );
            files.insert(
                String::from("src/auth.rs"),
                String::from("fn authenticate(token: &str) -> bool { token == \"let-me-in\" }"),
            );
            files.insert(String::from("notes.txt"), String::from("remember the milk"));
            files.insert(String::from(".env"), String::from("API_KEY=private"));
            Self { files }
        }
    }

    impl WorkspaceSnapshot for StaticWorkspace {
        fn read(&self, path: &str) -> Option<String> {
            self.files.get(path).cloned()
        }

        fn list(&self) -> Vec<String> {
            self.files.keys().cloned().collect()
        }
    }

    fn context_for(user: u64) -> ToolContext {
        ToolContext {
            session: crate::session::AiSessionId::new(1),
            user: UserId::from_u64(user),
            cube: CubeId::from_u64(7),
        }
    }

    fn broad_registry(gate: Arc<dyn ApprovalGate>) -> ToolRegistry {
        let permissions: Arc<dyn Fn(&ToolContext) -> AiPermissionSet + Send + Sync> =
            Arc::new(|_| {
                AiPermissionSet::new([
                    AiPermission::ReadWorkspace,
                    AiPermission::EnumerateWorkspace,
                    AiPermission::DeleteFiles,
                ])
            });
        let mut registry = ToolRegistry::new(permissions, gate);
        let workspace: Arc<dyn WorkspaceSnapshot> = Arc::new(StaticWorkspace::standard());
        registry.register(Arc::new(ReadFileTool::new(workspace.clone())));
        registry.register(Arc::new(ListFilesTool::new(workspace.clone())));
        registry.register(Arc::new(SearchWorkspaceTool::new(workspace)));
        registry
    }

    fn deny_gate() -> Arc<dyn ApprovalGate> {
        Arc::new(())
    }

    #[tokio::test]
    async fn read_file_returns_bodies_and_errors_missing_paths() {
        let registry = broad_registry(deny_gate());
        let context = context_for(1);
        let result = registry
            .call("read_file", &context, r#"{"path":"src/main.rs"}"#)
            .await
            .unwrap();
        assert!(result.output.contains("fn main"));
        assert!(!result.truncated);

        let err = registry
            .call("read_file", &context, r#"{"path":"ghost.rs"}"#)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no such file"));

        let bad = registry
            .call("read_file", &context, "not json")
            .await
            .unwrap_err();
        assert!(matches!(bad, ToolFailure::InvalidArguments(_)));
    }

    #[tokio::test]
    async fn unknown_tools_and_arguments_are_rejected_before_any_execution() {
        let registry = broad_registry(deny_gate());
        let context = context_for(1);
        assert_eq!(
            registry.call("rm_rf", &context, "{}").await.unwrap_err(),
            ToolFailure::UnknownTool(String::from("rm_rf"))
        );
        assert!(matches!(
            registry
                .call("list_files", &context, "[]")
                .await
                .unwrap_err(),
            ToolFailure::InvalidArguments(_)
        ));
    }

    #[tokio::test]
    async fn search_hits_file_names_with_tier_markers() {
        let registry = broad_registry(deny_gate());
        let result = registry
            .call(
                "search_workspace",
                &context_for(1),
                r#"{"query":"authenticate"}"#,
            )
            .await
            .unwrap();
        // Only the auth file mentions `authenticate`; .env is listed but not
        // a hit — and, notably, the *body* of no file is returned here.
        assert_eq!(result.output, "src/auth.rs [RelatedFiles]");
    }

    #[tokio::test]
    async fn permission_gate_refuses_ungranted_tools() {
        // A narrow permission set: no workspace access at all.
        let permissions: Arc<dyn Fn(&ToolContext) -> AiPermissionSet + Send + Sync> =
            Arc::new(|_| AiPermissionSet::new([AiPermission::NetworkEgress]));
        let mut registry = ToolRegistry::new(permissions, deny_gate());
        let workspace: Arc<dyn WorkspaceSnapshot> = Arc::new(StaticWorkspace::standard());
        registry.register(Arc::new(ReadFileTool::new(workspace)));

        let failure = registry
            .call("read_file", &context_for(1), r#"{"path":"src/main.rs"}"#)
            .await
            .unwrap_err();
        assert_eq!(
            failure,
            ToolFailure::Permission(AiPermissionFailure::NotGranted(AiPermission::ReadWorkspace))
        );
    }

    struct ApprovingGate;
    impl ApprovalGate for ApprovingGate {
        fn approved(&self, _user: UserId, _permission: AiPermission) -> bool {
            true
        }
    }

    struct EagerDeleteTool;

    #[async_trait]
    impl AiTool for EagerDeleteTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "delete_file",
                description: String::from("delete one file"),
                parameters: String::from(r#"{"path": "string"}"#),
                permission: AiPermission::DeleteFiles,
                timeout: Duration::from_secs(5),
                output_limit: 256,
            }
        }

        async fn execute(
            &self,
            _context: &ToolContext,
            arguments: &ToolArguments,
        ) -> Result<String, String> {
            let path = arguments.string("path").ok_or("missing `path`")?;
            Ok(format!("deleted {path}"))
        }
    }

    #[tokio::test]
    async fn destructive_tools_are_blocked_without_approval() {
        let mut registry = broad_registry(deny_gate());
        registry.register(Arc::new(EagerDeleteTool));

        let blocked = registry
            .call("delete_file", &context_for(1), r#"{"path":"notes.txt"}"#)
            .await
            .unwrap_err();
        assert_eq!(
            blocked,
            ToolFailure::Permission(AiPermissionFailure::NeedsApproval(
                AiPermission::DeleteFiles
            ))
        );

        // With an approving gate the same call goes through — the gate is a
        // server-side construct (`§32`), not a TUI nicety.
        let mut approving = broad_registry(Arc::new(ApprovingGate));
        approving.register(Arc::new(EagerDeleteTool));
        let result = approving
            .call("delete_file", &context_for(1), r#"{"path":"notes.txt"}"#)
            .await
            .unwrap();
        assert_eq!(result.output, "deleted notes.txt");
    }

    struct OversizeTool;

    #[async_trait]
    impl AiTool for OversizeTool {
        fn definition(&self) -> ToolDefinition {
            let mut def = ToolDefinition::readonly("oversize", "emits a wall of text", "{}");
            def.output_limit = 64;
            def
        }

        async fn execute(
            &self,
            _context: &ToolContext,
            _arguments: &ToolArguments,
        ) -> Result<String, String> {
            Ok("z".repeat(1_000))
        }
    }

    #[tokio::test]
    async fn output_is_capped_to_the_definition_limit() {
        let mut registry = broad_registry(deny_gate());
        registry.register(Arc::new(OversizeTool));
        let result = registry
            .call("oversize", &context_for(1), "{}")
            .await
            .unwrap();
        assert!(result.truncated);
        assert!(result.output.len() < 100);
        assert!(result.output.ends_with("…[output truncated]"));
    }

    struct SleepingTool;

    #[async_trait]
    impl AiTool for SleepingTool {
        fn definition(&self) -> ToolDefinition {
            let mut def = ToolDefinition::readonly("sleepy", "hangs forever", "{}");
            def.timeout = Duration::from_millis(50);
            def
        }

        async fn execute(
            &self,
            _context: &ToolContext,
            _arguments: &ToolArguments,
        ) -> Result<String, String> {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok("woke up".into())
        }
    }

    #[tokio::test]
    async fn hanging_tools_hit_the_timeout_wall() {
        let mut registry = broad_registry(deny_gate());
        registry.register(Arc::new(SleepingTool));
        let failure = registry
            .call("sleepy", &context_for(1), "{}")
            .await
            .unwrap_err();
        assert!(matches!(failure, ToolFailure::Timeout(_)));
    }

    #[tokio::test]
    async fn definitions_map_into_provider_shape() {
        let registry = broad_registry(deny_gate());
        let definitions = registry.definitions();
        assert!(definitions.iter().any(|d| d.name == "read_file"));
        assert!(definitions.iter().any(|d| d.name == "search_workspace"));
        assert!(registry.names().contains(&"list_files"));
    }
}
