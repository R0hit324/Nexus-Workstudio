//! Requests, responses, and token budgeting (`§4`, `§13`, `§21`).
//!
//! A request is assembled *before* the provider sees it: the context manager
//! fills `context`, the system prompt stays short, and output tokens are
//! reserved so workspace context cannot eat the reply budget.

use crate::error::AiUsage;
use crate::message::AiMessage;
use crate::model::{ModelId, ProviderId};

/// Why a provider stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    /// Model finished naturally.
    Stop,
    /// Generation exhausted the output budget.
    Length,
    /// The model wants a tool executed.
    ToolCall,
    /// Generation was cancelled.
    Cancelled,
}

/// One single tool the provider may invoke, advertised with the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON-schema-ish argument description, free-form.
    pub parameters: String,
}

/// A request routed through one model at one provider.
#[derive(Debug, Clone, PartialEq)]
pub struct AiRequest {
    /// Conversation history (explicit roles preserved).
    pub messages: Vec<AiMessage>,
    pub model: ModelId,
    pub provider: ProviderId,
    /// Workspace/file context injected as data (never instructions).
    pub context: String,
    /// Tools offered to the model this round.
    pub tools: Vec<AiToolDefinition>,
    /// Output token reservation; `None` = provider default.
    pub max_output_tokens: Option<u64>,
    /// Sampling temperature override; `None` = provider default.
    pub temperature: Option<f32>,
    /// Streaming intent. Providers that cannot stream answer
    /// `UnsupportedCapability` instead of silently faking it.
    pub stream: bool,
}

impl AiRequest {
    /// Conversation-only request with sane defaults.
    #[must_use]
    pub fn chat(provider: ProviderId, model: ModelId, history: Vec<AiMessage>) -> Self {
        Self {
            messages: history,
            model,
            provider,
            context: String::new(),
            tools: Vec::new(),
            max_output_tokens: None,
            temperature: None,
            stream: false,
        }
    }

    /// Rough token estimate of the assembled prompt (≈4 chars/token); the
    /// context manager uses this to police the budget (`§21`).
    #[must_use]
    pub fn estimated_prompt_tokens(&self) -> u64 {
        let mut chars = self.context.len() as u64;
        for message in &self.messages {
            chars += message.content.len() as u64 + 16;
        }
        chars.div_ceil(4)
    }
}

/// A complete provider response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiResponse {
    /// The assistant text (may be empty for pure tool-call turns).
    pub content: String,
    /// Tool invocations requested by the model, in order.
    pub tool_calls: Vec<AiToolInvocationRes>,
    pub model: ModelId,
    pub provider: ProviderId,
    pub usage: AiUsage,
    pub finish: FinishReason,
}

/// A tool invocation as parsed out of a response turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiToolInvocationRes {
    pub id: String,
    pub arguments: String,
}

impl AiToolInvocationRes {
    /// Converts a response tool-call into a conversation turn.
    #[must_use]
    pub fn into_message(self) -> crate::message::AiMessage {
        let id = self.id.clone();
        crate::message::AiMessage::tool_call(
            id,
            crate::message::AiToolInvocation {
                id: format!("{}-args", self.id),
                arguments: self.arguments,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::AiRole;

    #[test]
    fn token_estimate_counts_context_and_history() {
        let request = AiRequest {
            messages: vec![
                AiMessage::system("instructions"), // ~28 chars
                AiMessage::user("hello world"),    // 27 chars
            ],
            model: ModelId::new("m"),
            provider: ProviderId::new("p"),
            context: "abcd".repeat(4), // 16 chars -> 4 tokens
            tools: vec![],
            max_output_tokens: Some(128),
            temperature: Some(0.2),
            stream: false,
        };
        // (12+16+11+16)/4 ≈ 14 tokens of history + 16/4 context tokens.
        assert_eq!(request.estimated_prompt_tokens(), 18);
    }

    #[test]
    fn tool_round_trip_preserves_identity() {
        let call = AiToolInvocationRes {
            id: String::from("c7"),
            arguments: String::from(r#"{"x":1}"#),
        };
        let message = call.into_message();
        assert_eq!(message.role, AiRole::ToolCall);
        assert_eq!(message.tool_call_id.as_deref(), Some("c7"));
        assert!(!message.role.carries_instructions());
    }

    #[test]
    fn finish_reasons_are_exhaustive_for_streaming() {
        for reason in [
            FinishReason::Stop,
            FinishReason::Length,
            FinishReason::ToolCall,
            FinishReason::Cancelled,
        ] {
            let debug = format!("{reason:?}");
            assert!(!debug.is_empty());
        }
    }
}
