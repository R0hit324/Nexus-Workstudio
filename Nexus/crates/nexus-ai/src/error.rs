//! AI-domain errors.
//!
//! AI failures are independent of collaboration failures: every error carries
//! the operation that failed and a coarse kind that drives retry/fallback
//! decisions. Transient kinds may be retried (`§14`); auth, permission, and
//! invalid-request kinds must never be retried blindly.

use std::fmt;

/// Coarse failure class of an [`AiError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiErrorKind {
    /// Temporary network/provider hiccup; safe to retry.
    Transient,
    /// Provider refused with a rate limit; retry after a backoff.
    RateLimited,
    /// Provider timeout; safe to retry.
    Timeout,
    /// Request was malformed or violates constraints; never retry unchanged.
    InvalidRequest,
    /// Missing or rejected credentials; never retry.
    AuthDenied,
    /// The operation was refused by the permission system; never retry.
    PermissionDenied,
    /// A tool execution failed inside the opportunity loop; analyzable, not
    /// blindly retryable.
    ToolFailed,
    /// The caller cancelled the request; never retry.
    Cancelled,
    /// The provider (or a model within it) is not currently available.
    ProviderUnavailable,
    /// Catch-all internal failure.
    Internal,
}

/// An AI-scoped error.
#[derive(Debug, Clone)]
pub struct AiError {
    /// The failing operation, e.g. `"provider.stream"`, `"tool.read_file"`.
    pub operation: String,
    /// Human description.
    pub message: String,
    /// Coarse failure class driving retry/fallback.
    pub kind: AiErrorKind,
    /// Token usage consumed before failing (cost accounting on failure).
    pub usage: Option<AiUsage>,
}

/// Result alias across the AI crate.
pub type AiResult<T> = Result<T, AiError>;

/// Token accounting reported by a provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AiUsage {
    /// Tokens of the assembled prompt (instructions + context + history).
    pub prompt_tokens: u64,
    /// Tokens produced by the model.
    pub completion_tokens: u64,
}

impl AiUsage {
    /// Total token cost of an exchange.
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.prompt_tokens + self.completion_tokens
    }
}

impl AiError {
    /// Builds an AI-scoped error.
    #[must_use]
    pub fn new(
        kind: AiErrorKind,
        operation: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            operation: operation.into(),
            message: message.into(),
            kind,
            usage: None,
        }
    }

    /// Attaches usage consumed so far.
    #[must_use]
    pub fn with_usage(mut self, usage: AiUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    /// Whether retrying the identical request is sensible.
    #[must_use]
    pub const fn retries_permitted(&self) -> bool {
        matches!(
            self.kind,
            AiErrorKind::Transient
                | AiErrorKind::Timeout
                | AiErrorKind::RateLimited
                | AiErrorKind::ProviderUnavailable
        )
    }

    /// Whether a fallback provider should be tried after this error.
    #[must_use]
    pub const fn fallback_permitted(&self) -> bool {
        !matches!(
            self.kind,
            AiErrorKind::Cancelled | AiErrorKind::PermissionDenied
        )
    }

    /// Convenience constructor for [`AiErrorKind::Internal`].
    #[must_use]
    pub fn internal(operation: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(AiErrorKind::Internal, operation, message)
    }
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{:?}] {}: {}", self.kind, self.operation, self.message)
    }
}

impl std::error::Error for AiError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_policy_matches_the_vendors_we_know() {
        use AiErrorKind::*;
        for kind in [Transient, Timeout, RateLimited, ProviderUnavailable] {
            assert!(
                AiError::new(kind, "op", "msg").retries_permitted(),
                "{kind:?}"
            );
        }
        for kind in [
            InvalidRequest,
            AuthDenied,
            PermissionDenied,
            ToolFailed,
            Cancelled,
            Internal,
        ] {
            assert!(
                !AiError::new(kind, "op", "msg").retries_permitted(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn fallback_never_runs_after_cancellation_or_denial() {
        assert!(!AiError::new(AiErrorKind::Cancelled, "op", "x").fallback_permitted());
        assert!(!AiError::new(AiErrorKind::PermissionDenied, "op", "x").fallback_permitted());
        assert!(AiError::new(AiErrorKind::Transient, "op", "x").fallback_permitted());
        assert!(AiError::new(AiErrorKind::Internal, "op", "x").fallback_permitted());
    }

    #[test]
    fn usage_totals_and_failure_attachment() {
        let usage = AiUsage {
            prompt_tokens: 100,
            completion_tokens: 23,
        };
        assert_eq!(usage.total(), 123);
        let err = AiError::new(AiErrorKind::Timeout, "complete", "tick").with_usage(usage);
        assert_eq!(err.usage.unwrap().total(), 123);
        assert_eq!(err.to_string(), "[Timeout] complete: tick");
    }
}
