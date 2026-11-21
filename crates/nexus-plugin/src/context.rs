//! Scoped plugin context.
//!
//! The manager hands every hook a [`PluginContext`] bound to exactly one
//! plugin: it carries the stable identity, the activation scope, and the
//! least-privilege permission set granted by the host. Facades for storage,
//! commands, and events live on the attached [`PluginServices`] handle —
//! plugins never see internal managers or locks.

use std::path::PathBuf;
use std::sync::Arc;

use crate::id::PluginId;
use crate::permission::{GrantedPermissions, PluginPermission};

/// Where a plugin is activated.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PluginScope {
    /// Platform-global plugin (runs on the Nexus server).
    Server,
    /// Runs inside the client/TUI.
    Client,
    /// Scoped to a single cube; isolated from other cubes' state.
    Cube(nexus_core::CubeId),
}

/// Facades a host (server or client) attaches to a plugin.
#[derive(Clone)]
pub struct PluginServices {
    /// Namespaced, quota-controlled key/value storage.
    pub storage: Arc<dyn crate::storage::PluginStorageBackend>,
    /// Command registry facade.
    pub commands: Arc<dyn crate::commands::CommandRegistryApi>,
    /// Event subscription/publication facade.
    pub events: Arc<dyn crate::events::PluginEventApi>,
}

/// Per-plugin scoped context handed to every lifecycle hook.
#[derive(Clone)]
pub struct PluginContext {
    plugin_id: PluginId,
    scope: PluginScope,
    granted: GrantedPermissions,
    data_root: Option<PathBuf>,
    services: Option<PluginServices>,
}

impl PluginContext {
    /// Builds a context.
    #[must_use]
    pub fn new(plugin_id: PluginId, scope: PluginScope, granted: GrantedPermissions) -> Self {
        Self {
            plugin_id,
            scope,
            granted,
            data_root: None,
            services: None,
        }
    }

    /// Attaches the namespaced data directory used by the storage facade.
    #[must_use]
    pub fn with_data_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.data_root = Some(root.into());
        self
    }

    /// Attaches host facades.
    #[must_use]
    pub fn with_services(mut self, services: PluginServices) -> Self {
        self.services = Some(services);
        self
    }

    /// The owning plugin.
    #[must_use]
    pub fn plugin_id(&self) -> &PluginId {
        &self.plugin_id
    }

    /// The activation scope.
    #[must_use]
    pub fn scope(&self) -> &PluginScope {
        &self.scope
    }

    /// Granted permissions (least privilege).
    #[must_use]
    pub fn permissions(&self) -> &GrantedPermissions {
        &self.granted
    }

    /// Whether a permission was granted to this context.
    #[must_use]
    pub fn allows(&self, permission: PluginPermission) -> bool {
        self.granted.allows(permission)
    }

    /// Returns an error tagged with this plugin when the permission is
    /// missing; the tagged error keeps plugin failure attribution uniform.
    ///
    /// # Errors
    /// [`crate::PluginError`] with operation `permission.check`.
    pub fn require(&self, permission: PluginPermission) -> crate::error::PluginResult<()> {
        if self.allows(permission) {
            Ok(())
        } else {
            Err(crate::error::PluginError::new(
                Some(self.plugin_id.clone()),
                "permission.check",
                format!("plugin lacks required permission `{}`", permission.as_str()),
            ))
        }
    }

    /// Namespaced data root, set for server-scoped plugins.
    #[must_use]
    pub fn data_root(&self) -> Option<&PathBuf> {
        self.data_root.as_ref()
    }

    /// Host facades, when attached.
    #[must_use]
    pub fn services(&self) -> Option<&PluginServices> {
        self.services.as_ref()
    }
}

impl std::fmt::Debug for PluginContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately shallow: never log secrets or permission details.
        f.debug_struct("PluginContext")
            .field("plugin_id", &self.plugin_id)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}
