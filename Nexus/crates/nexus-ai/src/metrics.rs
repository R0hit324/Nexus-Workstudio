//! AI usage metrics (`§53`): token/latency/tool/cost accounting per provider
//! and model. Counters are thread-safe; the orchestrator, tools, and hosts
//! all record into one shared [`AiMetrics`](AiMetrics) handle.
//!
//! Prompt/response text is never recorded here — only numbers and labels
//! ("do not log sensitive prompts by default", `§53`).

use crate::error::AiUsage;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

/// Stable key for per-model accounting.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UsageKey {
    pub provider: String,
    pub model: String,
}

impl UsageKey {
    #[must_use]
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }
}

impl core::fmt::Display for UsageKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}/{}", self.provider, self.model)
    }
}

/// Per-model rolling usage (`§53` request/token/cost block).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelUsage {
    pub requests: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub failures: u64,
    pub retries: u64,
    /// Estimated cost in the resolver's minor units (sum over calls).
    pub estimated_cost: Option<u64>,
}

impl ModelUsage {
    /// Total tokens as `Usage::total()` defines it.
    #[must_use]
    pub fn tokens(&self) -> u64 {
        self.prompt_tokens + self.completion_tokens
    }
}

/// Aggregated snapshot handed to hosts/TUI.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetricsSnapshot {
    pub requests: u64,
    pub completions: u64,
    pub failures: u64,
    pub retries: u64,
    pub tool_calls: u64,
    /// Total tool wall time across calls.
    pub tool_time: Duration,
    /// Total end-to-end completion latency across finished requests.
    pub completion_time: Duration,
    /// Time to first token, most recent observation.
    pub time_to_first_token: Option<Duration>,
    /// Fastest observed time to first token (sanity/quality signal).
    pub best_time_to_first_token: Option<Duration>,
    pub per_model: BTreeMap<UsageKey, ModelUsage>,
    /// Local-model bookkeeping, keyed provider/modellike (`§53` local block).
    pub local: BTreeMap<UsageKey, LocalModelStats>,
}

/// Local-model runtime stats (`§53`); hosts feed samplers, the metrics
/// store only what was reported.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalModelStats {
    pub load_time: Option<Duration>,
    /// Reported inference throughput (tokens/second), most recent value.
    pub throughput: Option<u64>,
}

impl LocalModelStats {
    fn observe_load(&mut self, load: Duration) {
        self.load_time = Some(load);
    }

    fn observe_throughput(&mut self, tokens_per_second: u64) {
        self.throughput = Some(tokens_per_second);
    }
}

#[derive(Debug, Default)]
struct MetricsCore {
    requests: u64,
    completions: u64,
    failures: u64,
    retries: u64,
    tool_calls: u64,
    tool_time: Duration,
    completion_time: Duration,
    ttft: Option<Duration>,
    best_ttft: Option<Duration>,
    per_model: BTreeMap<UsageKey, ModelUsage>,
    local: BTreeMap<UsageKey, LocalModelStats>,
}

/// Shared metrics recorder.
#[derive(Debug, Default)]
pub struct AiMetrics {
    core: Mutex<MetricsCore>,
}

impl AiMetrics {
    #[must_use]
    pub fn new() -> Self {
        Self {
            core: Mutex::new(MetricsCore::default()),
        }
    }

    /// Records the start of one request against a provider/model pair.
    pub fn request_started(&self, key: &UsageKey) {
        let mut core = self.core.lock().expect("metrics lock");
        core.requests += 1;
        core.per_model.entry(key.clone()).or_default().requests += 1;
    }

    /// Records one successful completion: latency, time-to-first-token and
    /// routed tokens from the provider's usage report.
    pub fn completed(
        &self,
        key: &UsageKey,
        latency: Duration,
        ttft: Option<Duration>,
        usage: &AiUsage,
    ) {
        let mut core = self.core.lock().expect("metrics lock");
        core.completions += 1;
        core.completion_time += latency;
        if let Some(ttft) = ttft {
            core.ttft = Some(ttft);
            let best = core.best_ttft.filter(|best| *best <= ttft).or(Some(ttft));
            core.best_ttft = best;
        }
        let entry = core.per_model.entry(key.clone()).or_default();
        entry.prompt_tokens += usage.prompt_tokens;
        entry.completion_tokens += usage.completion_tokens;
    }

    /// Records a request/tool failure (retryable attempts call this too).
    pub fn failed(&self, key: &UsageKey) {
        let mut core = self.core.lock().expect("metrics lock");
        core.failures += 1;
        core.per_model.entry(key.clone()).or_default().failures += 1;
    }

    /// Records one orchestrator retry against a provider/model pair.
    pub fn retried(&self, key: &UsageKey) {
        let mut core = self.core.lock().expect("metrics lock");
        core.retries += 1;
        core.per_model.entry(key.clone()).or_default().retries += 1;
    }

    /// Records one finished tool call with its wall time.
    pub fn tool_finished(&self, key: &UsageKey, elapsed: Duration) {
        let mut core = self.core.lock().expect("metrics lock");
        core.tool_calls += 1;
        core.tool_time += elapsed;
        let _ = key;
    }

    /// Records an estimated cost for a routed call, in whatever minor unit
    /// the cost resolver emits ([`crate::model::CostInformation`] defaults
    /// to USD per million tokens).
    pub fn cost_estimated(&self, key: &UsageKey, cost: u64) {
        let mut core = self.core.lock().expect("metrics lock");
        let entry = core.per_model.entry(key.clone()).or_default();
        entry.estimated_cost = Some(entry.estimated_cost.unwrap_or(0) + cost);
    }

    /// Estimates and records cost straight from a catalogue entry when the
    /// model carries cost information; no-op models stay unbilled.
    pub fn usage_cost(
        &self,
        key: &UsageKey,
        cost_of: &crate::model::CostInformation,
        usage: &AiUsage,
    ) {
        if let Some(cost) = cost_of.cost_of(usage) {
            self.cost_estimated(key, cost);
        }
    }

    /// Records local-model statistics observed by the host (`§53`).
    pub fn local_load(&self, key: &UsageKey, load: Duration) {
        let mut core = self.core.lock().expect("metrics lock");
        core.local
            .entry(key.clone())
            .or_default()
            .observe_load(load);
    }

    pub fn local_throughput(&self, key: &UsageKey, tokens_per_second: u64) {
        let mut core = self.core.lock().expect("metrics lock");
        core.local
            .entry(key.clone())
            .or_default()
            .observe_throughput(tokens_per_second);
    }

    /// Point-in-time aggregate. Durations sum monotonically.
    #[must_use]
    pub fn snapshot(&self) -> MetricsSnapshot {
        let core = self.core.lock().expect("metrics lock");
        MetricsSnapshot {
            requests: core.requests,
            completions: core.completions,
            failures: core.failures,
            retries: core.retries,
            tool_calls: core.tool_calls,
            tool_time: core.tool_time,
            completion_time: core.completion_time,
            time_to_first_token: core.ttft,
            best_time_to_first_token: core.best_ttft,
            per_model: core.per_model.clone(),
            local: core.local.clone(),
        }
    }

    /// Average completion latency across finished requests.
    #[must_use]
    pub fn average_completion_latency(&self) -> Option<Duration> {
        let core = self.core.lock().expect("metrics lock");
        if core.completions == 0 {
            return None;
        }
        Some(core.completion_time / core.completions as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::{AiMetrics, LocalModelStats, UsageKey};
    use crate::error::AiUsage;
    use crate::model::CostInformation;
    use std::time::Duration;

    fn small() -> AiUsage {
        AiUsage {
            prompt_tokens: 100,
            completion_tokens: 40,
        }
    }

    fn cost(input: u64, output: u64) -> CostInformation {
        CostInformation {
            input_per_million: Some(input),
            output_per_million: Some(output),
        }
    }

    #[test]
    fn request_and_completion_counters_roll_up() {
        let metrics = AiMetrics::new();
        let key = UsageKey::new("mock", "mock-large");
        metrics.request_started(&key);
        metrics.request_started(&key);
        metrics.completed(
            &key,
            Duration::from_millis(120),
            Some(Duration::from_millis(40)),
            &small(),
        );
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.requests, 2);
        assert_eq!(snapshot.completions, 1);
        assert_eq!(
            snapshot.time_to_first_token,
            Some(Duration::from_millis(40))
        );
        let usage = &snapshot.per_model[&key];
        assert_eq!(usage.requests, 2);
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 40);
        assert_eq!(usage.tokens(), 140);
        assert_eq!(snapshot.completion_time, Duration::from_millis(120));
    }

    #[test]
    fn best_time_to_first_token_keeps_the_fastest() {
        let metrics = AiMetrics::new();
        let key = UsageKey::new("mock", "mock-small");
        metrics.request_started(&key);
        metrics.completed(
            &key,
            Duration::from_millis(50),
            Some(Duration::from_millis(30)),
            &small(),
        );
        metrics.completed(
            &key,
            Duration::from_millis(50),
            Some(Duration::from_millis(12)),
            &small(),
        );
        metrics.completed(&key, Duration::from_millis(50), None, &small());
        let snapshot = metrics.snapshot();
        assert_eq!(
            snapshot.time_to_first_token,
            Some(Duration::from_millis(12))
        );
        assert_eq!(
            snapshot.best_time_to_first_token,
            Some(Duration::from_millis(12))
        );
    }

    #[test]
    fn failures_retries_and_tool_time_aggregate() {
        let metrics = AiMetrics::new();
        let key = UsageKey::new("mock", "mock-large");
        metrics.request_started(&key);
        metrics.failed(&key);
        metrics.retried(&key);
        metrics.completed(&key, Duration::from_millis(10), None, &small());
        metrics.tool_finished(&key, Duration::from_millis(15));
        metrics.tool_finished(&key, Duration::from_millis(25));
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.failures, 1);
        assert_eq!(snapshot.retries, 1);
        assert_eq!(snapshot.tool_calls, 2);
        assert_eq!(snapshot.tool_time, Duration::from_millis(40));
        let usage = &snapshot.per_model[&key];
        assert_eq!(usage.failures, 1);
        assert_eq!(usage.retries, 1);
        // Attributed to the model that actually served the failed try.
        assert_eq!(usage.requests, 1);
    }

    #[test]
    fn costs_estimate_from_usage_and_accumulate() {
        let metrics = AiMetrics::new();
        let key = UsageKey::new("mock", "mock-large");
        let cost = cost(30, 60);
        metrics.usage_cost(&key, &cost, &small());
        metrics.usage_cost(&key, &cost, &small());
        let billed = metrics.snapshot().per_model[&key].estimated_cost;
        assert!(billed.is_some(), "cost records when model carries pricing");
    }

    #[test]
    fn unbilled_models_stay_unbilled() {
        let metrics = AiMetrics::new();
        let key = UsageKey::new("mock", "free-tier");
        metrics.usage_cost(&key, &cost(0, 0), &small());
        assert_eq!(metrics.snapshot().per_model[&key].estimated_cost, Some(0));
        assert_eq!(metrics.snapshot().requests, 0);
    }

    #[test]
    fn local_model_stats_surface() {
        let metrics = AiMetrics::new();
        let key = UsageKey::new("ollama", "llama-local");
        metrics.local_load(&key, Duration::from_millis(900));
        metrics.local_throughput(&key, 43);
        let stats = metrics
            .snapshot()
            .local
            .get(&key)
            .cloned()
            .unwrap_or(LocalModelStats::default());
        assert_eq!(stats.load_time, Some(Duration::from_millis(900)));
        assert_eq!(stats.throughput, Some(43));
    }

    #[test]
    fn average_latency_requires_a_completion() {
        let metrics = AiMetrics::new();
        assert!(metrics.average_completion_latency().is_none());
        let key = UsageKey::new("mock", "mock-small");
        metrics.request_started(&key);
        metrics.completed(&key, Duration::from_millis(200), None, &small());
        metrics.request_started(&key);
        metrics.completed(&key, Duration::from_millis(100), None, &small());
        assert_eq!(
            metrics.average_completion_latency(),
            Some(Duration::from_millis(150))
        );
    }
}
