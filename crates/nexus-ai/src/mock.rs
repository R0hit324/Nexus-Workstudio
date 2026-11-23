//! Deterministic mock AI provider (`§77`).
//!
//! Lets tests script responses, tool calls, streaming sequences, delays,
//! failures, timeouts, and cancellation — no external APIs involved.

use crate::error::{AiError, AiErrorKind, AiResult, AiUsage};
use crate::model::{AiCapabilities, AiModel, ModelId, ProviderId};
use crate::provider::{AiProvider, AiStream, AiStreamEvent};
use crate::request::{AiRequest, AiResponse, FinishReason};
use async_trait::async_trait;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

/// One scripted "turn" the mock replays, in order, per call.
#[derive(Debug, Clone)]
pub enum MockTurn {
    /// Static completion with usage defaults.
    Content(String),
    /// Completion representing a tool request.
    ToolCalls(Vec<(String, String)>),
    /// Fail deterministically with this error kind.
    Fail(AiErrorKind, String),
    /// Pause before answering (drives timeout/cancel tests).
    Delay(Duration, Box<MockTurn>),
    /// Streaming script: replay these exact stream events.
    Streamed(Vec<StreamScript>),
}

/// One event inside a scripted stream.
#[derive(Debug, Clone)]
pub enum StreamScript {
    Delta(String),
    Tool(String, String),
    Complete(u64, u64, FinishReason),
    Fail(String),
}

/// Builder-style configuration so tests read declaratively.
#[derive(Debug, Clone, Default)]
pub struct MockConfig {
    turns: Vec<MockTurn>,
}

impl MockConfig {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends turns; the provider replays them in order, the last one
    /// sticking when calls exceed the script.
    #[must_use]
    pub fn turn(mut self, turn: MockTurn) -> Self {
        self.turns.push(turn);
        self
    }
}

/// The deterministic mock [`AiProvider`].
pub struct MockProvider {
    id: ProviderId,
    config: MockConfig,
    /// How many turns have been consumed (shared across clones: tests can
    /// share one provider through `Arc<dyn AiProvider>`).
    cursor: AtomicU32,
    /// Total requests served — diagnostics for integrations.
    served: AtomicU32,
    /// Whether `stream()` should report failure to establish.
    fail_establishment: std::sync::atomic::AtomicBool,
}

impl MockProvider {
    #[must_use]
    pub fn new(id: impl Into<String>, config: MockConfig) -> Self {
        Self {
            id: ProviderId::new(id.into()),
            config,
            cursor: AtomicU32::new(0),
            served: AtomicU32::new(0),
            fail_establishment: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Discovery reports a fixed catalogue.
    #[must_use]
    pub fn catalogue() -> Vec<AiModel> {
        let mut basic = AiModel::basic(ProviderId::new("mock"), ModelId::new("mock-small"), 8_192);
        basic.display_name = String::from("Mock Small");
        basic.local = true;
        basic.capabilities = AiCapabilities::agentic();
        let mut big = AiModel::basic(ProviderId::new("mock"), ModelId::new("mock-large"), 32_768);
        big.display_name = String::from("Mock Large");
        big.local = true;
        big.capabilities = AiCapabilities::agentic();
        vec![basic, big]
    }

    /// Flags stream establishment failures (before first event).
    pub fn fail_stream_establishment(&self, enabled: bool) {
        self.fail_establishment.store(enabled, Ordering::SeqCst);
    }

    async fn replay_turn(&self, request: &AiRequest) -> AiResult<AiResponse> {
        // Cursor advances monotonically; overflow sticks to the last script
        // entry so repeated calls stay deterministic.
        let index = self
            .cursor
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                Some(n.saturating_add(1))
            })
            .unwrap_or(u32::MAX);
        let turn = match self.config.turns.split_last() {
            Some((_last, prefix)) => prefix.get(index as usize),
            None => None,
        };
        let turn = turn
            .cloned()
            .or_else(|| self.config.turns.split_last().map(|(last, _)| last.clone()))
            .unwrap_or_else(|| MockTurn::Content(String::from("mock says hello")));
        self.evaluate(request, turn).await
    }

    async fn evaluate(&self, request: &AiRequest, turn: MockTurn) -> AiResult<AiResponse> {
        match turn {
            MockTurn::Content(text) => Ok(self.finish(request, text, vec![], FinishReason::Stop)),
            MockTurn::ToolCalls(calls) => {
                Ok(self.finish(request, String::new(), calls, FinishReason::ToolCall))
            }
            MockTurn::Fail(kind, message) => Err(AiError::new(kind, "mock.complete", message)),
            MockTurn::Delay(delay, inner) => {
                async move {
                    tokio::time::sleep(delay).await;
                    match *inner {
                        MockTurn::Content(text) => {
                            Ok(self.finish(request, text, vec![], FinishReason::Stop))
                        }
                        MockTurn::ToolCalls(calls) => {
                            Ok(self.finish(request, String::new(), calls, FinishReason::ToolCall))
                        }
                        MockTurn::Fail(kind, message) => {
                            Err(AiError::new(kind, "mock.complete", message))
                        }
                        _ => Ok(self.finish(
                            request,
                            String::from("delayed"),
                            vec![],
                            FinishReason::Stop,
                        )),
                    }
                }
                .await
            }
            MockTurn::Streamed(_) => {
                Ok(self.finish(request, String::new(), vec![], FinishReason::Stop))
            }
        }
    }

    fn finish(
        &self,
        request: &AiRequest,
        content: String,
        calls: Vec<(String, String)>,
        reason: FinishReason,
    ) -> AiResponse {
        // Return the catalog model id the request asked for so tests can
        // assert routing fidelity.
        self.served.fetch_add(1, Ordering::SeqCst);
        AiResponse {
            content,
            tool_calls: calls
                .into_iter()
                .map(|(id, arguments)| crate::request::AiToolInvocationRes { id, arguments })
                .collect(),
            model: request.model.clone(),
            provider: request.provider.clone(),
            usage: AiUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
            },
            finish: reason,
        }
    }

    /// Total turns served.
    #[must_use]
    pub fn served(&self) -> u32 {
        self.served.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl AiProvider for MockProvider {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }

    async fn list_models(&self) -> AiResult<Vec<AiModel>> {
        Ok(Self::catalogue())
    }

    async fn capabilities(&self, _model: &AiModel) -> AiCapabilities {
        AiCapabilities::agentic()
    }

    async fn complete(&self, request: AiRequest) -> AiResult<AiResponse> {
        self.replay_turn(&request).await
    }

    async fn stream(&self, _request: AiRequest) -> AiResult<AiStream> {
        if self.fail_establishment.load(Ordering::SeqCst) {
            return Err(AiError::new(
                AiErrorKind::ProviderUnavailable,
                "mock.stream",
                "cannot establish stream",
            ));
        }
        let (tx, rx) = mpsc::channel(16);
        let (cancel_tx, mut cancel_rx) = mpsc::channel::<()>(1);
        let index = self
            .cursor
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                Some(n.saturating_add(1))
            })
            .unwrap_or(u32::MAX);
        let turns_len = self.config.turns.len();
        let turn = match turns_len {
            0 => MockTurn::Content(String::from("mock streams")),
            _ => self.config.turns[index.min((turns_len - 1) as u32) as usize].clone(),
        };
        tokio::spawn(async move {
            // Respect establishment cancellation: the mock honours cancel
            // before its first delta to keep cancel-latency tests honest.
            let script: Vec<StreamScript> = match turn {
                MockTurn::Content(text) => vec![
                    StreamScript::Delta(text),
                    StreamScript::Complete(10, 5, FinishReason::Stop),
                ],
                MockTurn::Streamed(events) => events,
                MockTurn::ToolCalls(calls) => {
                    let mut events: Vec<StreamScript> = calls
                        .into_iter()
                        .map(|(id, args)| StreamScript::Tool(id, args))
                        .collect();
                    events.push(StreamScript::Complete(10, 5, FinishReason::ToolCall));
                    events
                }
                MockTurn::Fail(kind, message) => {
                    // Establishment error surfaces as a stream failure event.
                    let _ = tx
                        .send(AiStreamEvent::Failed {
                            error: AiError::new(kind, "mock.stream", message),
                        })
                        .await;
                    return;
                }
                MockTurn::Delay(delay, inner) => {
                    let wall = tokio::time::sleep(delay);
                    tokio::select! {
                        _ = wall => {}
                        _ = cancel_rx.recv() => {
                            let _ = tx
                                .send(AiStreamEvent::Failed {
                                    error: AiError::new(AiErrorKind::Cancelled, "mock.stream", "cancelled during delay"),
                                })
                                .await;
                            return;
                        }
                    }
                    match *inner {
                        MockTurn::Content(text) => vec![
                            StreamScript::Delta(text),
                            StreamScript::Complete(10, 5, FinishReason::Stop),
                        ],
                        _ => vec![
                            StreamScript::Delta(String::from("delayed")),
                            StreamScript::Complete(10, 5, FinishReason::Stop),
                        ],
                    }
                }
            };
            for event in script {
                // Each step checks for cancellation first (`§73`).
                if cancel_rx.try_recv().is_ok() {
                    let _ = tx
                        .send(AiStreamEvent::Failed {
                            error: AiError::new(AiErrorKind::Cancelled, "mock.stream", "cancelled"),
                        })
                        .await;
                    return;
                }
                let emit = match event {
                    StreamScript::Delta(delta) => AiStreamEvent::Content { delta },
                    StreamScript::Tool(id, args) => AiStreamEvent::ToolCall {
                        id,
                        arguments: args,
                    },
                    StreamScript::Complete(prompt, completion, stop) => AiStreamEvent::Completed {
                        usage: AiUsage {
                            prompt_tokens: prompt,
                            completion_tokens: completion,
                        },
                        stop,
                    },
                    StreamScript::Fail(message) => AiStreamEvent::Failed {
                        error: AiError::new(AiErrorKind::Transient, "mock.stream", message),
                    },
                };
                if tx.send(emit).await.is_err() {
                    // Receiver gone: TUI abandoned the stream.
                    return;
                }
            }
        });
        self.served.fetch_add(1, Ordering::SeqCst);
        Ok(AiStream::new(rx, cancel_tx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::AiMessage;

    fn request() -> AiRequest {
        AiRequest::chat(
            ProviderId::new("mock"),
            ModelId::new("mock-small"),
            vec![AiMessage::user("ping")],
        )
    }

    #[tokio::test]
    async fn scripted_turns_replay_in_order_then_stick() {
        let provider = MockProvider::new(
            "mock",
            MockConfig::new()
                .turn(MockTurn::Content(String::from("first")))
                .turn(MockTurn::Content(String::from("second"))),
        );
        let first = provider.complete(request()).await.unwrap();
        assert_eq!(first.content, "first");
        let second = provider.complete(request()).await.unwrap();
        assert_eq!(second.content, "second");
        // Overflow sticks to the last script entry.
        let third = provider.complete(request()).await.unwrap();
        assert_eq!(third.content, "second");
        assert_eq!(provider.served(), 3);
    }

    #[tokio::test]
    async fn tool_call_turns_round_trip_through_the_response() {
        let provider = MockProvider::new(
            "mock",
            MockConfig::new().turn(MockTurn::ToolCalls(vec![(
                String::from("call-1"),
                String::from(r#"{"path":"src/main.rs"}"#),
            )])),
        );
        let reply = provider.complete(request()).await.unwrap();
        assert_eq!(reply.finish, FinishReason::ToolCall);
        assert_eq!(reply.tool_calls.len(), 1);
        let turn = reply.tool_calls[0].clone().into_message();
        assert_eq!(turn.tool_call_id.as_deref(), Some("call-1"));
        assert!(!turn.role.carries_instructions());
    }

    #[tokio::test]
    async fn failures_surface_their_kinds() {
        let provider = MockProvider::new(
            "mock",
            MockConfig::new().turn(MockTurn::Fail(
                AiErrorKind::RateLimited,
                String::from("429"),
            )),
        );
        let err = provider.complete(request()).await.unwrap_err();
        assert_eq!(err.kind, AiErrorKind::RateLimited);
        assert!(err.retries_permitted());
    }

    #[tokio::test]
    async fn streams_replay_scripted_events_and_honour_cancel() {
        let provider = MockProvider::new(
            "mock",
            MockConfig::new().turn(MockTurn::Streamed(vec![
                StreamScript::Delta(String::from("hel")),
                StreamScript::Delta(String::from("lo")),
                StreamScript::Complete(3, 2, FinishReason::Stop),
            ])),
        );
        let mut stream = provider.stream(request()).await.unwrap();
        let mut seen = Vec::new();
        while let Some(event) = stream.next().await {
            seen.push(event);
            if seen.len() == 3 {
                break;
            }
        }
        assert!(matches!(seen[0], AiStreamEvent::Content { .. }));
        assert!(matches!(seen[2], AiStreamEvent::Completed { .. }));

        // A second stream from the same provider still replays (stuck last).
        let mut stream = provider.stream(request()).await.unwrap();
        stream.cancel().await.unwrap();
        let event = stream.next().await.unwrap();
        assert!(matches!(
            event,
            AiStreamEvent::Failed { error } if error.kind == AiErrorKind::Cancelled
        ));
        stream.shutdown();
    }

    #[tokio::test]
    async fn establishment_failure_keeps_the_orchestrator_in_business() {
        let provider = MockProvider::new("mock", MockConfig::new());
        provider.fail_stream_establishment(true);
        let err = provider.stream(request()).await.unwrap_err();
        assert_eq!(err.kind, AiErrorKind::ProviderUnavailable);
    }

    #[tokio::test]
    async fn discovery_and_capabilities_are_stable() {
        let provider = MockProvider::new("mock", MockConfig::new());
        let models = provider.list_models().await.unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].display_name, "Mock Small");
        assert_eq!(models[0].provider_id.as_str(), "mock");
        assert!(models.iter().all(|m| m.local));
        let caps = provider.capabilities(&models[1]).await;
        assert!(caps.tool_calling && caps.streaming);
    }
}
