//! AI orchestrator (`§2`, `§14`).
//!
//! Sends requests through the provider registry with:
//! - *retry* limited to retryable error kinds,
//! - *fallback* to a secondary provider (primary always first),
//! - usage propagation for cost accounting.
//!
//! The orchestrator is intentionally library-like: it never authorizes tool
//! execution itself; tools flow to the server-side tool system in a later
//! slice.

use crate::error::{AiError, AiErrorKind, AiResult, AiUsage};
use crate::events::{AiEventKind, EventSink};
use crate::metrics::{AiMetrics, UsageKey};
use crate::model::ProviderId;
use crate::registry::AiProviderRegistry;
use crate::request::{AiRequest, AiResponse};
use std::sync::Arc;
use std::time::Instant;

/// Retry/turnaround policy.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Attempts on the primary provider after the first failure.
    pub primary_retries: usize,
    /// Maximum transient failures tolerated before giving up entirely.
    pub max_total_attempts: usize,
}

impl RetryPolicy {
    /// Reasonable defaults: one retry, then fallback, then stop.
    #[must_use]
    pub const fn default_policy() -> Self {
        Self {
            primary_retries: 1,
            max_total_attempts: 4,
        }
    }
}

/// Provider orchestration with retry/fallback.
pub struct AiOrchestrator {
    policy: RetryPolicy,
    fallback: Option<ProviderId>,
    usage: AiUsage,
    fallback_uses: u64,
    retries: u64,
    metrics: Option<Arc<AiMetrics>>,
}

impl AiOrchestrator {
    #[must_use]
    pub const fn retries(&self) -> u64 {
        self.retries
    }

    #[must_use]
    pub const fn fallback_uses(&self) -> u64 {
        self.fallback_uses
    }

    #[must_use]
    pub const fn usage(&self) -> AiUsage {
        self.usage
    }
}

impl core::fmt::Debug for AiOrchestrator {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AiOrchestrator")
            .field("policy", &self.policy)
            .field("fallback", &self.fallback)
            .field("usage", &self.usage)
            .field("fallback_uses", &self.fallback_uses)
            .field("retries", &self.retries)
            .field("metrics", &self.metrics.is_some())
            .finish()
    }
}

impl Default for AiOrchestrator {
    fn default() -> Self {
        Self::new(RetryPolicy::default_policy(), None)
    }
}

impl AiOrchestrator {
    #[must_use]
    pub fn new(policy: RetryPolicy, fallback: Option<ProviderId>) -> Self {
        Self {
            policy,
            fallback,
            usage: AiUsage::default(),
            fallback_uses: 0,
            retries: 0,
            metrics: None,
        }
    }

    /// Attaches usage metrics recording (`§53`); the orchestrator itself
    /// stays provider-agnostic — metrics are pure counters.
    #[must_use]
    pub fn with_metrics(mut self, metrics: Arc<AiMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Executes a turn: retry transient failures on the primary, then fall
    /// back if a fallback provider exists. Non-retryable errors abort
    /// immediately (`§14`).
    ///
    /// # Errors
    /// The last error seen when every attempt is exhausted.
    pub async fn complete(
        &mut self,
        registry: &AiProviderRegistry,
        mut request: AiRequest,
    ) -> AiResult<AiResponse> {
        let primary = request.provider.clone();
        let mut attempt = 0usize;
        let mut last_error: Option<AiError> = None;

        for provider in [Some(primary.clone()), self.fallback.clone()] {
            let Some(provider_id) = provider else {
                continue;
            };
            let Some(provider) = registry.get(&provider_id) else {
                last_error = Some(AiError::new(
                    AiErrorKind::InvalidRequest,
                    "orchestrator.complete",
                    format!("provider {provider_id} is not registered"),
                ));
                continue;
            };
            // Only the primary retries; a fallback gets exactly one attempt
            // before the orchestrator gives up.
            let extra_tries = if provider_id == primary {
                self.policy.primary_retries
            } else {
                0
            };
            request.provider = provider_id.clone();

            for try_index in 0..=extra_tries {
                if attempt >= self.policy.max_total_attempts {
                    break;
                }
                attempt += 1;
                let _ = try_index;
                match provider.complete(request.clone()).await {
                    Ok(response) => {
                        self.usage.prompt_tokens += response.usage.prompt_tokens;
                        self.usage.completion_tokens += response.usage.completion_tokens;
                        if provider_id != primary {
                            self.fallback_uses += 1;
                        }
                        return Ok(response);
                    }
                    Err(err) => {
                        if !err.retries_permitted() {
                            return Err(err);
                        }
                        self.retries += 1;
                    }
                }
            }
            if attempt >= self.policy.max_total_attempts {
                break;
            }
        }

        Err(last_error.unwrap_or_else(|| {
            AiError::new(
                AiErrorKind::ProviderUnavailable,
                "orchestrator.complete",
                "no provider answered the request",
            )
        }))
    }

    /// [`AiOrchestrator::complete`] plus AI-event emission through a
    /// session-bound sink (`§50`). Only metadata is relayed — never
    /// prompt/response content.
    ///
    /// # Errors
    /// The last error seen when every attempt is exhausted.
    pub async fn complete_with_events(
        &mut self,
        registry: &AiProviderRegistry,
        mut request: AiRequest,
        events: Option<&EventSink>,
    ) -> AiResult<AiResponse> {
        let started = Instant::now();
        let primary = request.provider.clone();
        if let Some(metrics) = &self.metrics {
            metrics.request_started(&Self::key(&primary, &request.model));
        }
        if let Some(sink) = events {
            sink.emit(
                AiEventKind::AiRequestStarted,
                format!("model {} via {primary}", request.model),
            );
        }

        let mut attempt = 0usize;
        let mut last_error: Option<AiError> = None;
        let mut last_provider = primary.clone();

        for provider in [Some(primary.clone()), self.fallback.clone()] {
            let Some(provider_id) = provider else {
                continue;
            };
            let Some(provider) = registry.get(&provider_id) else {
                last_error = Some(AiError::new(
                    AiErrorKind::InvalidRequest,
                    "orchestrator.complete",
                    format!("provider {provider_id} is not registered"),
                ));
                continue;
            };
            let key = Self::key(&provider_id, &request.model);
            // Only the primary retries; a fallback gets exactly one attempt
            // before the orchestrator gives up.
            let extra_tries = if provider_id == primary {
                self.policy.primary_retries
            } else {
                0
            };
            request.provider = provider_id.clone();

            for try_index in 0..=extra_tries {
                if attempt >= self.policy.max_total_attempts {
                    break;
                }
                attempt += 1;
                let will_retry =
                    try_index < extra_tries && attempt < self.policy.max_total_attempts;
                match provider.complete(request.clone()).await {
                    Ok(response) => {
                        self.usage.prompt_tokens += response.usage.prompt_tokens;
                        self.usage.completion_tokens += response.usage.completion_tokens;
                        if provider_id != primary {
                            self.fallback_uses += 1;
                        }
                        if let Some(metrics) = &self.metrics {
                            metrics.completed(&key, started.elapsed(), None, &response.usage);
                        }
                        if let Some(sink) = events {
                            sink.emit(
                                AiEventKind::AiResponseCompleted,
                                format!(
                                    "tokens {}+{} via {provider_id}",
                                    response.usage.prompt_tokens, response.usage.completion_tokens
                                ),
                            );
                        }
                        return Ok(response);
                    }
                    Err(err) => {
                        if let Some(sink) = events {
                            sink.emit(
                                AiEventKind::AiProviderError,
                                format!("provider {provider_id}: {:?}", err.kind),
                            );
                        }
                        if !err.retries_permitted() {
                            if let Some(sink) = events {
                                if err.kind == AiErrorKind::Cancelled {
                                    sink.emit(
                                        AiEventKind::AiRequestCancelled,
                                        format!("model {}", request.model),
                                    );
                                }
                            }
                            return Err(err);
                        }
                        self.retries += 1;
                        last_provider = provider_id.clone();
                        if will_retry {
                            if let Some(metrics) = &self.metrics {
                                metrics.retried(&key);
                            }
                        }
                    }
                }
            }
            if attempt >= self.policy.max_total_attempts {
                break;
            }
        }

        let error = last_error.unwrap_or_else(|| {
            AiError::new(
                AiErrorKind::ProviderUnavailable,
                "orchestrator.complete",
                "no provider answered the request",
            )
        });
        if let Some(metrics) = &self.metrics {
            metrics.failed(&Self::key(&last_provider, &request.model));
        }
        Err(error)
    }

    fn key(provider: &ProviderId, model: &crate::model::ModelId) -> UsageKey {
        UsageKey::new(provider.as_str(), model.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::AiMessage;
    use crate::model::{AiCapabilities, AiModel, ModelId};
    use crate::provider::{AiProvider, AiStream, AiStreamEvent};
    use crate::request::FinishReason;
    use async_trait::async_trait;
    use std::sync::Arc;

    /// Flaky-on-demand provider: fails the first `fail_first` turns with
    /// `fail_kind`, then succeeds.
    struct FlakyProvider {
        id: ProviderId,
        fail_first: std::sync::atomic::AtomicU32,
        fail_kind: AiErrorKind,
        content: String,
    }

    impl FlakyProvider {
        fn new(id: &str, fail_first: u32, kind: AiErrorKind, content: &str) -> Self {
            Self {
                id: ProviderId::new(id),
                fail_first: std::sync::atomic::AtomicU32::new(fail_first),
                fail_kind: kind,
                content: content.into(),
            }
        }
    }

    #[async_trait]
    impl AiProvider for FlakyProvider {
        fn provider_id(&self) -> &ProviderId {
            &self.id
        }

        async fn list_models(&self) -> AiResult<Vec<AiModel>> {
            Ok(vec![AiModel::basic(
                self.id.clone(),
                ModelId::new("m"),
                8192,
            )])
        }

        async fn capabilities(&self, _model: &AiModel) -> AiCapabilities {
            AiCapabilities::base_chat()
        }

        async fn complete(&self, request: AiRequest) -> AiResult<AiResponse> {
            Ok(AiResponse {
                content: self.content.clone(),
                tool_calls: vec![],
                model: request.model,
                provider: request.provider,
                usage: AiUsage {
                    prompt_tokens: 5,
                    completion_tokens: 7,
                },
                finish: FinishReason::Stop,
            })
        }

        async fn stream(&self, _request: AiRequest) -> AiResult<AiStream> {
            let (tx, rx) = tokio::sync::mpsc::channel(4);
            let (cancel, _cancel_rx) = tokio::sync::mpsc::channel(1);
            tx.send(AiStreamEvent::Content {
                delta: self.content.clone(),
            })
            .await
            .ok();
            Ok(AiStream::new(rx, cancel))
        }
    }

    fn chat_request(provider: &str) -> AiRequest {
        AiRequest::chat(
            ProviderId::new(provider),
            ModelId::new("m"),
            vec![AiMessage::user("compute")],
        )
    }

    fn noisy_registry() -> AiProviderRegistry {
        let mut registry = AiProviderRegistry::new();
        registry.register(Arc::new(FlakyProvider::new(
            "primary",
            0,
            AiErrorKind::Transient,
            "primary-ok",
        )));
        registry.register(Arc::new(FlakyProvider::new(
            "fallback",
            0,
            AiErrorKind::Transient,
            "fallback-ok",
        )));
        registry
    }

    #[tokio::test]
    async fn retries_transient_failures_on_the_primary_then_succeeds() {
        let mut registry = noisy_registry();
        // Replace the primary with a once-flaky binding.
        registry.register(Arc::new(FlakyProvider::new(
            "primary",
            1,
            AiErrorKind::Transient,
            "primary-ok",
        )));

        let mut orchestrator = AiOrchestrator::new(RetryPolicy::default_policy(), None);
        let reply = orchestrator
            .complete(&registry, chat_request("primary"))
            .await
            .unwrap();
        assert_eq!(reply.content, "primary-ok");
        assert_eq!(orchestrator.retries(), 1);
        // Successes still accrue usage for cost accounting (`§53`).
        assert_eq!(orchestrator.usage().total(), 12);
    }

    #[tokio::test]
    async fn never_retries_cancellation_or_denial() {
        for kind in [AiErrorKind::Cancelled, AiErrorKind::AuthDenied] {
            let mut registry = noisy_registry();
            registry.register(Arc::new(FlakyProvider::new("primary", 5, kind, "never")));
            let mut orchestrator = AiOrchestrator::new(RetryPolicy::default_policy(), None);
            let err = orchestrator
                .complete(&registry, chat_request("primary"))
                .await
                .unwrap_err();
            assert_eq!(err.kind, kind);
            assert_eq!(orchestrator.retries(), 0);
        }
    }

    #[tokio::test]
    async fn exhausted_primary_falls_back_to_the_secondary() {
        let mut registry = noisy_registry();
        registry.register(Arc::new(FlakyProvider::new(
            "primary",
            2,
            AiErrorKind::Transient,
            "primary-ok",
        )));

        let mut orchestrator = AiOrchestrator::new(
            RetryPolicy::default_policy(),
            Some(ProviderId::new("fallback")),
        );
        let reply = orchestrator
            .complete(&registry, chat_request("primary"))
            .await
            .unwrap();
        assert_eq!(reply.content, "fallback-ok");
        assert_eq!(orchestrator.fallback_uses(), 1);
        assert_eq!(orchestrator.retries(), 2);
    }

    #[tokio::test]
    async fn unregistered_fallback_surfaces_a_clear_error() {
        let mut registry = noisy_registry();
        registry.register(Arc::new(FlakyProvider::new(
            "primary",
            2,
            AiErrorKind::Transient,
            "primary-ok",
        )));

        let mut orchestrator = AiOrchestrator::new(
            RetryPolicy::default_policy(),
            Some(ProviderId::new("ghost")),
        );
        let err = orchestrator
            .complete(&registry, chat_request("primary"))
            .await
            .unwrap_err();
        assert_eq!(err.kind, AiErrorKind::InvalidRequest);
        assert!(err.message.contains("ghost"));
    }

    #[tokio::test]
    async fn metrics_record_the_success_slice_with_tokens() {
        let registry = noisy_registry();
        let metrics = Arc::new(crate::metrics::AiMetrics::new());
        let mut orchestrator =
            AiOrchestrator::new(RetryPolicy::default_policy(), None).with_metrics(metrics.clone());
        let reply = orchestrator
            .complete_with_events(&registry, chat_request("primary"), None)
            .await
            .unwrap();
        assert_eq!(reply.content, "primary-ok");
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.requests, 1);
        assert_eq!(snapshot.completions, 1);
        assert_eq!(snapshot.failures, 0);
        let usage = &snapshot.per_model[&crate::metrics::UsageKey::new("primary", "m")];
        assert_eq!(usage.prompt_tokens, 5);
        assert_eq!(usage.completion_tokens, 7);
        assert!(snapshot.completion_time > std::time::Duration::ZERO);
    }

    #[tokio::test]
    async fn metrics_record_retries_then_the_final_failure() {
        let mut registry = noisy_registry();
        registry.register(Arc::new(FlakyProvider::new(
            "primary",
            5,
            AiErrorKind::Transient,
            "unreached",
        )));
        let metrics = Arc::new(crate::metrics::AiMetrics::new());
        let mut orchestrator =
            AiOrchestrator::new(RetryPolicy::default_policy(), None).with_metrics(metrics.clone());
        assert!(orchestrator
            .complete_with_events(&registry, chat_request("primary"), None)
            .await
            .is_err());
        let snapshot = metrics.snapshot();
        // Default policy: primary_retries 1 -> two attempts, both retried;
        // the request exhausts into one aggregate failure record.
        assert_eq!(snapshot.requests, 1);
        assert_eq!(snapshot.retries, 2);
        assert_eq!(snapshot.completions, 0);
        assert_eq!(snapshot.failures, 1);
        let usage = &snapshot.per_model[&crate::metrics::UsageKey::new("primary", "m")];
        assert_eq!(usage.retries, 2);
        assert_eq!(usage.failures, 1);
    }
}
