//! `nexus-plugin` implements the Part 11 plugin ecosystem.
//!
//! Layers:
//!
//! ```text
//! PluginId / manifest / capabilities / permissions        (contracts)
//! PluginManager / registry / dependencies / lifecycle    (control plane)
//! Context facades: commands, storage, events              (capability planes)
//! ```
//!
//! Core rules: plugins extend the platform through stable APIs only, every
//! sensitive capability needs explicit approval, dependency graphs resolve
//! before activation, and plugin failures are isolated and observable
//! without crashing Nexus.

pub mod builtin;
pub mod capability;
pub mod caps;
pub mod commands;
pub mod config;
pub mod context;
pub mod dependencies;
pub mod error;
pub mod events;
pub mod id;
pub mod lifecycle;
pub mod logs;
pub mod manager;
pub mod manifest;
pub mod metrics;
pub mod permission;
pub mod plugin;
pub mod registry;
pub mod storage;
pub mod ui;
pub mod version;

pub use builtin::{GitPlugin, RustPlugin};
pub use capability::PluginCapability;
pub use caps::{
    Breakpoint, BuildPlugin, BuildSystemDefinition, DebuggerDefinition, DebuggerPlugin,
    LanguageDefinition, LanguagePlugin, RuntimeBackendDefinition, RuntimePlugin, ThemeDefinition,
    ThemePlugin, THEME_KEYS,
};
pub use commands::{
    CommandArgument, CommandDescriptor, CommandHandler, CommandOutput, CommandRegistry,
    CommandRegistryApi, RESERVED_NAMESPACES,
};
pub use config::{ConfigField, ConfigKind, ConfigSchema, ConfigValue};
pub use context::{PluginContext, PluginScope, PluginServices};
pub use dependencies::DependencyOrder;
pub use error::{PluginError, PluginResult};
pub use events::{
    PluginEventApi, PluginEventCategory, PluginEventEnvelope, PluginEventHub, PluginScopeKey,
    PluginSubscription, PluginSubscriptionHandle, DEFAULT_EVENTS_PER_MINUTE,
    ENVELOPE_SCHEMA_VERSION, MAX_PLUGIN_EVENT_PAYLOAD,
};
pub use id::{InvalidPluginId, PluginId};
pub use lifecycle::PluginState;
pub use logs::{PluginLogBuffer, PluginLogEntry, PluginLogLevel, LOG_ENTRIES_PER_PLUGIN};
pub use manager::{
    current_platform, default_plugin_directory, NativeInstanceProvider, PluginInstanceProvider,
    PluginManager, PluginManagerConfig,
};
pub use manifest::{PluginDependency, PluginManifest, SUPPORTED_API_VERSION};
pub use metrics::{PluginMetrics, PluginMetricsSnapshot};
pub use permission::{GrantedPermissions, PluginPermission};
pub use plugin::Plugin;
pub use registry::{PluginRecord, PluginRegistry, PluginTrust};
pub use storage::{
    FilesystemStorage, InMemorySecretStore, InMemoryStorage, PluginSecretStore,
    PluginStorageBackend, StorageQuota,
};
pub use ui::{
    KeyBinding, PanelDefinition, UiRegistry, MAX_KEYBINDINGS_PER_PLUGIN, MAX_PANELS_PER_PLUGIN,
    RESERVED_PANEL_IDS,
};
pub use version::{InvalidVersion, PluginVersion, VersionReq};

/// Phase marker for version detection.
pub const PHASE: &str = "part11-plugin";

#[cfg(test)]
mod tests {
    #[test]
    fn phase_is_correct() {
        assert_eq!(super::PHASE, "part11-plugin");
    }

    #[test]
    fn api_version_is_platform_stable() {
        assert_eq!(super::SUPPORTED_API_VERSION, 1);
    }
}
