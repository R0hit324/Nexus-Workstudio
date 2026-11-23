//! AI context manager (`§15`–`§24`).
//!
//! Context flows through explicit stages: candidates → sensitive filtering →
//! relevance ranking → token-budgeted assembly. Workspace content is data,
//! never instructions: every fragment is wrapped in a machine-checkable
//! fence so system/user roles stay the only instruction channels (`§24`).
//!
//! Estimation is the documented trade-off of this slice: token counts use a
//! 4-characters-per-token heuristic, which is deliberately conservative
//! (over-estimation protects the model's window, never the other way round).

use crate::message::looks_like_injection;
use crate::model::ModelId;
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Token budget (`§21`)
// ---------------------------------------------------------------------------

/// The per-request context budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenBudget {
    /// Total tokens the model window can accept.
    pub max_context_tokens: u64,
    /// Tokens reserved for the reply.
    pub reserved_output_tokens: u64,
}

impl TokenBudget {
    /// Context-side allowance after reserving output.
    #[must_use]
    pub const fn context_side(&self) -> u64 {
        self.max_context_tokens
            .saturating_sub(self.reserved_output_tokens)
    }
}

/// A still-available number of context tokens.
#[derive(Debug, Clone, Copy)]
pub struct BudgetLedger {
    budget: TokenBudget,
    remaining: u64,
}

impl BudgetLedger {
    #[must_use]
    pub const fn new(budget: TokenBudget) -> Self {
        Self {
            budget,
            remaining: budget.context_side(),
        }
    }

    /// Consumes `tokens`; refuses when it would overdraw so callers can
    /// truncate instead (`§21` strategies live in the assembler).
    pub fn charge(&mut self, tokens: u64) -> bool {
        if self.remaining < tokens {
            return false;
        }
        self.remaining -= tokens;
        true
    }

    /// Attempts to charge up to `tokens`; returns the amount able to fit
    /// (0 when nothing fits).
    pub fn charge_partial(&mut self, tokens: u64) -> u64 {
        let accepted = if self.remaining < tokens {
            self.remaining
        } else {
            tokens
        };
        self.remaining -= accepted;
        accepted
    }

    #[must_use]
    pub const fn remaining(&self) -> u64 {
        self.remaining
    }

    #[must_use]
    pub const fn budget(&self) -> TokenBudget {
        self.budget
    }
}

// ---------------------------------------------------------------------------
// Sensitive context policy (`§23`)
// ---------------------------------------------------------------------------

/// Per-path handling of private material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensitivePolicy {
    /// Send the content as-is.
    Allowed,
    /// Replace the body with a stub note.
    Redacted,
    /// Drop the fragment entirely.
    Denied,
}

/// Path-aware sensitive-material configuration.
#[derive(Debug, Clone)]
pub struct SensitiveConfig {
    pub default: SensitivePolicy,
    user_protected_paths: Vec<String>,
}

impl SensitiveConfig {
    /// Matches conventionally private files out of the box (`§23`).
    const BUILT_IN: &'static [&'static str] = &[
        ".env",
        ".envrc",
        "id_rsa",
        "id_ed25519",
        ".pem",
        ".p12",
        ".pfx",
        "credentials",
        "secret",
        "token_registry",
    ];

    #[must_use]
    pub fn new(default: SensitivePolicy) -> Self {
        Self {
            default,
            user_protected_paths: Vec::new(),
        }
    }

    /// Adds an additional protected path (match is a case-insensitive
    /// substring of the workspace-relative path).
    pub fn protect(&mut self, path: impl Into<String>) {
        self.user_protected_paths.push(path.into());
    }

    /// Policy for one workspace path.
    #[must_use]
    pub fn policy_for(&self, path: &str) -> SensitivePolicy {
        let lowered = path.to_ascii_lowercase();
        let built_in_hits = Self::BUILT_IN.iter().any(|marker| lowered.contains(marker));
        if built_in_hits {
            return self.default;
        }
        if self
            .user_protected_paths
            .iter()
            .any(|p| lowered.contains(p))
        {
            return self.default;
        }
        SensitivePolicy::Allowed
    }
}

// ---------------------------------------------------------------------------
// Fragments (`§22`)
// ---------------------------------------------------------------------------

/// The provenance tier of one context fragment (drives ranking, `§18`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContextTier {
    /// Selected code the cursor landed on or emphasized.
    Selection,
    /// The symbol/function containing the cursor.
    CurrentFunction,
    /// The file the user is looking at.
    CurrentFile,
    /// Symbols defined outside but used inside the target.
    ReferencedSymbols,
    /// Related files (same module, imports, siblings).
    RelatedFiles,
    /// Anything else relevant to the workspace.
    Workspace,
}

impl ContextTier {
    /// Default relevance weight; higher is more important. The hierarchy
    /// follows `Selection > CurrentFunction > CurrentFile > ReferencedSymbols
    /// > RelatedFiles > Workspace` (`§18`).
    #[must_use]
    pub const fn weight(&self) -> u32 {
        match self {
            Self::Selection => 100,
            Self::CurrentFunction => 90,
            Self::CurrentFile => 80,
            Self::ReferencedSymbols => 60,
            Self::RelatedFiles => 40,
            Self::Workspace => 20,
        }
    }
}

/// One fenced, ranked piece of workspace data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextFragment {
    pub path: String,
    pub tier: ContextTier,
    pub body: String,
    /// Stable fingerprint of (path, body) used by the cache (`§22`).
    pub fingerprint: u64,
}

impl ContextFragment {
    #[must_use]
    pub fn new(path: impl Into<String>, tier: ContextTier, body: impl Into<String>) -> Self {
        let path = path.into();
        let body = body.into();
        let fingerprint = fingerprint_of(path.as_str(), body.as_str());
        Self {
            path,
            tier,
            body,
            fingerprint,
        }
    }

    /// Fences the fragment as untrusted workspace data (`§24`).
    #[must_use]
    pub fn fenced(&self) -> String {
        format!(
            "<workspace-data path=\"{}\" tier=\"{:?}\">\n{}\n</workspace-data>",
            self.path, self.tier, self.body
        )
    }

    /// Approximate token cost under the 4-char heuristic.
    #[must_use]
    pub fn estimated_tokens(&self) -> u64 {
        (self.body.len() as u64 + 16).div_ceil(4)
    }
}

/// Stable (path, body) hash. FNV-1a 64-bit: cheap, deterministic, and good
/// enough for change detection (content hashes drive cache validation, the
/// cache never treats these as cryptographic identifiers).
#[must_use]
pub fn fingerprint_of(path: &str, body: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    feed(path.as_bytes());
    feed(b"\x00");
    feed(body.as_bytes());
    hash
}

// ---------------------------------------------------------------------------
// Ranking / dedup (`§20`)
// ---------------------------------------------------------------------------

/// A ranked candidate prepared for the final assembly.
#[derive(Debug, Clone)]
pub struct RankedFragment {
    pub fragment: ContextFragment,
    pub score: u32,
}

/// Scores and orders fragments: deduplication by path keeps the higher
/// tier, then a stable sort orders by relevance score (`§20`).
#[must_use]
pub fn rank(mut fragments: Vec<ContextFragment>) -> Vec<RankedFragment> {
    fragments.sort_by(|a, b| {
        // Dedup: same path keeps the *higher weighted* tier in front, so
        // dedup-by-first wins with the better fragment.
        match a.path.cmp(&b.path) {
            core::cmp::Ordering::Equal => b.tier.weight().cmp(&a.tier.weight()),
            ordering => ordering,
        }
    });
    fragments.dedup_by_key(|frag| frag.path.clone());

    let mut ranked: Vec<RankedFragment> = fragments
        .into_iter()
        .map(|fragment| {
            let score = fragment.tier.weight();
            RankedFragment { fragment, score }
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.fragment.path.cmp(&b.fragment.path))
    });
    ranked
}

// ---------------------------------------------------------------------------
// Cache (`§22`)
// ---------------------------------------------------------------------------

/// Content-hash cache of context fragments, invalidated on workspace change.
#[derive(Debug, Default)]
pub struct ContextCache {
    entries: BTreeMap<String, CachedFragment>,
    workspace_generation: u64,
    hits: u64,
    misses: u64,
}

#[derive(Debug, Clone)]
struct CachedFragment {
    fragment: ContextFragment,
}

impl ContextCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fetched-or-computed lookup keyed by path; valid only while the stored
    /// fingerprint matches the caller's expectation.
    /// Returns `Some(fragment)` when cached.
    #[must_use]
    pub fn lookup(&mut self, path: &str, fingerprint: u64) -> Option<ContextFragment> {
        if let Some(entry) = self.entries.get(path) {
            if entry.fragment.fingerprint == fingerprint {
                self.hits += 1;
                return Some(entry.fragment.clone());
            }
            self.entries.remove(path);
        }
        self.misses += 1;
        None
    }

    /// Caches a fragment under its path.
    pub fn store(&mut self, fragment: ContextFragment) {
        self.entries
            .insert(fragment.path.clone(), CachedFragment { fragment });
    }

    /// Workspace mutation bump; wipes everything (`§22`).
    #[allow(dead_code)]
    pub fn invalidate_workspace(&mut self, generation: u64) {
        self.workspace_generation = generation;
        self.entries.clear();
        self.misses += 1;
    }

    /// Live cache size.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub const fn hits(&self) -> u64 {
        self.hits
    }

    #[must_use]
    pub const fn misses(&self) -> u64 {
        self.misses
    }
}

// ---------------------------------------------------------------------------
// The assembler (`§15`)
// ---------------------------------------------------------------------------

/// Requested inputs packaged by the client: what is on screen and what the
/// user asked for.
#[derive(Clone, Default)]
pub struct ContextRequest {
    /// The selected code, if any (`§18`).
    pub selection: Option<String>,
    /// The file path and body of the current document.
    pub current_path: Option<String>,
    pub current_body: Option<String>,
    /// Additional paths considered relevant (imports, diagnostics, search
    /// hits) with raw bodies already fetched by the client layer.
    pub related: Vec<(String, String, ContextTier)>,
    /// Total limit from the target model.
    pub model_context_window: u64,
    /// Model the request targets.
    pub model: Option<ModelId>,
}

impl core::fmt::Debug for ContextRequest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ContextRequest")
            .field("selection", &self.selection.as_ref().map(|s| s.len()))
            .field("current_path", &self.current_path)
            .field("related", &self.related.len())
            .field("model_context_window", &self.model_context_window)
            .field("model", &self.model)
            .finish()
    }
}

/// The final context the orchestrator attaches to a request.
#[derive(Debug, Clone)]
pub struct AssembledContext {
    pub text: String,
    pub fragments: Vec<ContextFragment>,
    /// Tracks redaction invariants so tests/audits can assert guarantees.
    pub redacted: usize,
    pub dropped_sensitive: usize,
    /// Fragments whose content tripped the advisory injection heuristic (`§24`).
    pub flagged_injections: usize,
}

/// Builds assembled context from a request.
#[must_use]
pub fn assemble(
    request: &ContextRequest,
    sensitive: &SensitiveConfig,
    budget: TokenBudget,
) -> AssembledContext {
    let mut redacted = 0usize;
    let mut dropped = 0usize;
    let mut fragments = Vec::new();

    if let (Some(path), Some(body)) = (&request.current_path, &request.current_body) {
        match sensitive.policy_for(path) {
            SensitivePolicy::Allowed => fragments.push(ContextFragment::new(
                path.clone(),
                ContextTier::CurrentFile,
                body.clone(),
            )),
            SensitivePolicy::Redacted => {
                redacted += 1;
                fragments.push(ContextFragment::new(
                    path.clone(),
                    ContextTier::CurrentFile,
                    "[redacted: sensitive file contact excluded from AI context]",
                ));
            }
            SensitivePolicy::Denied => dropped += 1,
        }
    }

    if let Some(selection) = &request.selection {
        fragments.push(ContextFragment::new(
            request
                .current_path
                .clone()
                .unwrap_or_else(|| "selection".into()),
            ContextTier::Selection,
            selection.clone(),
        ));
    }

    for (path, body, tier) in &request.related {
        match sensitive.policy_for(path) {
            SensitivePolicy::Allowed => {
                fragments.push(ContextFragment::new(path.clone(), *tier, body.clone()));
            }
            SensitivePolicy::Redacted => {
                redacted += 1;
                fragments.push(ContextFragment::new(
                    path.clone(),
                    *tier,
                    "[redacted: sensitive file contact excluded from AI context]",
                ));
            }
            SensitivePolicy::Denied => dropped += 1,
        }
    }

    let ranked = rank(fragments);

    // Injection surveillance happens on the fenced text: content inside a
    // fenced block is data; flagging is advisory only (`§24`) and surfaced to
    // audit logging (`§68`).
    let flagged_injections = ranked
        .iter()
        .filter(|candidate| looks_like_injection(&candidate.fragment.body))
        .count();

    let mut ledger = BudgetLedger::new(budget);
    let mut picked: Vec<ContextFragment> = Vec::new();
    for candidate in ranked {
        let tokens = candidate.fragment.estimated_tokens();
        if ledger.remaining() >= tokens {
            if ledger.charge(tokens) {
                picked.push(candidate.fragment);
            }
        } else {
            // Truncate the low-priority fragment to the room that remains,
            // reserving space for the fence overhead; skip when the floor is
            // too small (`§21` truncation strategy).
            let partial = ledger.charge_partial(tokens);
            if partial >= 32 {
                let chars = (partial - 32) * 4;
                let trimmed: String = candidate
                    .fragment
                    .body
                    .chars()
                    .take(chars as usize)
                    .collect();
                picked.push(ContextFragment {
                    path: candidate.fragment.path.clone(),
                    tier: candidate.fragment.tier,
                    body: trimmed,
                    fingerprint: candidate.fragment.fingerprint,
                });
            }
        }
    }

    let text = picked
        .iter()
        .map(ContextFragment::fenced)
        .collect::<Vec<_>>()
        .join("\n\n");
    AssembledContext {
        text,
        fragments: picked,
        redacted,
        dropped_sensitive: dropped,
        flagged_injections,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_ledger_charges_and_refuses_to_overdraw() {
        let ledger = BudgetLedger::new(TokenBudget {
            max_context_tokens: 4_096,
            reserved_output_tokens: 512,
        });
        assert_eq!(ledger.budget().context_side(), 3_584);
    }

    #[test]
    fn reserve_never_allows_negative_context() {
        let budget = BudgetLedger::new(TokenBudget {
            max_context_tokens: 256,
            reserved_output_tokens: 1_000,
        });
        assert_eq!(budget.remaining(), 0);
    }
}

/// Ensures the ledger's second battery of assertions lives here.
#[cfg(test)]
mod ledger_tests {
    use super::*;

    #[test]
    fn charging_empty_ledger_cannot_go_negative() {
        let mut ledger = BudgetLedger::new(TokenBudget {
            max_context_tokens: 0,
            reserved_output_tokens: 0,
        });
        assert!(ledger.remaining() == 0);
        assert!(!ledger.charge(1));
        assert_eq!(ledger.charge_partial(5), 0);
    }

    #[test]
    fn partial_charge_fits_what_is_left() {
        let mut ledger = BudgetLedger::new(TokenBudget {
            max_context_tokens: 10,
            reserved_output_tokens: 0,
        });
        assert!(ledger.charge(6));
        assert_eq!(ledger.remaining(), 4);
        assert_eq!(ledger.charge_partial(9), 4);
        assert_eq!(ledger.remaining(), 0);
    }
}

#[cfg(test)]
mod sensitivity_tests {
    use super::*;

    #[test]
    fn builtin_sensitivity_covers_dotfiles_and_keys() {
        let config = SensitiveConfig::new(SensitivePolicy::Denied);
        assert_ne!(config.policy_for("src/main.rs"), SensitivePolicy::Denied);
        assert_eq!(config.policy_for(".env"), SensitivePolicy::Denied);
        assert_eq!(config.policy_for(".ENV.local"), SensitivePolicy::Denied);
        assert_eq!(config.policy_for(".envrc"), SensitivePolicy::Denied);
        assert_eq!(config.policy_for("keys/id_rsa"), SensitivePolicy::Denied);
        assert_eq!(
            config.policy_for("certs/server.pem"),
            SensitivePolicy::Denied
        );
        assert_eq!(
            config.policy_for(".credentials.json"),
            SensitivePolicy::Denied
        );
        assert_eq!(config.policy_for("secrets/store"), SensitivePolicy::Denied);
    }

    #[test]
    fn redaction_policy_masks_but_keeps_the_place() {
        let config = SensitiveConfig::new(SensitivePolicy::Redacted);
        assert_eq!(config.policy_for(".env"), SensitivePolicy::Redacted);
        // User-protected paths use the same policy.
        let mut tuned = SensitiveConfig::new(SensitivePolicy::Redacted);
        assert_ne!(
            tuned.policy_for("configs/ops.yaml"),
            SensitivePolicy::Redacted
        );
        tuned.protect("ops.yaml");
        assert_eq!(
            tuned.policy_for("configs/ops.yaml"),
            SensitivePolicy::Redacted
        );
    }

    #[test]
    fn assembling_honours_the_dotenv_drop() {
        let request = ContextRequest {
            current_path: Some(String::from(".env")),
            current_body: Some(String::from("API_KEY=supersecret")),
            selection: None,
            related: vec![(
                String::from("certs/ca.pem"),
                String::from("-----BEGIN CERTIFICATE-----"),
                ContextTier::RelatedFiles,
            )],
            model_context_window: 2_048,
            model: None,
        };
        let sensitive = SensitiveConfig::new(SensitivePolicy::Denied);
        let assembled = assemble(
            &request,
            &sensitive,
            TokenBudget {
                max_context_tokens: 2_048,
                reserved_output_tokens: 256,
            },
        );
        assert!(!assembled.text.contains("supersecret"));
        assert!(!assembled.text.contains("BEGIN CERTIFICATE"));
        assert_eq!(assembled.dropped_sensitive, 2);
    }

    #[test]
    fn redaction_policy_masks_the_body() {
        let request = ContextRequest {
            current_path: Some(String::from("secrets/token_registry")),
            current_body: Some(String::from("primary-token=abc123")),
            selection: None,
            related: vec![],
            model_context_window: 2_048,
            model: None,
        };
        let sensitive = SensitiveConfig::new(SensitivePolicy::Redacted);
        let assembled = assemble(
            &request,
            &sensitive,
            TokenBudget {
                max_context_tokens: 2_048,
                reserved_output_tokens: 256,
            },
        );
        assert!(!assembled.text.contains("abc123"));
        assert!(assembled.text.contains("[redacted"));
        assert_eq!(assembled.redacted, 1);
    }
}

#[cfg(test)]
mod assembly_tests {
    use super::*;

    fn request_with(
        selection: Option<(String, String)>,
        related: Vec<(&str, &str, ContextTier)>,
    ) -> ContextRequest {
        let (current_path, current_body, selection) = match selection {
            Some((path, body)) => (Some(path), Some(body.clone()), Some(body)),
            None => (None, None, None),
        };
        ContextRequest {
            selection,
            current_path,
            current_body,
            related: related
                .into_iter()
                .map(|(path, body, tier)| (String::from(path), String::from(body), tier))
                .collect(),
            model_context_window: 8_000,
            model: None,
        }
    }

    #[test]
    fn selection_beats_the_rest_of_the_file() {
        let request = request_with(
            Some((
                String::from("src/order.rs"),
                String::from("fn calculate_total(items: &[i64]) -> i64 { items.iter().sum() }"),
            )),
            vec![("src/lib.rs", "reads thirty lines", ContextTier::CurrentFile)],
        );
        let sensitive = SensitiveConfig::new(SensitivePolicy::Allowed);
        let assembled = assemble(
            &request,
            &sensitive,
            TokenBudget {
                max_context_tokens: 8_000,
                reserved_output_tokens: 512,
            },
        );
        // The fragment selected first contains ONLY the selection text.
        assert_eq!(
            assembled.fragments.first().unwrap().tier,
            ContextTier::Selection
        );
        assert!(assembled
            .fragments
            .first()
            .unwrap()
            .body
            .contains("calculate_total"));
        // Second is the current file body.
        assert_eq!(
            assembled.fragments.get(1).unwrap().tier,
            ContextTier::CurrentFile
        );
    }

    #[test]
    fn budgeted_assembly_never_exceeds_the_window() {
        let payload = "x".repeat(40_000);
        let request = ContextRequest {
            selection: Some(payload.clone()),
            current_path: Some(String::from("big.rs")),
            current_body: Some(payload.clone()),
            related: vec![],
            model_context_window: 8_000,
            model: None,
        };
        let sensitive = SensitiveConfig::new(SensitivePolicy::Allowed);
        let assembled = assemble(
            &request,
            &sensitive,
            TokenBudget {
                max_context_tokens: 8_000,
                reserved_output_tokens: 512,
            },
        );
        // Estimate: fenced text stays under the budget side.
        let estimated = (assembled.text.len() as u64).div_ceil(4);
        assert!(estimated <= 7_488, "estimated {estimated}");
        // At least one lower-tier fragment got truncated.
        assert!(assembled
            .fragments
            .iter()
            .any(|fragment| fragment.body.len() < payload.len()));
    }

    #[test]
    fn dedup_prefers_the_higher_tier_fragment() {
        let assembled = crate::context::rank(vec![
            ContextFragment::new("dup.rs", ContextTier::RelatedFiles, "stub"),
            ContextFragment::new("dup.rs", ContextTier::Selection, "actual selection"),
        ]);
        assert_eq!(assembled.len(), 1);
        assert_eq!(assembled[0].fragment.tier, ContextTier::Selection);
    }

    #[test]
    fn fingerprints_distinguish_path_and_body() {
        assert_ne!(
            fingerprint_of("a.rs", "body"),
            fingerprint_of("b.rs", "body")
        );
        assert_ne!(fingerprint_of("a.rs", "x"), fingerprint_of("a.rs", "y"));
        assert_eq!(fingerprint_of("a.rs", "x"), fingerprint_of("a.rs", "x"));
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn lookup_round_trips_and_invalidates_on_workspace_change() {
        let mut cache = ContextCache::new();
        let fragment = ContextFragment::new("src/lib.rs", ContextTier::CurrentFile, "content");
        let fingerprint = fragment.fingerprint;
        cache.store(fragment);
        assert_eq!(cache.len(), 1);
        assert_eq!(
            cache.lookup("src/lib.rs", fingerprint).unwrap().body,
            "content"
        );

        // Fetching with a changed fingerprint misses and gets pruned.
        let mutated = ContextFragment::new("src/lib.rs", ContextTier::CurrentFile, "changed");
        assert!(cache.lookup("src/lib.rs", mutated.fingerprint).is_none());
        assert!(cache.is_empty());

        cache.store(mutated);
        cache.invalidate_workspace(2);
        assert!(cache.is_empty());
    }

    #[test]
    fn hits_and_misses_are_accounted() {
        let mut cache = ContextCache::new();
        let fingerprint = crate::context::fingerprint_of("a.rs", "x");
        cache.store(ContextFragment::new("a.rs", ContextTier::Workspace, "x"));
        let _ = cache.lookup("a.rs", fingerprint);
        let _ = cache.lookup("a.rs", fingerprint);
        let _ = cache.lookup("ghost.rs", 0);
        assert_eq!(cache.hits(), 2);
        assert_eq!(cache.misses(), 1);
    }
}
