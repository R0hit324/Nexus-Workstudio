//! Plugin-domain errors.
//!
//! Plugin failures are represented independently so a misbehaving plugin
//! cannot corrupt core error handling: every error carries the plugin it
//! belongs to (when known) and the operation that failed.

use std::fmt;

use crate::id::PluginId;

/// A plugin-scoped error.
#[derive(Debug, Clone)]
pub struct PluginError {
    /// The responsible plugin, when the error is attributable.
    pub plugin_id: Option<PluginId>,
    /// The failing operation, e.g. `"load"`, `"config.validate"`.
    pub operation: String,
    /// Human description.
    pub message: String,
}

/// Result alias across the plugin crate.
pub type PluginResult<T> = Result<T, PluginError>;

impl PluginError {
    /// Builds a plugin-scoped error.
    #[must_use]
    pub fn new(
        plugin_id: Option<PluginId>,
        operation: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            plugin_id,
            operation: operation.into(),
            message: message.into(),
        }
    }

    /// Platform-level error (no responsible plugin yet).
    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(None, "internal", message)
    }

    /// Attaches a plugin to an existing error.
    #[must_use]
    pub fn with_plugin(mut self, plugin_id: PluginId) -> Self {
        self.plugin_id = Some(plugin_id);
        self
    }
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.plugin_id {
            Some(id) => write!(f, "[{}] ({}): {}", id, self.operation, self.message),
            None => write!(f, "({}): {}", self.operation, self.message),
        }
    }
}

impl From<crate::id::InvalidPluginId> for PluginError {
    fn from(err: crate::id::InvalidPluginId) -> Self {
        Self::internal(err.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_error_display() {
        let id = PluginId::new("org.nexus.git").unwrap();
        let err = PluginError::new(Some(id), "load", "manifest missing field `permissions`");
        assert_eq!(
            err.to_string(),
            "[org.nexus.git] (load): manifest missing field `permissions`"
        );
    }

    #[test]
    fn unscoped_error_display() {
        let err = PluginError::internal("dependency cycle detected");
        assert_eq!(err.to_string(), "(internal): dependency cycle detected");
    }

    #[test]
    fn attach_plugin() {
        let err = PluginError::internal("boom")
            .with_plugin(PluginId::new("community.foo.theme").unwrap());
        assert_eq!(err.plugin_id.unwrap().as_str(), "community.foo.theme");
    }
}
