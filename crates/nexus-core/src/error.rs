use std::fmt;

/// Classifies every error Nexus can produce.
///
/// This is the stable category surface used for decision-making (logging,
/// retry policies, protocol error codes). Internal implementation details live
/// in `Error::internal` and must never be forwarded verbatim to clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    Network,
    Authentication,
    Authorization,
    Protocol,
    NotFound,
    InvalidRequest,
    AlreadyExists,
    Workspace,
    Storage,
    RateLimited,
    InvalidPath,
    Runtime,
    Build,
    Terminal,
    Snapshot,
    Plugin,
    Ai,
    Internal,
}

/// The unified Nexus error type.
///
/// Each error carries a stable [`ErrorCode`] plus a safe, user-facing message.
/// A separate optional `internal` field holds sensitive diagnostic detail that
/// stays server-side.
#[derive(Debug)]
pub struct Error {
    code: ErrorCode,
    message: String,
    internal: Option<String>,
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

/// Convenience alias for [`Result`](std::result::Result) with [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Creates a new error with the given code and user-facing message.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            internal: None,
            source: None,
        }
    }

    /// Creates an error and attaches sensitive internal detail.
    #[must_use]
    pub fn with_internal(mut self, detail: impl Into<String>) -> Self {
        self.internal = Some(detail.into());
        self
    }

    /// Creates an error and attaches an underlying source error.
    #[must_use]
    pub fn with_source<E: std::error::Error + Send + Sync + 'static>(mut self, source: E) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// The stable error category.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    /// The safe, user-facing error message.
    #[must_use]
    pub fn user_message(&self) -> &str {
        &self.message
    }

    /// Sensitive internal detail, if any. Log this only server-side.
    #[must_use]
    pub fn internal_detail(&self) -> Option<&str> {
        self.internal.as_deref()
    }

    /// Returns `true` when this error is likely retryable without changes.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(self.code, ErrorCode::Network | ErrorCode::Storage)
    }

    // --- Constructors ------------------------------------------------------

    #[must_use]
    pub fn network(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Network, msg)
    }

    #[must_use]
    pub fn authentication(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Authentication, msg)
    }

    #[must_use]
    pub fn authorization(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Authorization, msg)
    }

    #[must_use]
    pub fn protocol(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Protocol, msg)
    }

    #[must_use]
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, msg)
    }

    #[must_use]
    pub fn invalid_request(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidRequest, msg)
    }

    #[must_use]
    pub fn already_exists(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::AlreadyExists, msg)
    }

    #[must_use]
    pub fn workspace(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Workspace, msg)
    }

    #[must_use]
    pub fn storage(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Storage, msg)
    }

    #[must_use]
    pub fn rate_limited(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::RateLimited, msg)
    }

    #[must_use]
    pub fn invalid_path(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidPath, msg)
    }

    #[must_use]
    pub fn runtime(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Runtime, msg)
    }

    #[must_use]
    pub fn build(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Build, msg)
    }

    #[must_use]
    pub fn terminal(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Terminal, msg)
    }

    #[must_use]
    pub fn snapshot(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Snapshot, msg)
    }

    #[must_use]
    pub fn plugin(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Plugin, msg)
    }

    #[must_use]
    pub fn ai(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Ai, msg)
    }

    #[must_use]
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, msg)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (code={:?})", self.message, self.code)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|s| s.as_ref() as &(dyn std::error::Error + 'static))
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::network("I/O failure").with_source(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separates_internal_detail_from_user_message() {
        let err = Error::storage("StorageUnavailable")
            .with_internal("database connection failed: os error 13");
        assert_eq!(err.user_message(), "StorageUnavailable");
        assert_eq!(
            err.internal_detail(),
            Some("database connection failed: os error 13")
        );
        assert_eq!(err.code(), ErrorCode::Storage);
    }

    #[test]
    fn retryability_rules() {
        assert!(Error::network("x").is_retryable());
        assert!(Error::storage("x").is_retryable());
        assert!(!Error::authorization("x").is_retryable());
        assert!(!Error::protocol("x").is_retryable());
    }

    #[test]
    fn display_contains_message_and_code() {
        let text = Error::authorization("PermissionDenied").to_string();
        assert!(text.contains("PermissionDenied"));
        assert!(text.contains("Authorization"));
    }

    #[test]
    fn converts_io_errors() {
        let io = std::io::Error::new(std::io::ErrorKind::Other, "boom");
        let err: Error = io.into();
        assert_eq!(err.code(), ErrorCode::Network);
        assert!(std::error::Error::source(&err).is_some());
    }
}
