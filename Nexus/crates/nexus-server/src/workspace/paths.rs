use nexus_core::error::Error;

use crate::storage::is_safe_relative_path;

/// Maximum accepted length of a client-supplied workspace path.
pub const MAX_WORKSPACE_PATH_LENGTH: usize = 1024;

/// Validates a client-supplied workspace-relative path.
///
/// Workspace paths are always relative to the Cube's workspace root and are
/// normalized to forward slashes. Rejected: empty paths, absolute paths,
/// parent traversal (`..`), drive letters, control characters, and paths that
/// exceed [`MAX_WORKSPACE_PATH_LENGTH`].
#[must_use]
pub fn is_valid_workspace_path(path: &str) -> bool {
    if path.is_empty() || path.len() > MAX_WORKSPACE_PATH_LENGTH {
        return false;
    }
    is_safe_relative_path(path)
}

/// Like [`is_valid_workspace_path`] but returns a stable error for messaging.
pub fn validate_workspace_path(path: &str) -> nexus_core::Result<()> {
    if is_valid_workspace_path(path) {
        Ok(())
    } else {
        Err(Error::invalid_path(format!(
            "invalid path: {path:?} (must be relative to the workspace root)"
        )))
    }
}

/// Validates a *destination* path used by [`super::manager::CubeWorkspace::rename_file`],
/// additionally forbidding collapse onto an existing entry (checked by the caller).
#[must_use]
pub fn is_valid_destination_path(path: &str) -> bool {
    is_valid_workspace_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_relative_paths() {
        assert!(is_valid_workspace_path("readme.md"));
        assert!(is_valid_workspace_path("docs/guide.md"));
        assert!(is_valid_workspace_path("src/lib/core.rs"));
    }

    #[test]
    fn rejects_unsafe_paths() {
        assert!(!is_valid_workspace_path(""));
        assert!(!is_valid_workspace_path("/etc/passwd"));
        assert!(!is_valid_workspace_path("\\windows\\system32"));
        assert!(!is_valid_workspace_path("../secret"));
        assert!(!is_valid_workspace_path("C:/windows"));
        assert!(!is_valid_workspace_path("a/../b"));
        assert!(!is_valid_workspace_path("."));
        assert!(!is_valid_workspace_path(".."));
        assert!(!is_valid_workspace_path(
            &"x".repeat(MAX_WORKSPACE_PATH_LENGTH + 1)
        ));
    }

    #[test]
    fn validate_error_is_stable() {
        let err = validate_workspace_path("../escape").unwrap_err();
        assert_eq!(err.code(), nexus_core::error::ErrorCode::InvalidPath);
    }
}
