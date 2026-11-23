//! Integration round-trips (`§78`).
//!
//! The "explain code" and "tool loop" workflows over the deterministic mock
//! provider — no real vendor, no real workspace state, full pipeline engaged.

#![cfg(test)]

use crate::context::{assemble, ContextRequest, ContextTier, SensitiveConfig, TokenBudget};
use crate::message::{AiMessage, AiRole};
use crate::mock::{MockConfig, MockProvider, MockTurn, StreamScript};
use crate::model::{ModelId, ProviderId};
use crate::orchestrator::{AiOrchestrator, RetryPolicy};
use crate::permission::{AiPermission, AiPermissionSet};
use crate::provider::{AiProvider, AiStreamEvent};
use crate::registry::AiProviderRegistry;
use crate::request::FinishReason;
use crate::session::{AiSessionId, AiSessionManager, AiSessionScope};
use crate::tools::{ReadFileTool, ToolContext, ToolRegistry};
use nexus_core::ids::{CubeId, UserId};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Minimal in-memory workspace implementing the tool snapshot boundary.
struct FakeWorkspace {
    files: BTreeMap<String, String>,
}

impl FakeWorkspace {
    fn standard() -> Self {
        let mut files = BTreeMap::new();
        files.insert(
            String::from("src/order.rs"),
            String::from("fn calculate_total(items: &[i64]) -> i64 { items.iter().sum() }"),
        );
        files.insert(
            String::from("src/main.rs"),
            String::from("fn main() { println!(\"nexus\"); }"),
        );
        Self { files }
    }
}

impl crate::tools::WorkspaceSnapshot for FakeWorkspace {
    fn read(&self, path: &str) -> Option<String> {
        self.files.get(path).cloned()
    }

    fn list(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }
}

/// Registry resolving any user to a permissive read-only set.
fn permissive_registry(gate: Arc<dyn crate::tools::ApprovalGate>) -> ToolRegistry {
    let permissions: Arc<dyn Fn(&ToolContext) -> AiPermissionSet + Send + Sync> =
        Arc::new(|_| AiPermissionSet::new([AiPermission::ReadWorkspace]));
    let mut tool_registry = ToolRegistry::new(permissions, gate);
    let workspace: Arc<dyn crate::tools::WorkspaceSnapshot> = Arc::new(FakeWorkspace::standard());
    tool_registry.register(Arc::new(ReadFileTool::new(workspace)));
    tool_registry
}

fn tool_context() -> ToolContext {
    ToolContext {
        session: AiSessionId::new(1),
        user: UserId::from_u64(1),
        cube: CubeId::from_u64(7),
    }
}

fn context_request_with_selection() -> ContextRequest {
    ContextRequest {
        selection: Some(String::from("fn calculate_total(items: &[i64]) -> i64")),
        current_path: Some(String::from("src/order.rs")),
        current_body: Some(String::from(
            "fn calculate_total(items: &[i64]) -> i64 { items.iter().sum() }",
        )),
        related: vec![(
            String::from("src/main.rs"),
            String::from("fn main() { println!(\"nexus\"); }"),
            ContextTier::RelatedFiles,
        )],
        model_context_window: 8_192,
        model: None,
    }
}

fn registry_with(provider: Arc<MockProvider>) -> AiProviderRegistry {
    let mut registry = AiProviderRegistry::new();
    registry.register(provider);
    registry
}

#[tokio::test]
async fn explain_code_round_trip_fences_context_and_streams() {
    // 1) Client assembles relevance-ranked, fenced context (§15, §18, §24).
    let request = context_request_with_selection();
    let sensitive = SensitiveConfig::new(crate::context::SensitivePolicy::Allowed);
    let assembled = assemble(
        &request,
        &sensitive,
        TokenBudget {
            max_context_tokens: 8_192,
            reserved_output_tokens: 512,
        },
    );
    // Dedup by path keeps the selection (higher tier) for src/order.rs.
    assert_eq!(assembled.fragments.len(), 2);
    assert_eq!(assembled.fragments[0].tier, ContextTier::Selection);
    assert!(assembled
        .text
        .contains("<workspace-data path=\"src/order.rs\""));

    // 2) The orchestrator routes through the registered mock provider.
    let provider = Arc::new(MockProvider::new(
        "mock",
        MockConfig::new()
            .turn(MockTurn::Streamed(vec![
                StreamScript::Delta(String::from("Sums the")),
                StreamScript::Delta(String::from(" slice.")),
                StreamScript::Complete(11, 3, FinishReason::Stop),
            ]))
            .turn(MockTurn::Content(String::from(
                "It sums a slice of integers.",
            ))),
    ));
    let providers = registry_with(provider.clone());
    let mut orchestrator = AiOrchestrator::new(RetryPolicy::default_policy(), None);

    let mut ai_request = crate::request::AiRequest::chat(
        ProviderId::new("mock"),
        ModelId::new("mock-small"),
        vec![AiMessage::user("explain what this selection does")],
    );
    ai_request.context = assembled.text.clone();
    ai_request.stream = true;

    // 3) Streaming path: deltas arrive before the completed event (§13).
    let mut stream = providers
        .get(provider.provider_id())
        .unwrap()
        .stream(ai_request.clone())
        .await
        .unwrap();
    let mut streamed_text = String::new();
    let mut saw_completed = false;
    while let Some(event) = stream.next().await {
        match event {
            AiStreamEvent::Content { delta } => streamed_text.push_str(&delta),
            AiStreamEvent::Completed { usage, stop } => {
                saw_completed = true;
                assert_eq!(stop, FinishReason::Stop);
                assert_eq!(usage.prompt_tokens + usage.completion_tokens, 14);
                break;
            }
            _ => panic!("unexpected stream event"),
        }
    }
    stream.shutdown();
    assert!(saw_completed && streamed_text.contains("Sums the"));

    // 4) Complete path through the orchestrator: retry metrics stay at zero
    //    and usage flows into the side-channel (§53).
    let reply = orchestrator.complete(&providers, ai_request).await.unwrap();
    assert_eq!(reply.content, "It sums a slice of integers.");
    assert_eq!(orchestrator.retries(), 0);
    assert_eq!(orchestrator.usage().total(), 15);

    // 5) The session records both sides of the exchange (§11–§12).
    let mut sessions = AiSessionManager::new();
    let alice = UserId::from_u64(1);
    let id = sessions.open(
        alice,
        AiSessionScope::Private,
        ProviderId::new("mock"),
        ModelId::new("mock-small"),
    );
    sessions
        .record(
            id,
            alice,
            AiMessage::user("explain what this selection does"),
        )
        .unwrap();
    sessions
        .record(id, alice, AiMessage::assistant(reply.content.clone()))
        .unwrap();
    let session = sessions.get(id, alice).unwrap();
    assert_eq!(session.history.len(), 2);
    assert_eq!(session.history[1].role, AiRole::Assistant);
}

#[tokio::test]
async fn tool_call_loop_round_trip_executes_read_file() {
    // 1) First round trip: the provider decides it needs a file read.
    let provider = Arc::new(MockProvider::new(
        "mock",
        MockConfig::new()
            .turn(MockTurn::ToolCalls(vec![(
                String::from("call-1"),
                String::from(r#"{"path":"src/order.rs"}"#),
            )]))
            .turn(MockTurn::Content(String::from(
                "The function sums every element of the slice.",
            ))),
    ));
    let providers = registry_with(provider.clone());
    let tool_registry = permissive_registry(Arc::new(()));
    let mut orchestrator = AiOrchestrator::new(RetryPolicy::default_policy(), None);

    let first = orchestrator
        .complete(
            &providers,
            crate::request::AiRequest::chat(
                ProviderId::new("mock"),
                ModelId::new("mock-small"),
                vec![AiMessage::user("explain the function in src/order.rs")],
            ),
        )
        .await
        .unwrap();
    assert_eq!(first.finish, FinishReason::ToolCall);
    let provider_calls = provider.served();

    // 2) Tool execution is authorized, then its result becomes untrusted
    //    tool-result content (`§24`).
    let result = tool_registry
        .call("read_file", &tool_context(), r#"{"path":"src/order.rs"}"#)
        .await
        .unwrap();
    assert!(result.output.contains("calculate_total"));

    // 3) Second round trip with the conversation flow appended: the mock's
    //    next script turn answers in plain text.
    let second = orchestrator
        .complete(
            &providers,
            crate::request::AiRequest::chat(
                ProviderId::new("mock"),
                ModelId::new("mock-small"),
                vec![
                    AiMessage::user("explain the function in src/order.rs"),
                    AiMessage::tool_call(
                        "call-1",
                        crate::message::AiToolInvocation {
                            id: String::from("scripted"),
                            arguments: String::from(r#"{"path":"src/order.rs"}"#),
                        },
                    ),
                    AiMessage::tool_result("call-1", result.output),
                ],
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        second.content,
        "The function sums every element of the slice."
    );
    assert_eq!(second.finish, FinishReason::Stop);
    assert_eq!(provider.served(), provider_calls + 1);
    assert_eq!(orchestrator.retries(), 0);
}

/// Final sanity: streaming a tool-call turn emits ToolCall events in order.
#[tokio::test]
async fn streamed_tool_calls_flow_through_the_stream() {
    let provider = Arc::new(MockProvider::new(
        "mock",
        MockConfig::new().turn(MockTurn::Streamed(vec![
            StreamScript::Delta(String::from("Let me check.")),
            StreamScript::Tool(
                String::from("course-2"),
                String::from(r#"{"path":"src/order.rs"}"#),
            ),
            StreamScript::Complete(4, 3, FinishReason::ToolCall),
        ])),
    ));
    let mut stream = provider
        .stream(crate::request::AiRequest::chat(
            ProviderId::new("mock"),
            ModelId::new("mock-small"),
            vec![AiMessage::user("what is inside?")],
        ))
        .await
        .unwrap();
    let mut tool_calls = 0;
    let mut deltas = String::new();
    let mut stop = None;
    while let Some(event) = stream.next().await {
        match event {
            AiStreamEvent::Content { delta } => deltas.push_str(&delta),
            AiStreamEvent::ToolCall { .. } => tool_calls += 1,
            AiStreamEvent::Completed { stop: s, .. } => {
                stop = Some(s);
                break;
            }
            AiStreamEvent::Failed { .. } => panic!("stream failure not expected here"),
            AiStreamEvent::Started { .. } => {}
        }
    }
    stream.shutdown();
    assert_eq!(tool_calls, 1);
    assert_eq!(stop, Some(FinishReason::ToolCall));
    assert!(deltas.contains("Let me check"));
}
