//! AI subsystem crate (Part 12).
//!
//! Provider-independent abstractions: models, conversations, requests,
//! streaming providers, and the provider registry. Everything here is
//! vendor-neutral — vendor conversion code lives only inside adapters, and
//! the server stays authoritative over every AI action.

pub mod context;
pub mod editing;
pub mod error;
pub mod events;
pub mod message;
pub mod metrics;
pub mod mock;
pub mod model;
pub mod orchestrator;
pub mod permission;
pub mod provider;
pub mod registry;
pub mod request;
pub mod roundtrip;
pub mod session;
pub mod tools;

pub use context::{
    assemble, fingerprint_of, rank, AssembledContext, BudgetLedger, ContextCache, ContextFragment,
    ContextRequest, ContextTier, RankedFragment, SensitiveConfig, SensitivePolicy, TokenBudget,
};
pub use editing::{
    unified_diff, AppliedTransaction, Applier, CreateFileTool, DeleteFileTool, EditOperation,
    EditStaging, EditTransaction, ProposeEditTool, ProposedEdit, StagingFailure, TransactionState,
};
pub use error::{AiError, AiErrorKind, AiResult, AiUsage};
pub use events::{AiEvent, AiEventKind, AiEventLedger, AiVisibility};
pub use message::{AiMessage, AiRole, AiToolInvocation};
pub use metrics::{AiMetrics, LocalModelStats, MetricsSnapshot, ModelUsage, UsageKey};
pub use mock::{MockConfig, MockProvider, MockTurn, StreamScript};
pub use model::{AiCapabilities, AiModel, CostInformation, ModelId, ProviderId};
pub use orchestrator::{AiOrchestrator, RetryPolicy};
pub use permission::{AiPermission, AiPermissionFailure, AiPermissionSet};
pub use provider::{AiProvider, AiStream, AiStreamEvent};
pub use registry::AiProviderRegistry;
pub use request::{AiRequest, AiResponse, AiToolDefinition, AiToolInvocationRes, FinishReason};
pub use session::{AiSession, AiSessionId, AiSessionManager, AiSessionScope};
pub use tools::{
    AiTool, ApprovalGate, ListFilesTool, ReadFileTool, SearchWorkspaceTool, ToolArguments,
    ToolContext, ToolDefinition, ToolFailure, ToolRegistry, ToolResult, WorkspaceSnapshot,
};

/// Crate phase marker.
pub const PHASE: &str = "part12-ai";

#[cfg(test)]
mod tests {
    #[test]
    fn phase_is_tracked() {
        assert_eq!(super::PHASE, "part12-ai");
    }
}
