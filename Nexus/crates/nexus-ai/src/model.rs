//! AI model abstraction, provider-independent (`§5`).
//!
//! A model is data: discovery, capabilities, cost, and availability live on
//! the struct, never behind a vendor SDK. New capabilities are added as new
//! boolean fields *plus* the free-form `extras` set, so older clients keep
//! compiling when newer providers advertise future abilities.

use core::fmt;

/// Identifies a model within a provider (e.g. `"llama3-8b"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModelId(String);

impl ModelId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identifies a provider (e.g. `"local"`, `"acme-cloud"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProviderId(String);

impl ProviderId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Abilities a model may advertise.
///
/// Known capabilities are explicit fields so Rust callers get exhaustive
/// checks today; *future* capabilities ride along in `extras` without a
/// breaking change (`§5`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AiCapabilities {
    pub chat: bool,
    pub code_generation: bool,
    pub tool_calling: bool,
    pub streaming: bool,
    pub vision: bool,
    pub embeddings: bool,
    #[allow(clippy::struct_field_names)]
    pub structured_output: bool,
    pub reasoning: bool,
    /// Future capabilities, name-keyed.
    pub extras: Vec<String>,
}

impl AiCapabilities {
    /// Chat-only baseline: no tools, no streaming.
    #[must_use]
    pub fn base_chat() -> Self {
        Self {
            chat: true,
            ..Self::default()
        }
    }

    /// Full agentic profile: chat + generation + tools + streaming.
    #[must_use]
    pub fn agentic() -> Self {
        Self {
            chat: true,
            code_generation: true,
            tool_calling: true,
            streaming: true,
            ..Self::default()
        }
    }

    /// Whether an `extra` capability name is advertised.
    #[must_use]
    pub fn has_extra(&self, name: &str) -> bool {
        self.extras.iter().any(|n| n.eq_ignore_ascii_case(name))
    }
}

/// Cost per million tokens, in arbitrary provider units.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CostInformation {
    pub input_per_million: Option<u64>,
    pub output_per_million: Option<u64>,
}

impl CostInformation {
    /// Cost of an exchange, `None` when the model is unbilled.
    #[must_use]
    pub fn cost_of(&self, usage: &crate::error::AiUsage) -> Option<u64> {
        let input = self.input_per_million?.checked_mul(usage.prompt_tokens)? / 1_000_000;
        let output = self
            .output_per_million?
            .checked_mul(usage.completion_tokens)?
            / 1_000_000;
        Some(input + output)
    }
}

/// A model descriptor, fully provider-independent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiModel {
    pub model_id: ModelId,
    pub provider_id: ProviderId,
    pub display_name: String,
    /// Total context window the model accepts, in tokens.
    pub context_window: u64,
    /// Maximum tokens the model may generate.
    pub max_output_tokens: u64,
    pub capabilities: AiCapabilities,
    pub cost: CostInformation,
    /// `true` for locally served models that keep working offline (`§8`).
    pub local: bool,
    /// Provider model-version label, free-form.
    pub version: String,
    /// Whether the provider currently serves the model.
    pub available: bool,
}

impl AiModel {
    /// A minimal model descriptor.
    #[must_use]
    pub fn basic(provider: ProviderId, id: ModelId, context_window: u64) -> Self {
        Self {
            model_id: id,
            provider_id: provider,
            display_name: String::new(),
            context_window,
            max_output_tokens: 0,
            capabilities: AiCapabilities::default(),
            cost: CostInformation::default(),
            local: false,
            version: String::from("1.0.0"),
            available: true,
        }
    }

    /// Cheap preflight: whether a request of `prompt_tokens` fits the
    /// window while leaving room for output.
    #[must_use]
    pub fn fits(&self, prompt_tokens: u64, output_headroom: u64) -> bool {
        self.available && self.context_window.saturating_sub(prompt_tokens) >= output_headroom
    }
}

impl fmt::Display for AiModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.display_name.is_empty() {
            true => write!(f, "{}/{}", self.provider_id, self.model_id),
            false => f.write_str(&self.display_name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_render_and_sort() {
        let mut ids = [ModelId::new("zeta"), ModelId::new("alpha")];
        ids.sort();
        assert_eq!(ids[0].as_str(), "alpha");
        assert_eq!(ProviderId::new("local").to_string(), "local");
    }

    #[test]
    fn capability_profile_defaults_match_baseline() {
        let base = AiCapabilities::base_chat();
        assert!(base.chat);
        assert!(!base.tool_calling);
        assert!(!base.streaming);
        assert!(!base.has_extra("vision-plus"));

        let agentic = AiCapabilities::agentic();
        assert!(agentic.chat);
        assert!(agentic.code_generation);
        assert!(agentic.tool_calling);
        assert!(agentic.streaming);
        assert!(!agentic.vision);
    }

    #[test]
    fn future_capabilities_extend_extras_without_breaking() {
        let mut caps = AiCapabilities::agentic();
        caps.extras.push(String::from("vision-plus"));
        assert!(caps.has_extra("VISION-PLUS"));
        assert!(!caps.has_extra("other"));
    }

    #[test]
    fn context_preflight_is_conservative() {
        let mut model = AiModel::basic(ProviderId::new("local"), ModelId::new("m1"), 8_192);
        assert!(model.fits(100, 512));
        assert!(!model.fits(8_000, 1_000));

        model.available = false;
        assert!(!model.fits(0, 0));
    }

    #[test]
    fn cost_information_computes_the_bill() {
        let mut model = AiModel::basic(ProviderId::new("cloud"), ModelId::new("gpt-x"), 1_000_000);
        assert_eq!(model.to_string(), "cloud/gpt-x");
        model.display_name = String::from("GPT-X");
        assert_eq!(model.to_string(), "GPT-X");

        let cost = CostInformation {
            input_per_million: Some(1),
            output_per_million: Some(3),
        };
        let usage = crate::error::AiUsage {
            prompt_tokens: 500_000,
            completion_tokens: 2_000_000,
        };
        assert_eq!(cost.cost_of(&usage), Some(6));
        assert_eq!(CostInformation::default().cost_of(&usage), None);
    }
}
