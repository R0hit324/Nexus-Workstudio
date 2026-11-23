//! Server-side plugin host glue (Part 11 Â§58, Â§104).
//!
//! Bridges the authoritative server runtime and the `nexus-plugin`
//! manager: built-in plugins are registered as trusted instances, external
//! manifests are discovered from `data_dir/plugins`, activation failures
//! are isolated (boot continues), and a summary is surfaced for
//! observability.

use std::path::PathBuf;
use std::sync::Arc;

use nexus_plugin::{
    lifecycle::PluginState, GitPlugin, InMemoryStorage, NativeInstanceProvider, PluginId,
    PluginManager, PluginManagerConfig, PluginResult, PluginTrust, RustPlugin,
};
use tracing::{info, warn};

/// Server side plugin host.
pub struct PluginHost {
    manager: Arc<PluginManager>,
    provider: Arc<NativeInstanceProvider>,
}

impl PluginHost {
    /// Builds the host: built-ins trusted, external manifests default
    /// restricted. Storage stays in memory until a per-user backing store
    /// is wired.
    #[must_use]
    pub fn new(search_paths: Vec<PathBuf>, config: Option<PluginManagerConfig>) -> Self {
        let mut manager_config = config.unwrap_or_default();
        manager_config.search_paths = search_paths;
        manager_config.trusted_ids = vec![
            PluginId::new("org.nexus.git").expect("valid builtin id"),
            PluginId::new("org.nexus.rust").expect("valid builtin id"),
        ];
        if manager_config.default_trust == PluginTrust::Untrusted {
            manager_config.default_trust = PluginTrust::Restricted;
        }
        let provider = Arc::new(NativeInstanceProvider::default());
        provider.register(Arc::new(GitPlugin::new()));
        provider.register(Arc::new(RustPlugin::new()));
        let manager = Arc::new(PluginManager::with_storage(
            manager_config,
            Arc::new(InMemoryStorage::new()),
        ));
        Self { manager, provider }
    }

    /// Discovers external manifests and activates everything resolvable in
    /// dependency order. Built-in manifests are injected when not present on
    /// disk. One bad plugin never wedges boot: activation failures are
    /// logged and recorded as `Failed` records.
    ///
    /// # Errors
    /// Surfaced only when discovery itself fails (unreadable roots);
    /// individual plugin failures are recorded rather than propagated.
    ///
    /// Returns the number of plugins enabled.
    pub async fn start(&self) -> PluginResult<usize> {
        self.manager.discover()?;
        self.inject_builtin_manifests();
        if let Err(failure) = self.manager.enable_all(self.provider.as_ref()).await {
            warn!(error = %failure.message, "plugin_activation_partial_failure");
        }
        let summary = self.summary();
        info!(%summary, "plugin_host_ready");
        Ok(summary.enabled)
    }

    /// Registers the built-in manifests unless the same ids arrived with
    /// on-disk manifests (installers may replace built-in tooling).
    fn inject_builtin_manifests(&self) {
        for manifest in builtin_manifests() {
            if self.manager.registry().record(&manifest.id).is_some() {
                continue;
            }
            let record = nexus_plugin::PluginRecord {
                state: PluginState::Discovered,
                granted: self.manager.grant_for(&manifest),
                trust: self.trust_for(manifest.id.as_str()),
                manifest,
                configuration: Default::default(),
                configuration_schema: None,
                last_error: None,
                scope: nexus_plugin::context::PluginScope::Server,
            };
            if let Err(failure) = self.manager.registry().upsert(record, true) {
                warn!(error = %failure.message, "builtin_manifest_register_failed");
            }
        }
    }

    fn trust_for(&self, id: &str) -> nexus_plugin::PluginTrust {
        match id {
            "org.nexus.git" | "org.nexus.rust" => PluginTrust::Trusted,
            _ => PluginTrust::Restricted,
        }
    }

    /// The underlying manager (commands/events/registry access).
    #[must_use]
    pub fn manager(&self) -> &Arc<PluginManager> {
        &self.manager
    }

    /// Registry snapshot for diagnostics.
    #[must_use]
    pub fn summary(&self) -> PluginStatusSummary {
        let mut enabled = 0;
        let mut failed = 0;
        let mut disabled = 0;
        let mut discovered = 0;
        for record in self.manager.registry().all() {
            match record.state {
                PluginState::Enabled => enabled += 1,
                PluginState::Failed => failed += 1,
                PluginState::Disabled => disabled += 1,
                _ => discovered += 1,
            }
        }
        PluginStatusSummary {
            enabled,
            failed,
            disabled,
            discovered,
        }
    }

    /// Graceful plugin shutdown during server stop.
    pub async fn shutdown(&self) {
        for id in self.manager.registry().ids_in_state(PluginState::Enabled) {
            if let Err(failure) = self.manager.disable(&id).await {
                warn!(plugin = %id, error = %failure.message, "plugin_disable_failed");
            }
        }
    }
}

/// Counts by state for the startup line and diagnostics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PluginStatusSummary {
    pub enabled: usize,
    pub failed: usize,
    pub disabled: usize,
    pub discovered: usize,
}

impl std::fmt::Display for PluginStatusSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "plugins: {} enabled, {} failed, {} disabled, {} staged",
            self.enabled, self.failed, self.disabled, self.discovered
        )
    }
}

/// The git built-in manifest (stable, API v1).
const BUILTIN_GIT_MANIFEST_TOML: &str = r#"
id = "org.nexus.git"
name = "Git Integration"
version = "1.0.0"
author = "Nexus"
description = "Built-in git VCS commands."
capabilities = ["vcs", "command"]
permissions = ["ui.register"]
"#;

/// The rust language built-in manifest.
const BUILTIN_RUST_MANIFEST_TOML: &str = r#"
id = "org.nexus.rust"
name = "Rust Support"
version = "1.0.0"
author = "Nexus"
description = "Built-in Rust language commands."
capabilities = ["language", "build", "command"]
permissions = ["ui.register"]
"#;

fn builtin_manifests() -> Vec<nexus_plugin::PluginManifest> {
    [BUILTIN_GIT_MANIFEST_TOML, BUILTIN_RUST_MANIFEST_TOML]
        .iter()
        .filter_map(
            |source| match nexus_plugin::PluginManifest::parse_toml(source) {
                Ok(manifest) => Some(manifest),
                Err(_) => None,
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use nexus_plugin::{GrantedPermissions, PluginContext, PluginPermission, PluginScope};

    fn caller_ctx() -> PluginContext {
        PluginContext::new(
            PluginId::new("org.nexus.host").unwrap(),
            PluginScope::Server,
            GrantedPermissions::from_set([PluginPermission::UiRegister]),
        )
    }

    fn unique_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nexus-server-pl-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[tokio::test]
    async fn builtins_activate_and_expose_commands() {
        let host = PluginHost::new(vec![unique_dir("empty")], None);
        let enabled = host.start().await.unwrap();
        assert_eq!(enabled, 2, "both built-ins activate");
        let host_ctx = caller_ctx();
        let out = host
            .manager()
            .commands()
            .execute(&host_ctx, "git.status", &[])
            .await
            .unwrap();
        assert!(out.text.contains("git:"), "output: {}", out.text);
        let out = host
            .manager()
            .commands()
            .execute(&host_ctx, "rust.check", &[])
            .await
            .unwrap();
        assert!(out.text.contains("cargo check"), "output: {}", out.text);
        host.shutdown().await;
    }

    #[tokio::test]
    async fn bad_manifests_never_wedge_boot() {
        let dir = unique_dir("bad");
        let plugin_dir = dir.join("broken");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("plugin.toml"), "not a manifest {{{{").unwrap();
        let host = PluginHost::new(vec![dir], None);
        let enabled = host.start().await.unwrap_or_default();
        assert_eq!(enabled, 2, "broken plugin is skipped, not fatal");
        host.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_disables_everything() {
        let host = PluginHost::new(vec![unique_dir("shutdown")], None);
        let _ = host.start().await;
        host.shutdown().await;
        // Bounded retry so a regression cannot hang the test runner.
        for _ in 0..10 {
            if host.summary().enabled == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(host.summary().enabled, 0);
    }
}
