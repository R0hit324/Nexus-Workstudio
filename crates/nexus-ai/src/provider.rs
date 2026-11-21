//! The provider abstraction (`§4`, `§13`).
//!
//! One `async` surface every local/cloud adapter implements. Vendor-specific
//! conversion code lives behind this trait only — orchestrators never name a
//! vendor (`§85.10–11`).

use crate::error::{AiError, AiResult};
use crate::model::{AiCapabilities, AiModel, ProviderId};
use crate::request::{AiRequest, AiResponse};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Incremental stream events; `content` deltas and tool calls flow in order.
#[derive(Debug, Clone)]
pub enum AiStreamEvent {
    /// The first turn arrived; `model` echoes what answered.
    Started { model: Option<String> },
    /// Content chunk.
    Content { delta: String },
    /// The model wants one tool executed.
    ToolCall { id: String, arguments: String },
    /// Stream completed with final usage.
    Completed {
        usage: crate::error::AiUsage,
        stop: crate::request::FinishReason,
    },
    /// Stream failed; the orchestrator decides retry/fallback.
    Failed { error: AiError },
}

/// A handle that multiplexes intents over an active stream.
pub struct AiStream {
    receiver: mpsc::Receiver<AiStreamEvent>,
    cancel: mpsc::Sender<()>,
    tap: Option<StreamTap>,
}

impl core::fmt::Debug for AiStream {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AiStream")
            .field("open", &self.receiver.len())
            .field("tapped", &self.tap.is_some())
            .finish()
    }
}

/// Observes stream events at consumption time (`§50` ResponseChunk is
/// ephemeral by policy; taps are the only in-frame hook that ever sees it).
pub type StreamTap = Arc<dyn Fn(&AiStreamEvent) + Send + Sync>;

impl AiStream {
    /// Wraps an in-flight provider stream.
    pub fn new(receiver: mpsc::Receiver<AiStreamEvent>, cancel: mpsc::Sender<()>) -> Self {
        Self {
            receiver,
            cancel,
            tap: None,
        }
    }

    /// Attaches an observer fired for every event consumed through
    /// [`AiStream::next`] — hosts bridge this to metrics/ledger emissions
    /// (chunk counting, time-to-first-token, completion usage).
    #[must_use]
    pub fn tapped(mut self, tap: StreamTap) -> Self {
        self.tap = Some(tap);
        self
    }

    /// Next event, or `None` once the stream closed.
    pub async fn next(&mut self) -> Option<AiStreamEvent> {
        let event = self.receiver.recv().await?;
        if let Some(tap) = &self.tap {
            tap(&event);
        }
        Some(event)
    }

    /// Cancellation: the provider should terminate the underlying request
    /// rather than merely stop routing tokens (`§73`).
    ///
    /// # Errors
    /// `Err` when the stream/cancel channel is already closed (the request
    /// finished first) — harmless and not retried.
    pub async fn cancel(&mut self) -> Result<(), AiError> {
        self.cancel
            .send(())
            .await
            .map_err(|_| AiError::internal("stream", "cancellation channel already closed"))
    }

    /// Closes the receive side early (TUI abandoned the stream).
    pub fn shutdown(mut self) {
        self.receiver.close();
    }
}

/// Vendor-independent provider surface.
#[async_trait]
pub trait AiProvider: Send + Sync {
    /// Stable provider identity used in requests (`ProviderId`s must match
    /// the registry entry this instance was registered under).
    fn provider_id(&self) -> &ProviderId;

    /// Model discovery (`§4`): include availability mirrors.
    async fn list_models(&self) -> AiResult<Vec<AiModel>>;

    /// Capability lookup for one model advertised by this provider.
    async fn capabilities(&self, model: &AiModel) -> AiCapabilities;

    /// Complete a turn in one shot (no partial content).
    ///
    /// # Errors
    /// Any [`AiErrorKind`] the underlying provider reports.
    async fn complete(&self, request: AiRequest) -> AiResult<AiResponse>;

    /// Stream a turn's tokens as they arrive.
    ///
    /// # Errors
    /// Errors raised *before* the stream establishes (auth, model missing).
    async fn stream(&self, request: AiRequest) -> AiResult<AiStream>;
}

#[cfg(test)]
mod tests {
    use super::{AiStream, AiStreamEvent, StreamTap};
    use crate::error::{AiError, AiErrorKind};
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    async fn channel_pair(
        buffer: usize,
    ) -> (mpsc::Sender<AiStreamEvent>, mpsc::Receiver<AiStreamEvent>) {
        mpsc::channel(buffer)
    }

    #[tokio::test]
    async fn tapped_stream_forward_events_in_consume_order() {
        let (tx, rx) = channel_pair(8).await;
        let observed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let seen = observed.clone();
        let tap: StreamTap = Arc::new(move |event: &AiStreamEvent| {
            let label = match event {
                AiStreamEvent::Started { .. } => "started",
                AiStreamEvent::Content { delta } => {
                    assert!(delta.contains("word"));
                    "content"
                }
                AiStreamEvent::ToolCall { .. } => "toolcall",
                AiStreamEvent::Completed { .. } => "completed",
                AiStreamEvent::Failed { .. } => "failed",
            };
            seen.lock().unwrap().push(String::from(label));
        });
        let mut stream = AiStream::new(rx, mpsc::channel(1).0).tapped(tap);
        // Send all four events, then drop the sender so recv returns None.
        tx.send(AiStreamEvent::Started {
            model: Some(String::from("m")),
        })
        .await
        .ok();
        tx.send(AiStreamEvent::Content {
            delta: String::from("one word"),
        })
        .await
        .ok();
        tx.send(AiStreamEvent::Content {
            delta: String::from(" two words"),
        })
        .await
        .ok();
        tx.send(AiStreamEvent::Completed {
            usage: crate::error::AiUsage {
                prompt_tokens: 9,
                completion_tokens: 3,
            },
            stop: crate::request::FinishReason::Stop,
        })
        .await
        .ok();
        drop(tx);
        // Drain until recv returns None.
        while stream.next().await.is_some() {}
        let labels = observed.lock().unwrap();
        assert_eq!(
            *labels,
            [
                String::from("started"),
                String::from("content"),
                String::from("content"),
                String::from("completed")
            ]
        );
    }

    #[tokio::test]
    async fn tapped_stream_sees_failure_events_too() {
        let (tx, rx) = channel_pair(4).await;
        let observed: Arc<Mutex<Option<AiErrorKind>>> = Arc::new(Mutex::new(None));
        let seen = observed.clone();
        let tap: StreamTap = Arc::new(move |event: &AiStreamEvent| {
            if let AiStreamEvent::Failed { error } = event {
                *seen.lock().unwrap() = Some(error.kind);
            }
        });
        tx.send(AiStreamEvent::Failed {
            error: AiError::new(
                AiErrorKind::ProviderUnavailable,
                "stream",
                "mocked provider outage",
            ),
        })
        .await
        .ok();
        drop(tx);
        let (cancel_tx, _) = mpsc::channel(1);
        let mut stream = AiStream::new(rx, cancel_tx).tapped(tap);
        while stream.next().await.is_some() {}
        assert_eq!(
            *observed.lock().unwrap(),
            Some(AiErrorKind::ProviderUnavailable)
        );
    }

    #[tokio::test]
    async fn untapped_streams_stay_driven_the_same_way() {
        let (tx, rx) = channel_pair(2).await;
        tx.send(AiStreamEvent::Content {
            delta: String::from("x"),
        })
        .await
        .ok();
        drop(tx);
        let mut stream = AiStream::new(rx, mpsc::channel(1).0);
        let first = stream.next().await;
        assert!(first.is_some());
        assert!(stream.next().await.is_none());
    }
}
