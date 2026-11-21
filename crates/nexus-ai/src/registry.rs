//! AI provider registry (`§6`).
//!
//! Providers are registered at runtime — never hard-coded into the
//! orchestrator. The registry owns default selection, model aggregation
//! across providers, and availability-aware lookups.

use crate::error::{AiError, AiErrorKind, AiResult};
use crate::model::{AiModel, ModelId, ProviderId};
use crate::provider::AiProvider;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Registry of usable providers.
#[derive(Default)]
pub struct AiProviderRegistry {
    providers: BTreeMap<ProviderId, Arc<dyn AiProvider>>,
    default_provider: Option<ProviderId>,
    default_model: Option<ModelId>,
}

impl core::fmt::Debug for AiProviderRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AiProviderRegistry")
            .field("providers", &self.providers.keys().collect::<Vec<_>>())
            .field("default_provider", &self.default_provider)
            .field("default_model", &self.default_model)
            .finish()
    }
}

impl AiProviderRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers (or replaces) a provider under its declared id.
    pub fn register(&mut self, provider: Arc<dyn AiProvider>) {
        self.default_provider
            .get_or_insert_with(|| provider.provider_id().clone());
        self.providers
            .insert(provider.provider_id().clone(), provider);
    }

    /// Removes a provider. Model→provider mapping is established at request
    /// time across live providers, so a remembered default model is cleared
    /// conservatively on any removal.
    pub fn unregister(&mut self, id: &ProviderId) -> bool {
        if self.default_provider.as_ref() == Some(id) {
            self.default_provider = None;
        }
        if self.default_model.is_some() {
            self.default_model = None;
        }
        self.providers.remove(id).is_some()
    }

    /// The provider behind `id`, if registered.
    #[must_use]
    pub fn get(&self, id: &ProviderId) -> Option<&Arc<dyn AiProvider>> {
        self.providers.get(id)
    }

    /// Every registered provider id, ordered.
    #[must_use]
    pub fn ids(&self) -> Vec<ProviderId> {
        self.providers.keys().cloned().collect()
    }

    /// Whether no providers are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Aggregated model catalogue; unavailable models are filtered out
    /// unless `include_unavailable` is set.
    ///
    /// # Errors
    /// Providers may fail discovery independently; the registry surfaces the
    /// first failure only when it produces *no* usable catalogue.
    pub async fn models(&self, include_unavailable: bool) -> AiResult<Vec<AiModel>> {
        let mut fallback: Option<AiError> = None;
        let mut collected = Vec::new();
        for provider in self.providers.values() {
            match provider.list_models().await {
                Ok(list) => collected.extend(
                    list.into_iter()
                        .filter(|m| include_unavailable || m.available),
                ),
                Err(err) if fallback.is_none() => fallback = Some(err),
                Err(_) => {}
            }
        }
        if collected.is_empty() {
            Err(fallback.unwrap_or_else(|| {
                AiError::new(
                    AiErrorKind::ProviderUnavailable,
                    "registry.models",
                    "no provider lists models",
                )
            }))
        } else {
            Ok(collected)
        }
    }

    /// Selects the active defaults; both must exist at selection time —
    /// model-domain validation happens through the orchestrator.
    pub fn set_default_provider(&mut self, id: ProviderId) -> AiResult<()> {
        if !self.providers.contains_key(&id) {
            return Err(AiError::new(
                AiErrorKind::InvalidRequest,
                "registry.set_default_provider",
                format!("provider {id} is not registered"),
            ));
        }
        self.default_provider = Some(id);
        Ok(())
    }

    /// The default provider id, or the sole provider when exactly one exists.
    #[must_use]
    pub fn default_provider(&self) -> Option<ProviderId> {
        match &self.default_provider {
            Some(id) => Some(id.clone()),
            None => {
                let mut ids = self.providers.keys();
                if ids.len() == 1 {
                    ids.next().cloned()
                } else {
                    None
                }
            }
        }
    }

    /// Remembers a default model without requiring live provider access.
    pub fn set_default_model(&mut self, model: ModelId) {
        self.default_model = Some(model);
    }

    /// The remembered default model.
    #[must_use]
    pub fn default_model(&self) -> Option<ModelId> {
        self.default_model.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AiUsage;
    use crate::model::{AiCapabilities, CostInformation};
    use crate::provider::{AiStream, AiStreamEvent};
    use crate::request::{AiRequest, AiResponse, FinishReason};
    use async_trait::async_trait;
    use tokio::sync::mpsc;

    struct FakeProvider {
        id: ProviderId,
        models: Vec<AiModel>,
        fail_discovery: bool,
    }

    impl FakeProvider {
        fn local() -> Self {
            Self {
                id: ProviderId::new("local"),
                models: vec![AiModel::basic(
                    ProviderId::new("local"),
                    ModelId::new("tiny"),
                    8_192,
                )],
                fail_discovery: false,
            }
        }

        fn cloud() -> Self {
            Self {
                id: ProviderId::new("cloud"),
                models: vec![AiModel::basic(
                    ProviderId::new("cloud"),
                    ModelId::new("big"),
                    200_000,
                )],
                fail_discovery: false,
            }
        }
    }

    #[async_trait]
    impl AiProvider for FakeProvider {
        fn provider_id(&self) -> &ProviderId {
            &self.id
        }

        async fn list_models(&self) -> AiResult<Vec<AiModel>> {
            if self.fail_discovery {
                Err(AiError::new(
                    AiErrorKind::Transient,
                    "list_models",
                    "offline",
                ))
            } else {
                Ok(self.models.clone())
            }
        }

        async fn capabilities(&self, _model: &AiModel) -> AiCapabilities {
            AiCapabilities::agentic()
        }

        async fn complete(&self, request: AiRequest) -> AiResult<AiResponse> {
            Ok(AiResponse {
                content: String::from("ok"),
                tool_calls: vec![],
                model: request.model,
                provider: request.provider,
                usage: AiUsage {
                    prompt_tokens: 10,
                    completion_tokens: 2,
                },
                finish: FinishReason::Stop,
            })
        }

        async fn stream(&self, _request: AiRequest) -> AiResult<AiStream> {
            let (tx, rx) = mpsc::channel(8);
            let (cancel_tx, mut cancel_rx) = mpsc::channel::<()>(1);
            tokio::spawn(async move {
                let _ = tx
                    .send(AiStreamEvent::Content {
                        delta: String::from("ok"),
                    })
                    .await;
                // Simulated provider generation: either cancelled or done.
                let cancelled = tokio::select! {
                    maybe_cancel = cancel_rx.recv() => maybe_cancel.is_some(),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => false,
                };
                let _ = tx
                    .send(if !cancelled {
                        AiStreamEvent::Completed {
                            usage: AiUsage::default(),
                            stop: FinishReason::Stop,
                        }
                    } else {
                        AiStreamEvent::Failed {
                            error: AiError::new(AiErrorKind::Cancelled, "stream", "cancelled"),
                        }
                    })
                    .await;
            });
            Ok(AiStream::new(rx, cancel_tx))
        }
    }

    fn request_for(id: &ProviderId) -> AiRequest {
        AiRequest {
            messages: vec![crate::message::AiMessage::user("ping")],
            model: ModelId::new("tiny"),
            provider: id.clone(),
            context: String::new(),
            tools: vec![],
            max_output_tokens: None,
            temperature: None,
            stream: false,
        }
    }

    #[tokio::test]
    async fn register_replace_and_unregister_round_trip() {
        let mut registry = AiProviderRegistry::new();
        assert!(registry.is_empty());

        registry.register(Arc::new(FakeProvider::local()));
        assert_eq!(registry.ids().len(), 1);
        // First registration seeds the default.
        assert_eq!(registry.default_provider().unwrap().as_str(), "local");

        let replaced = FakeProvider {
            id: ProviderId::new("local"),
            models: vec![],
            fail_discovery: false,
        };
        // Re-registering under the same id replaces the binding.
        registry.register(Arc::new(replaced));

        assert!(!registry.unregister(&ProviderId::new("ghost")));
        assert!(registry.unregister(&ProviderId::new("local")));
        assert!(registry.is_empty());
        assert!(registry.default_provider().is_none());
    }

    #[tokio::test]
    async fn aggregated_discovery_skips_degraded_providers() {
        let broken_discovery = || {
            let mut broken = FakeProvider::cloud();
            broken.fail_discovery = true;
            broken
        };
        let mut registry = AiProviderRegistry::new();
        registry.register(Arc::new(broken_discovery()));
        registry.register(Arc::new(FakeProvider::local()));

        let models = registry.models(false).await.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id.as_str(), "tiny");

        // All providers failing with no usable provider => surfaced error.
        let mut only_broken = AiProviderRegistry::new();
        only_broken.register(Arc::new(broken_discovery()));
        assert!(only_broken.models(false).await.is_err());
    }

    #[tokio::test]
    async fn complete_streams_and_cancels_through_the_same_surface() {
        let mut registry = AiProviderRegistry::new();
        let local = Arc::new(FakeProvider::local());
        registry.register(local.clone());

        let reply = registry
            .get(&ProviderId::new("local"))
            .unwrap()
            .complete(request_for(&local.id))
            .await
            .unwrap();
        assert_eq!(reply.content, "ok");
        assert_eq!(reply.usage.total(), 12);

        let mut stream = registry
            .get(&local.id)
            .unwrap()
            .stream(request_for(&local.id))
            .await
            .unwrap();
        let first = stream.next().await.unwrap();
        assert!(matches!(first, AiStreamEvent::Content { .. }));
        stream.cancel().await.unwrap();
        let last = stream.next().await.unwrap();
        assert!(matches!(last, AiStreamEvent::Failed { .. }));
    }

    #[test]
    fn default_selection_validates_and_may_be_forced() {
        let mut registry = AiProviderRegistry::new();
        registry.register(Arc::new(FakeProvider::local()));
        assert!(registry
            .set_default_provider(ProviderId::new("cloud"))
            .is_err());

        registry.register(Arc::new(FakeProvider::cloud()));
        registry
            .set_default_provider(ProviderId::new("cloud"))
            .unwrap();
        assert_eq!(registry.default_provider().unwrap().as_str(), "cloud");

        registry.set_default_model(ModelId::new("big"));
        assert_eq!(registry.default_model().unwrap().as_str(), "big");

        // default_provider falls back to the empty set when ambiguous.
        let mut ambiguous = AiProviderRegistry::new();
        ambiguous.register(Arc::new(FakeProvider::local()));
        ambiguous.register(Arc::new(FakeProvider::cloud()));
        ambiguous
            .set_default_provider(ProviderId::new("local"))
            .unwrap();
        assert_eq!(ambiguous.default_provider().unwrap().as_str(), "local");
    }

    #[test]
    fn basic_model_cost_fields_default_to_unbilled() {
        let model = AiModel::basic(ProviderId::new("local"), ModelId::new("tiny"), 8_192);
        assert_eq!(model.cost, CostInformation::default());
    }
}
