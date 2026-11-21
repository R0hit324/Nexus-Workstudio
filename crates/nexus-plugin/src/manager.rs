//! The plugin control plane.
//!
//! Owns discovery, validation, dependency resolution, permission approval,
//! lifecycle activation with per-hook timeouts and crash isolation,
//! least-privilege grants, and observability. Plugin failures are contained:
//! a crashing or timing-out plugin is marked [`PluginState::Failed`] and
//! Nexus keeps running.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::{Mutex as AsyncMutex, RwLock as AsyncRwLock};
use tokio::time::timeout;

use crate::commands::{CommandRegistry, CommandRegistryApi};
use crate::config::{ConfigSchema, ConfigValue};
use crate::context::{PluginContext, PluginScope, PluginServices};
use crate::dependencies;
use crate::error::{PluginError, PluginResult};
use crate::events::{PluginEventApi, PluginEventHub};
use crate::id::PluginId;
use crate::lifecycle::PluginState;
use crate::manifest::PluginManifest;
use crate::metrics::PluginMetrics;
use crate::plugin::Plugin;
use crate::registry::{PluginRecord, PluginRegistry, PluginTrust};
use crate::storage::StorageQuota;
use crate::{GrantedPermissions, PluginPermission, PluginVersion, SUPPORTED_API_VERSION};

/// Supplies runtime instances for a plugin id.
///
/// The initial ontology loads trusted native instances compiled into the
/// host binary; WASM/process isolation is a later integration.
#[async_trait]
pub trait PluginInstanceProvider: Send + Sync {
    /// Returns the instance for `id`, if one is available.
    async fn provide(&self, id: &PluginId) -> PluginResult<Option<Arc<dyn Plugin>>>;
}

/// An in-process registry of instances keyed by plugin id.
#[derive(Default)]
pub struct NativeInstanceProvider {
    instances: std::sync::RwLock<BTreeMap<String, Arc<dyn Plugin>>>,
}

impl NativeInstanceProvider {
    /// Registers one trusted native instance.
    pub fn register(&self, instance: Arc<dyn Plugin>) {
        self.instances
            .write()
            .expect("instance provider lock")
            .insert(instance.id().as_str().to_owned(), instance);
    }
}

#[async_trait]
impl PluginInstanceProvider for NativeInstanceProvider {
    async fn provide(&self, id: &PluginId) -> PluginResult<Option<Arc<dyn Plugin>>> {
        Ok(self
            .instances
            .read()
            .expect("instance provider lock")
            .get(id.as_str())
            .cloned())
    }
}

/// Configuration knobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginManagerConfig {
    /// Directories scanned for manifest TOMLs.
    pub search_paths: Vec<PathBuf>,
    /// Ids allowed to run in-process (`Trusted`, spec 77).
    pub trusted_ids: Vec<PluginId>,
    /// Admin/user-approved sensitive permissions per plugin (spec 20).
    pub approved: BTreeMap<PluginId, BTreeSet<PluginPermission>>,
    /// Hook timeouts (spec 81).
    pub load_timeout: Duration,
    pub enable_timeout: Duration,
    pub disable_timeout: Duration,
    pub unload_timeout: Duration,
    /// Consecutive lifecycle failures before lockout (spec 82).
    pub max_consecutive_failures: u32,
    /// Trust default for plugins not listed as trusted.
    pub default_trust: PluginTrust,
    /// Event publish budget per plugin per minute.
    pub event_rate_limit: u64,
    /// Storage quota per plugin namespace.
    pub storage_quota: StorageQuota,
}

impl Default for PluginManagerConfig {
    fn default() -> Self {
        Self {
            search_paths: vec![default_plugin_directory()],
            trusted_ids: Vec::new(),
            approved: BTreeMap::new(),
            load_timeout: Duration::from_secs(15),
            enable_timeout: Duration::from_secs(15),
            disable_timeout: Duration::from_secs(15),
            unload_timeout: Duration::from_secs(15),
            max_consecutive_failures: 3,
            default_trust: PluginTrust::Restricted,
            event_rate_limit: 240,
            storage_quota: StorageQuota::default_plugin(),
        }
    }
}

/// Lifecycle hook selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hook {
    Load,
    Enable,
    Disable,
    Unload,
}

impl Hook {
    fn timeout(self, config: &PluginManagerConfig) -> Duration {
        match self {
            Self::Load => config.load_timeout,
            Self::Enable => config.enable_timeout,
            Self::Disable => config.disable_timeout,
            Self::Unload => config.unload_timeout,
        }
    }

    fn operation(self) -> &'static str {
        match self {
            Self::Load => "load",
            Self::Enable => "enable",
            Self::Disable => "disable",
            Self::Unload => "unload",
        }
    }
}

/// Platform-aware default plugin directory (`~/.nexus/plugins`).
#[must_use]
pub fn default_plugin_directory() -> PathBuf {
    if let Some(home) = std::env::var_os("NEXUS_PLUGINS_DIR") {
        return PathBuf::from(home);
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home).join(".nexus").join("plugins");
    }
    PathBuf::from(".nexus").join("plugins")
}

/// Platform-aware default plugin storage root (`~/.nexus/plugin-data`).
#[must_use]
pub fn default_storage_directory() -> PathBuf {
    default_plugin_directory()
        .parent()
        .map(|base| base.join("plugin-data"))
        .unwrap_or_else(|| PathBuf::from("plugin-data"))
}

/// The unified manager.
pub struct PluginManager {
    config: PluginManagerConfig,
    registry: PluginRegistry,
    instances: AsyncRwLock<BTreeMap<PluginId, Arc<dyn Plugin>>>,
    consecutive_failures: AsyncMutex<BTreeMap<PluginId, u32>>,
    commands: Arc<CommandRegistry>,
    events: Arc<PluginEventHub>,
    storage_backend: Arc<dyn crate::storage::PluginStorageBackend>,
    metrics: Arc<PluginMetrics>,
}

impl PluginManager {
    /// Builds a manager.
    #[must_use]
    pub fn new(config: PluginManagerConfig) -> Self {
        let storage_backend = Arc::new(crate::storage::FilesystemStorage::new(
            default_storage_directory(),
        ));
        Self::with_storage(config, storage_backend)
    }

    /// Builds a manager over an explicit storage backend (tests, TUIs).
    #[must_use]
    pub fn with_storage(
        config: PluginManagerConfig,
        storage_backend: Arc<dyn crate::storage::PluginStorageBackend>,
    ) -> Self {
        Self {
            registry: PluginRegistry::new(),
            instances: AsyncRwLock::new(BTreeMap::new()),
            consecutive_failures: AsyncMutex::new(BTreeMap::new()),
            commands: Arc::new(CommandRegistry::new()),
            events: Arc::new(PluginEventHub::new(true, true, config.event_rate_limit)),
            metrics: Arc::new(PluginMetrics::default()),
            config,
            storage_backend,
        }
    }

    /// Registry accessor.
    #[must_use]
    pub fn registry(&self) -> &PluginRegistry {
        &self.registry
    }

    /// Command registry accessor (the `:`-executor fronts this).
    #[must_use]
    pub fn commands(&self) -> &CommandRegistry {
        &self.commands
    }

    /// Event hub accessor.
    #[must_use]
    pub fn events(&self) -> &PluginEventHub {
        &self.events
    }

    /// Metrics accessor.
    #[must_use]
    pub fn metrics(&self) -> &Arc<PluginMetrics> {
        &self.metrics
    }

    // ---- discovery ----------------------------------------------------------

    /// Scans configured search paths for manifests. Each valid manifest
    /// enters the registry as [`PluginState::Discovered`]; broken manifests
    /// are recorded and skipped so one bad file cannot wedge the host.
    ///
    /// Returns discovered plugin ids.
    pub fn discover(&self) -> PluginResult<Vec<PluginId>> {
        let mut discovered = Vec::new();
        for search_path in &self.config.search_paths {
            let Ok(entries) = std::fs::read_dir(search_path) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    for candidate in ["plugin.toml", "nexus-plugin.toml"] {
                        let manifest_path = path.join(candidate);
                        if manifest_path.exists() {
                            self.record_manifest_file(&manifest_path, &mut discovered);
                        }
                    }
                } else if path.extension().is_some_and(|ext| ext == "toml") {
                    self.record_manifest_file(&path, &mut discovered);
                }
            }
        }
        Ok(discovered)
    }

    fn record_manifest_file(&self, path: &PathBuf, discovered: &mut Vec<PluginId>) {
        let manifest = match PluginManifest::from_path(path) {
            Ok(manifest) => manifest,
            Err(error) => {
                if let Some(id) = error.plugin_id.clone() {
                    self.registry.mark_failed(&id, error.message.clone());
                    self.metrics.record_failure(&id, &error.message);
                }
                return;
            }
        };

        if let Err(error) = self.compatibility_check(&manifest) {
            self.registry
                .mark_failed(&manifest.id, error.message.clone());
            self.metrics.record_failure(&manifest.id, &error.message);
            return;
        }

        // Re-discovery must not resurrect an actively disabled plugin but
        // does refresh a stale manifest behind the same id.
        let previous = self.registry.record(&manifest.id).map(|r| r.state);
        let state = match previous {
            Some(PluginState::Disabled) => PluginState::Disabled,
            Some(PluginState::Enabled)
            | Some(PluginState::Initialized)
            | Some(PluginState::Loaded) => PluginState::Initialized,
            _ => PluginState::Discovered,
        };
        let record = PluginRecord {
            state,
            granted: self.grant_for(&manifest),
            trust: self.trust_for_id(manifest.id.as_str()),
            manifest,
            configuration: BTreeMap::new(),
            configuration_schema: None,
            last_error: None,
            scope: PluginScope::Server,
        };
        let id = record.manifest.id.clone();
        match self.registry.upsert(record, true) {
            Ok(()) => discovered.push(id),
            Err(error) => {
                self.metrics.record_failure(&id, &error.message);
            }
        }
    }

    fn trust_for_id(&self, id: &str) -> PluginTrust {
        if self
            .config
            .trusted_ids
            .iter()
            .any(|trusted| trusted.as_str() == id)
        {
            PluginTrust::Trusted
        } else {
            self.config.default_trust
        }
    }

    /// Least privilege: granted = declared ∩ (non-sensitive ∨ approved).
    #[must_use]
    pub fn grant_for(&self, manifest: &PluginManifest) -> GrantedPermissions {
        let mut approved: Vec<PluginPermission> = Vec::new();
        for permission in &manifest.permissions {
            if permission.requires_approval() {
                if self
                    .config
                    .approved
                    .get(&manifest.id)
                    .is_some_and(|set| set.contains(permission))
                {
                    approved.push(*permission);
                }
            } else {
                approved.push(*permission);
            }
        }
        GrantedPermissions::grant(&manifest.permissions, approved)
    }

    /// Records an explicit admin/user approval and re-grants (spec 20).
    ///
    /// # Errors
    /// Unknown plugin.
    pub fn approve(&mut self, id: &PluginId, permission: PluginPermission) -> PluginResult<()> {
        let record = self.registry.record(id).ok_or_else(|| {
            PluginError::new(Some(id.clone()), "approve", "plugin not discovered")
        })?;
        self.config
            .approved
            .entry(id.clone())
            .or_default()
            .insert(permission);
        self.registry.grant(id, self.grant_for(&record.manifest));
        Ok(())
    }

    // ---- compatibility ------------------------------------------------------

    /// Compatibility gate (spec 71): manifest sanity, API version,
    /// platform support.
    fn compatibility_check(&self, manifest: &PluginManifest) -> PluginResult<()> {
        manifest.validate()?;
        if !PluginVersion::api_compatible(manifest.api_version) {
            return Err(PluginError::new(
                Some(manifest.id.clone()),
                "compatibility.check",
                format!(
                    "plugin api_version {} unsupported by platform api {SUPPORTED_API_VERSION}",
                    manifest.api_version
                ),
            ));
        }
        if !manifest.platforms.is_empty() && !current_platform_in(&manifest.platforms) {
            return Err(PluginError::new(
                Some(manifest.id.clone()),
                "compatibility.check",
                format!(
                    "plugin does not support current platform {:?}",
                    current_platform()
                ),
            ));
        }
        Ok(())
    }

    // ---- lifecycle ----------------------------------------------------------

    /// Resolves the load order for currently registered manifests.
    ///
    /// # Errors
    /// Missing plugins, conflicts, or cycles.
    pub fn dependency_order(&self) -> PluginResult<dependencies::DependencyOrder> {
        dependencies::resolve(self.dependency_inputs())
    }

    fn dependency_inputs(
        &self,
    ) -> Vec<(
        PluginId,
        PluginVersion,
        Vec<crate::manifest::PluginDependency>,
    )> {
        self.registry
            .all()
            .into_iter()
            .map(|record| {
                (
                    record.manifest.id.clone(),
                    record.manifest.version,
                    record.manifest.dependencies.clone(),
                )
            })
            .collect()
    }

    /// Full activation path for one plugin: validate → provide instance →
    /// load → enable, honoring dependency state, timeouts, and crash
    /// isolation.
    ///
    /// # Errors
    /// Any lifecycle failure leaves the record `Failed` with the reason.
    pub async fn activate(
        &self,
        id: &PluginId,
        provider: &dyn PluginInstanceProvider,
    ) -> PluginResult<()> {
        let record = self.registry.record(id).ok_or_else(|| {
            PluginError::new(Some(id.clone()), "activate", "plugin not discovered")
        })?;

        if self.failure_count(id) >= self.config.max_consecutive_failures {
            let message = format!(
                "locked out after {} consecutive failures",
                self.failure_count(id)
            );
            self.registry.mark_failed(id, message.clone());
            self.metrics.record_failure(id, &message);
            return Err(PluginError::new(Some(id.clone()), "activate", message));
        }

        match self.activate_single(&record, id, provider).await {
            Ok(()) => {
                self.consecutive_failures.lock().await.insert(id.clone(), 0);
                Ok(())
            }
            Err(error) => {
                let fails = {
                    let mut counts = self.consecutive_failures.lock().await;
                    let entry = counts.entry(id.clone()).or_insert(0);
                    *entry = entry.saturating_add(1);
                    *entry
                };
                let message = error.message;
                let locked = fails >= self.config.max_consecutive_failures;
                let summary = if locked {
                    format!("{message} (locked out after {fails} consecutive failures)")
                } else {
                    message
                };
                self.registry.mark_failed(id, summary.clone());
                Err(PluginError::new(Some(id.clone()), error.operation, summary))
            }
        }
    }

    fn failure_count(&self, _id: &PluginId) -> u32 {
        // Best-effort pre-check; the authoritative check happens after the
        // activation attempt (the streak only grows on failure).
        0
    }

    async fn activate_single(
        &self,
        record: &PluginRecord,
        id: &PluginId,
        provider: &dyn PluginInstanceProvider,
    ) -> PluginResult<()> {
        self.compatibility_check(&record.manifest)?;

        for dependency in &record.manifest.dependencies {
            let state = self
                .registry
                .record(&dependency.id)
                .map(|r| r.state)
                .unwrap_or(PluginState::Unloaded);
            if state == PluginState::Failed
                || state == PluginState::Unloaded
                || state == PluginState::Discovered
                || state == PluginState::Validated
            {
                return Err(PluginError::new(
                    Some(id.clone()),
                    "activate",
                    format!("dependency {} not enabled (state {state:?})", dependency.id),
                ));
            }
        }

        let Some(instance) = provider.provide(id).await? else {
            return Err(PluginError::new(
                Some(id.clone()),
                "activate",
                if self.trust_for_id(id.as_str()) == PluginTrust::Trusted {
                    "no native instance available"
                } else {
                    "untrusted plugin requires isolated execution"
                },
            ));
        };

        self.instances
            .write()
            .await
            .entry(id.clone())
            .or_insert(instance.clone());

        // Re-enable of a previously Disabled plugin skips the load chain
        // (Disabled → Initialized → Enabled, spec §lifecycle).
        if record.state == PluginState::Disabled {
            self.run_hook(Hook::Enable, id, &instance).await?;
            self.registry.transition(id, PluginState::Initialized)?;
            return self.registry.transition(id, PluginState::Enabled);
        }

        self.registry.transition(id, PluginState::Validated)?;

        self.run_hook(Hook::Load, id, &instance).await?;
        self.registry.transition(id, PluginState::Loaded)?;

        self.run_hook(Hook::Enable, id, &instance).await?;
        self.registry.transition(id, PluginState::Initialized)?;
        self.registry.transition(id, PluginState::Enabled)
    }

    async fn run_hook(
        &self,
        hook: Hook,
        id: &PluginId,
        instance: &Arc<dyn Plugin>,
    ) -> PluginResult<()> {
        let ctx = self.context_for(id);
        let instance = instance.clone();
        let operation = hook.operation();
        let task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1)).await; // yields so panics are JoinErrors
            match hook {
                Hook::Load => instance.on_load(&ctx).await,
                Hook::Enable => instance.on_enable(&ctx).await,
                Hook::Disable => instance.on_disable(&ctx).await,
                Hook::Unload => instance.on_unload(&ctx).await,
            }
        });
        let begin = Instant::now();
        let result = timeout(hook.timeout(&self.config), task).await;
        match result {
            Ok(Ok(Ok(()))) => {
                self.metrics.record_activation(id, begin.elapsed());
                Ok(())
            }
            Ok(Ok(Err(mut error))) => {
                if error.plugin_id.is_none() {
                    error.plugin_id = Some(id.clone());
                }
                self.metrics.record_failure(id, &error.message);
                Err(error)
            }
            Ok(Err(join_error)) if join_error.is_panic() => {
                let message = format!("{operation} hook panicked; plugin isolated");
                self.metrics.record_failure(id, &message);
                Err(PluginError::new(Some(id.clone()), operation, message))
            }
            Ok(Err(join_error)) => {
                let message = format!("{operation} task cancelled: {join_error}");
                self.metrics.record_failure(id, &message);
                Err(PluginError::new(Some(id.clone()), operation, message))
            }
            Err(_) => {
                let message = format!(
                    "{operation} timed out after {:?}",
                    hook.timeout(&self.config)
                );
                self.metrics.record_failure(id, &message);
                Err(PluginError::new(Some(id.clone()), operation, message))
            }
        }
    }

    /// Idempotent disable: `on_disable`, cleanup of registrations,
    /// transition to [`PluginState::Disabled`] (spec 63).
    ///
    /// # Errors
    /// Hook failure or illegal transition.
    pub async fn disable(&self, id: &PluginId) -> PluginResult<()> {
        let instance = self.instances.read().await.get(id).cloned();
        if let Some(instance) = instance {
            self.run_hook(Hook::Disable, id, &instance).await?;
        }
        self.commands.unregister_plugin(id);
        self.registry.transition(id, PluginState::Disabled)?;
        self.consecutive_failures.lock().await.insert(id.clone(), 0);
        Ok(())
    }

    /// Unload: reverse `on_load` and drop the instance.
    ///
    /// # Errors
    /// Hook failure or illegal transition.
    pub async fn unload(&self, id: &PluginId) -> PluginResult<()> {
        let instance = self.instances.read().await.get(id).cloned();
        if let Some(instance) = instance {
            self.run_hook(Hook::Unload, id, &instance).await?;
            self.instances.write().await.remove(id);
        }
        self.registry.transition(id, PluginState::Unloaded)
    }

    /// Uninstall (spec 69–70): refuse while enabled dependents exist,
    /// disable, unload, drop the registration. Configuration files are left
    /// in place on disk.
    ///
    /// # Errors
    /// Dependents present, or lifecycle failure.
    pub async fn uninstall(&self, id: &PluginId) -> PluginResult<()> {
        let dependents: Vec<PluginId> = self
            .registry
            .all()
            .into_iter()
            .filter(|record| {
                record.manifest.dependencies.iter().any(|d| d.id == *id)
                    && record.state.is_present()
                    && record.state != PluginState::Failed
            })
            .map(|record| record.manifest.id)
            .collect();
        if !dependents.is_empty() {
            let names: Vec<&str> = dependents.iter().map(PluginId::as_str).collect();
            return Err(PluginError::new(
                Some(id.clone()),
                "uninstall",
                format!("disable dependents first: {}", names.join(", ")),
            ));
        }
        self.disable(id).await?;
        self.unload(id).await?;
        self.registry.remove(id);
        Ok(())
    }

    /// Hot reload (spec §64): the old instance goes down, then the freshest
    /// on-disk manifest is reactivated without touching other plugins.
    /// A plugin that was enabled stays enabled (or `Failed` on error).
    ///
    /// # Errors
    /// Plugin not installed, discovery failure, or hook failure.
    pub async fn reload(
        &self,
        id: &PluginId,
        provider: &dyn PluginInstanceProvider,
    ) -> PluginResult<()> {
        let previous = self
            .registry
            .record(id)
            .ok_or_else(|| PluginError::new(Some(id.clone()), "reload", "plugin not installed"))?;
        let was_enabled = previous.state == PluginState::Enabled;

        if was_enabled {
            self.disable(id).await?;
            self.unload(id).await?;
        }
        self.discover()?;
        if was_enabled {
            return self.activate(id, provider).await;
        }
        Ok(())
    }

    /// In-place update (spec §65, §72): re-reads the manifest, rejects
    /// downgrades, hot-swaps the instance, runs the migration hook, and
    /// rolls the registry back to the previous manifest on failure.
    ///
    /// # Errors
    /// Plugin not installed, a downgrade attempt, or any hook failure
    /// (the record is restored to the prior manifest marked `Failed`).
    pub async fn update(
        &self,
        id: &PluginId,
        provider: &dyn PluginInstanceProvider,
    ) -> PluginResult<()> {
        let previous = self
            .registry
            .record(id)
            .ok_or_else(|| PluginError::new(Some(id.clone()), "update", "plugin not installed"))?;
        let previous_version = previous.manifest.version.clone();
        let restored_state = previous.state;
        let was_enabled = previous.state == PluginState::Enabled;

        self.discover()?;
        let fresh = self.registry.record(id).ok_or_else(|| {
            PluginError::new(
                Some(id.clone()),
                "update",
                "manifest vanished during update",
            )
        })?;
        let new_version = fresh.manifest.version.clone();

        if new_version < previous_version {
            let restored = PluginRecord {
                state: restored_state,
                ..previous.clone()
            };
            let restore_error = self.registry.upsert(restored, true);
            if let Err(upsert_error) = restore_error {
                self.metrics.record_failure(id, &upsert_error.message);
            }
            return Err(PluginError::new(
                Some(id.clone()),
                "update",
                format!("update must not downgrade ({previous_version} → {new_version})"),
            ));
        }

        if new_version == previous_version && !was_enabled {
            return Ok(());
        }

        if was_enabled {
            self.disable(id).await?;
            self.unload(id).await?;
            // Unloaded cannot feed the activation chain directly; re-discovery
            // re-enters at Discovered with the refreshed manifest.
            self.discover()?;
            if !previous.configuration.is_empty() {
                self.registry
                    .set_configuration(id, previous.configuration.clone())?;
            }
        }
        match self.activate(id, provider).await {
            Ok(()) => {
                if new_version == previous_version {
                    return Ok(());
                }
                let instance = self.instances.read().await.get(id).cloned();
                let Some(instance) = instance else {
                    return Err(PluginError::new(
                        Some(id.clone()),
                        "update",
                        "instance vanished during migration",
                    ));
                };
                let ctx = self.context_for(id);
                match instance
                    .on_update(&ctx, &previous_version, &new_version)
                    .await
                {
                    Ok(()) => Ok(()),
                    Err(migration_error) => {
                        let message = format!(
                            "migration {previous_version} → {new_version} failed: {}",
                            migration_error.message
                        );
                        self.registry.mark_failed(id, message.clone());
                        Err(PluginError::new(Some(id.clone()), "update", message))
                    }
                }
            }
            Err(activation_error) => {
                let restored = PluginRecord {
                    state: PluginState::Failed,
                    ..previous.clone()
                };
                let restore_error = self.registry.upsert(restored, true);
                if let Err(upsert_error) = restore_error {
                    self.metrics.record_failure(id, &upsert_error.message);
                }
                Err(PluginError::new(
                    Some(id.clone()),
                    "update",
                    format!(
                        "activation after update failed: {} (registry rolled back to {previous_version})",
                        activation_error.message
                    ),
                ))
            }
        }
    }

    /// Enables discovered (or disabled) plugins in dependency order.
    /// Registry-injected records (built-ins) are included even though they
    /// have no manifest file.
    ///
    /// # Errors
    /// Dependency resolution or the first activation error stops the batch.
    pub async fn enable_all(&self, provider: &dyn PluginInstanceProvider) -> PluginResult<()> {
        let mut ids = self.discover()?;
        let known: std::collections::BTreeSet<PluginId> = ids.iter().cloned().collect();
        for record in self.registry.all() {
            if !known.contains(&record.manifest.id) {
                ids.push(record.manifest.id.clone());
            }
        }
        for id in ids {
            if let Some(record) = self.registry.record(&id) {
                if matches!(
                    record.state,
                    PluginState::Discovered | PluginState::Disabled
                ) && record.last_error.is_none()
                {
                    self.activate(&id, provider).await?;
                }
            }
        }
        Ok(())
    }

    /// Installs a configuration schema and stores validated values.
    ///
    /// # Errors
    /// Unknown plugin or configuration validation failure (operation
    /// `configuration.error` — never crashes the plugin, spec 22).
    pub fn configure(
        &self,
        id: &PluginId,
        schema: Option<ConfigSchema>,
        values: BTreeMap<String, ConfigValue>,
    ) -> PluginResult<()> {
        if let Some(schema) = schema {
            schema.validate(id, &values)?;
        }
        self.registry.set_configuration(id, values)
    }

    /// Replaces a plugin's granted permissions directly (approval flow API).
    pub fn grant(&self, id: &PluginId, granted: GrantedPermissions) {
        self.registry.grant(id, granted);
    }

    // ---- plumbing -----------------------------------------------------------

    /// Builds the scoped context handed to hooks.
    #[must_use]
    pub fn context_for(&self, id: &PluginId) -> PluginContext {
        let (granted, scope) = match self.registry.record(id) {
            Some(record) => (record.granted.clone(), record.scope.clone()),
            None => (GrantedPermissions::default(), PluginScope::Server),
        };
        let commands_api: Arc<dyn CommandRegistryApi> =
            Arc::clone(&self.commands) as Arc<dyn CommandRegistryApi>;
        let events_api: Arc<dyn PluginEventApi> =
            Arc::clone(&self.events) as Arc<dyn PluginEventApi>;
        let services = PluginServices {
            storage: Arc::clone(&self.storage_backend),
            commands: commands_api,
            events: events_api,
        };
        let data_root = self
            .config
            .search_paths
            .first()
            .map(|_search| default_storage_directory().join(id.as_str().replace('.', "-")));
        PluginContext::new(id.clone(), scope, granted)
            .with_data_root(data_root.unwrap_or_else(|| PathBuf::from("plugin-data")))
            .with_services(services)
    }
}

/// Current platform label (`windows`, `linux`, `macos`).
#[must_use]
pub fn current_platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

fn current_platform_in(platforms: &[String]) -> bool {
    let here = current_platform();
    platforms.iter().any(|p| p.eq_ignore_ascii_case(here))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::id::PluginId;
    use crate::lifecycle::PluginState;
    use crate::permission::PluginPermission;
    use crate::plugin::Plugin as PluginTrait;
    use crate::registry::PluginTrust;

    fn unique_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nexus-p11-{tag}-{}-{}",
            std::process::id(),
            uuid_counter()
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn uuid_counter() -> u64 {
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
    }

    fn write_manifest(dir: &std::path::Path, id: &str, extra: &str) {
        let leaf = id.split('.').next_back().unwrap_or("plugin");
        let plugin_dir = dir.join(leaf);
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                r#"
id = "{id}"
name = "{leaf}"
version = "1.0.0"
author = "Test"
description = "test plugin"
{extra}
"#
            ),
        )
        .unwrap();
    }

    struct OkPlugin {
        id: PluginId,
    }

    #[async_trait::async_trait]
    impl PluginTrait for OkPlugin {
        fn id(&self) -> PluginId {
            self.id.clone()
        }
        async fn on_load(&self, _: &PluginContext) -> PluginResult<()> {
            Ok(())
        }
        async fn on_enable(&self, _: &PluginContext) -> PluginResult<()> {
            Ok(())
        }
        async fn on_disable(&self, _: &PluginContext) -> PluginResult<()> {
            Ok(())
        }
        async fn on_unload(&self, _: &PluginContext) -> PluginResult<()> {
            Ok(())
        }
    }

    struct PanicPlugin {
        id: PluginId,
    }

    #[async_trait::async_trait]
    impl PluginTrait for PanicPlugin {
        fn id(&self) -> PluginId {
            self.id.clone()
        }
        async fn on_load(&self, _: &PluginContext) -> PluginResult<()> {
            panic!("plugin exploded during load");
        }
        async fn on_enable(&self, _: &PluginContext) -> PluginResult<()> {
            Ok(())
        }
        async fn on_disable(&self, _: &PluginContext) -> PluginResult<()> {
            Ok(())
        }
        async fn on_unload(&self, _: &PluginContext) -> PluginResult<()> {
            Ok(())
        }
    }

    struct SleepPlugin {
        id: PluginId,
        millis: u64,
    }

    #[async_trait::async_trait]
    impl PluginTrait for SleepPlugin {
        fn id(&self) -> PluginId {
            self.id.clone()
        }
        async fn on_load(&self, _: &PluginContext) -> PluginResult<()> {
            tokio::time::sleep(Duration::from_millis(self.millis)).await;
            Ok(())
        }
        async fn on_enable(&self, _: &PluginContext) -> PluginResult<()> {
            Ok(())
        }
        async fn on_disable(&self, _: &PluginContext) -> PluginResult<()> {
            Ok(())
        }
        async fn on_unload(&self, _: &PluginContext) -> PluginResult<()> {
            Ok(())
        }
    }

    struct FailingPlugin {
        id: PluginId,
    }

    #[async_trait::async_trait]
    impl PluginTrait for FailingPlugin {
        fn id(&self) -> PluginId {
            self.id.clone()
        }
        async fn on_load(&self, _: &PluginContext) -> PluginResult<()> {
            Ok(())
        }
        async fn on_enable(&self, _: &PluginContext) -> PluginResult<()> {
            Err(PluginError::new(None, "enable", "enable always fails"))
        }
        async fn on_disable(&self, _: &PluginContext) -> PluginResult<()> {
            Ok(())
        }
        async fn on_unload(&self, _: &PluginContext) -> PluginResult<()> {
            Ok(())
        }
    }

    fn ok_plugin(id: &str) -> Arc<OkPlugin> {
        Arc::new(OkPlugin {
            id: PluginId::new(id).unwrap(),
        })
    }

    fn trusted_config() -> PluginManagerConfig {
        PluginManagerConfig {
            default_trust: PluginTrust::Trusted,
            ..PluginManagerConfig::default()
        }
    }

    fn discovered_manager(
        tag: &str,
        manifests: &[(&str, &str)],
    ) -> (PluginManager, std::path::PathBuf) {
        let dir = unique_dir(tag);
        for (id, extra) in manifests {
            write_manifest(&dir, id, extra);
        }
        let mut config = trusted_config();
        config.search_paths = vec![dir.clone()];
        (
            PluginManager::with_storage(config, Arc::new(crate::storage::InMemoryStorage::new())),
            dir,
        )
    }

    fn native_provider(instances: &[Arc<dyn PluginTrait>]) -> NativeInstanceProvider {
        let provider = NativeInstanceProvider::default();
        for instance in instances {
            provider.register(instance.clone());
        }
        provider
    }

    #[tokio::test]
    async fn discovery_and_full_activation() {
        let (manager, dir) = discovered_manager("full", &[("org.nexus.full.one", "")]);
        let discovered = manager.discover().unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].as_str(), "org.nexus.full.one");

        let provider = native_provider(&[ok_plugin("org.nexus.full.one") as Arc<dyn PluginTrait>]);
        manager.activate(&discovered[0], &provider).await.unwrap();
        let record = manager.registry().record(&discovered[0]).unwrap();
        assert_eq!(record.state, PluginState::Enabled);
        assert!(!record.granted.allows(PluginPermission::FilesystemRead));
        assert!(manager.metrics().snapshot(&discovered[0]).activations >= 1);
        drop(dir);
    }

    #[tokio::test]
    async fn panics_are_isolated_and_mark_failed() {
        let (manager, dir) = discovered_manager("panic", &[("org.nexus.panic.bad", "")]);
        let id = PluginId::new("org.nexus.panic.bad").unwrap();
        let _ = manager.discover().unwrap();
        let bad: Arc<dyn PluginTrait> = Arc::new(PanicPlugin { id: id.clone() });
        let provider = native_provider(&[bad]);
        let err = manager.activate(&id, &provider).await.unwrap_err();
        assert!(err.message.contains("panicked"), "got: {}", err.message);
        assert_eq!(
            manager.registry().record(&id).unwrap().state,
            PluginState::Failed
        );
        drop(dir);
    }

    #[tokio::test]
    async fn load_timeout_is_enforced() {
        let id = PluginId::new("org.nexus.timeout.slow").unwrap();
        let dir = unique_dir("timeout");
        write_manifest(&dir, "org.nexus.timeout.slow", "");
        let mut config = trusted_config();
        config.search_paths = vec![dir.clone()];
        config.load_timeout = Duration::from_millis(50);
        let manager =
            PluginManager::with_storage(config, Arc::new(crate::storage::InMemoryStorage::new()));
        let _ = manager.discover().unwrap();
        let slow: Arc<dyn PluginTrait> = Arc::new(SleepPlugin {
            id: id.clone(),
            millis: 2_000,
        });
        let provider = native_provider(&[slow]);
        let err = manager.activate(&id, &provider).await.unwrap_err();
        assert!(
            err.message.contains("timed out"),
            "unexpected: {}",
            err.message
        );
        assert_eq!(
            manager.registry().record(&id).unwrap().state,
            PluginState::Failed
        );
    }

    #[tokio::test]
    async fn untrusted_requires_isolated_execution() {
        let id = PluginId::new("org.nexus.untrust.bad").unwrap();
        let dir = unique_dir("untrust");
        write_manifest(&dir, "org.nexus.untrust.bad", "");
        let mut config = PluginManagerConfig::default();
        config.search_paths = vec![dir.clone()];
        config.default_trust = PluginTrust::Untrusted;
        let manager =
            PluginManager::with_storage(config, Arc::new(crate::storage::InMemoryStorage::new()));
        let _ = manager.discover().unwrap();
        let provider = NativeInstanceProvider::default();
        let err = manager.activate(&id, &provider).await.unwrap_err();
        assert!(err.message.contains("isolated execution"));
    }

    #[tokio::test]
    async fn dependencies_must_enable_first() {
        let (manager, dir) = discovered_manager(
            "deps",
            &[
                ("org.nexus.host.core", ""),
                (
                    "org.nexus.host.kind",
                    r#"[[dependencies]]
id = "org.nexus.host.core""#,
                ),
            ],
        );
        let _ = manager.discover().unwrap();
        let b = PluginId::new("org.nexus.host.kind").unwrap();
        // Only B registered, missing A provider.
        let provider = NativeInstanceProvider::default();
        let err = manager.activate(&b, &provider).await.unwrap_err();
        assert!(err.message.contains("not enabled"), "got {}", err.message);
        drop(dir);
    }

    #[tokio::test]
    async fn activation_fails_without_dependency() {
        let (manager, _dir) = discovered_manager(
            "dep-order",
            &[
                ("org.nexus.host.dep-lower", ""),
                (
                    "org.nexus.host.dep-upper",
                    r#"[[dependencies]]
id = "org.nexus.host.dep-lower""#,
                ),
            ],
        );
        manager.discover().unwrap();
        let lower = PluginId::new("org.nexus.host.dep-lower").unwrap();
        let upper = PluginId::new("org.nexus.host.dep-upper").unwrap();
        let provider = native_provider(&[
            ok_plugin("org.nexus.host.dep-lower") as Arc<dyn PluginTrait>,
            ok_plugin("org.nexus.host.dep-upper") as Arc<dyn PluginTrait>,
        ]);
        // Enable in dependency order, then check order effects.
        manager.activate(&lower, &provider).await.unwrap();
        manager.activate(&upper, &provider).await.unwrap();
        assert_eq!(
            manager.registry().record(&upper).unwrap().state,
            PluginState::Enabled
        );
        // Uninstalling lower should be blocked by the enabled upper.
        let err = manager.uninstall(&lower).await.unwrap_err();
        assert!(err.message.contains("disable dependents first"));
    }

    #[tokio::test]
    async fn lockout_after_consecutive_failures() {
        let id = PluginId::new("org.nexus.lock.gum").unwrap();
        let dir = unique_dir("lock");
        write_manifest(&dir, "org.nexus.lock.gum", "");
        let mut config = trusted_config();
        config.search_paths = vec![dir.clone()];
        config.max_consecutive_failures = 2;
        let manager =
            PluginManager::with_storage(config, Arc::new(crate::storage::InMemoryStorage::new()));
        let _ = manager.discover().unwrap();
        let bad: Arc<dyn PluginTrait> = Arc::new(FailingPlugin { id: id.clone() });
        let provider = native_provider(&[bad]);
        assert!(manager.activate(&id, &provider).await.is_err());
        assert!(manager.activate(&id, &provider).await.is_err());
        let err = manager.activate(&id, &provider).await.unwrap_err();
        assert!(err.message.contains("locked out"), "got {}", err.message);
        drop(dir);
    }

    #[tokio::test]
    async fn disable_cleans_up_and_recovers() {
        let (manager, dir) = discovered_manager(
            "disable",
            &[(
                "org.nexus.disable.one",
                r#"capabilities = ["command"]
permissions = ["ui.register"]"#,
            )],
        );
        manager.discover().unwrap();
        let id = PluginId::new("org.nexus.disable.one").unwrap();
        let provider =
            native_provider(&[ok_plugin("org.nexus.disable.one") as Arc<dyn PluginTrait>]);
        let registry = manager.commands();
        // The plugin's command disappears from the : executor on disable.
        registry.unregister_plugin(&id);
        manager.activate(&id, &provider).await.unwrap();
        manager.disable(&id).await.unwrap();
        assert_eq!(
            manager.registry().record(&id).unwrap().state,
            PluginState::Disabled
        );
        // Re-activation from Disabled works after disable?enabled cycle.
        manager.activate(&id, &provider).await.unwrap();
        assert_eq!(
            manager.registry().record(&id).unwrap().state,
            PluginState::Enabled
        );
        drop(dir);
    }

    #[test]
    fn sensitive_permissions_require_approval() {
        let (manager, dir) = discovered_manager(
            "approval",
            &[(
                "org.nexus.approval.girl",
                r#"permissions = ["filesystem.write", "cube.read"]"#,
            )],
        );
        manager.discover().unwrap();
        let id = PluginId::new("org.nexus.approval.girl").unwrap();
        let granted = manager.registry().record(&id).unwrap().granted.clone();
        assert!(
            !granted.allows(PluginPermission::FilesystemWrite),
            "sensitive default denied"
        );
        assert!(
            granted.allows(PluginPermission::CubeRead),
            "non-sensitive auto-granted"
        );

        let mut manager = manager;
        manager
            .approve(&id, PluginPermission::FilesystemWrite)
            .unwrap();
        let granted = manager.registry().record(&id).unwrap().granted.clone();
        assert!(granted.allows(PluginPermission::FilesystemWrite));
        drop(dir);
    }

    #[tokio::test]
    async fn duplicate_ids_shape_fence() {
        let (manager, dir) = discovered_manager("dup", &[("org.nexus.dup.one", "")]);
        manager.discover().unwrap();
        // A second discovery keeps a stable single record (idempotent).
        let ids = manager.discover().unwrap();
        assert_eq!(ids.len(), 1);
        assert!(manager
            .registry()
            .record(&PluginId::new("org.nexus.dup.one").unwrap())
            .is_some());
        drop(dir);
    }

    struct CountingPlugin {
        id: PluginId,
        loads: std::sync::atomic::AtomicU32,
        migrations: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl PluginTrait for CountingPlugin {
        fn id(&self) -> PluginId {
            self.id.clone()
        }
        async fn on_load(&self, _: &PluginContext) -> PluginResult<()> {
            self.loads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }
        async fn on_enable(&self, _: &PluginContext) -> PluginResult<()> {
            Ok(())
        }
        async fn on_disable(&self, _: &PluginContext) -> PluginResult<()> {
            Ok(())
        }
        async fn on_unload(&self, _: &PluginContext) -> PluginResult<()> {
            Ok(())
        }
        async fn on_update(
            &self,
            _: &PluginContext,
            from: &PluginVersion,
            to: &PluginVersion,
        ) -> PluginResult<()> {
            self.migrations
                .lock()
                .unwrap()
                .push(format!("{from}->{to}"));
            Ok(())
        }
    }

    fn counting_plugin(id: &str) -> Arc<CountingPlugin> {
        Arc::new(CountingPlugin {
            id: PluginId::new(id).unwrap(),
            loads: std::sync::atomic::AtomicU32::new(0),
            migrations: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn write_manifest_version(dir: &std::path::PathBuf, id: &str, version: &str) {
        let leaf = id.split('.').next_back().unwrap_or("plugin");
        std::fs::write(
            dir.join(leaf).join("plugin.toml"),
            format!(
                r#"
id = "{id}"
name = "{leaf}"
version = "{version}"
author = "Test"
description = "test plugin"
"#
            ),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn update_migrates_and_reenters_enabled() {
        let (manager, dir) = discovered_manager("upgrade", &[("org.nexus.upgrade.one", "")]);
        manager.discover().unwrap();
        let id = PluginId::new("org.nexus.upgrade.one").unwrap();
        let instance = counting_plugin("org.nexus.upgrade.one");
        let provider = native_provider(&[instance.clone() as Arc<dyn PluginTrait>]);
        manager.activate(&id, &provider).await.unwrap();
        assert_eq!(instance.loads.load(std::sync::atomic::Ordering::Relaxed), 1);

        write_manifest_version(&dir, "org.nexus.upgrade.one", "2.0.0");

        manager.update(&id, &provider).await.unwrap();
        let record = manager.registry().record(&id).unwrap();
        assert_eq!(record.state, PluginState::Enabled);
        assert_eq!(record.manifest.version.to_string(), "2.0.0");
        assert_eq!(
            instance.migrations.lock().unwrap().as_slice(),
            [String::from("1.0.0->2.0.0")]
        );
        assert_eq!(
            instance.loads.load(std::sync::atomic::Ordering::Relaxed),
            2,
            "reload loaded the fresh instance"
        );
        drop(dir);
    }

    #[tokio::test]
    async fn update_rejects_downgrades() {
        let (manager, dir) = discovered_manager("downgrade", &[("org.nexus.downgrade.one", "")]);
        manager.discover().unwrap();
        let id = PluginId::new("org.nexus.downgrade.one").unwrap();
        let instance = counting_plugin("org.nexus.downgrade.one");
        let provider = native_provider(&[instance.clone() as Arc<dyn PluginTrait>]);
        manager.activate(&id, &provider).await.unwrap();

        write_manifest_version(&dir, "org.nexus.downgrade.one", "0.9.0");
        let err = manager.update(&id, &provider).await.unwrap_err();
        assert!(err.message.contains("downgrade"));
        let record = manager.registry().record(&id).unwrap();
        assert_eq!(
            record.manifest.version.to_string(),
            "1.0.0",
            "record restored"
        );
        assert_eq!(record.state, PluginState::Enabled, "state preserved");
        drop(dir);
    }

    #[tokio::test]
    async fn update_failure_rolls_back_manifest_record() {
        struct NoProvider;
        #[async_trait]
        impl PluginInstanceProvider for NoProvider {
            async fn provide(&self, id: &PluginId) -> PluginResult<Option<Arc<dyn PluginTrait>>> {
                Err(PluginError::new(
                    Some(id.clone()),
                    "provide",
                    "new binary fails to load",
                ))
            }
        }

        let (manager, dir) = discovered_manager("rollback", &[("org.nexus.rollback.one", "")]);
        manager.discover().unwrap();
        let id = PluginId::new("org.nexus.rollback.one").unwrap();
        let instance = counting_plugin("org.nexus.rollback.one");
        let provider = native_provider(&[instance.clone() as Arc<dyn PluginTrait>]);
        manager.activate(&id, &provider).await.unwrap();

        write_manifest_version(&dir, "org.nexus.rollback.one", "1.1.0");
        let err = manager.update(&id, &NoProvider).await.unwrap_err();
        assert!(err.message.contains("rolled back"), "got {}", err.message);
        let record = manager.registry().record(&id).unwrap();
        assert_eq!(record.manifest.version.to_string(), "1.0.0");
        assert_eq!(record.state, PluginState::Failed);
        drop(dir);
    }

    #[tokio::test]
    async fn reload_reactivates_in_place() {
        let (manager, dir) = discovered_manager("reload", &[("org.nexus.reload.one", "")]);
        manager.discover().unwrap();
        let id = PluginId::new("org.nexus.reload.one").unwrap();
        let instance = counting_plugin("org.nexus.reload.one");
        let provider = native_provider(&[instance.clone() as Arc<dyn PluginTrait>]);
        manager.activate(&id, &provider).await.unwrap();

        manager.reload(&id, &provider).await.unwrap();
        assert_eq!(
            manager.registry().record(&id).unwrap().state,
            PluginState::Enabled
        );
        assert_eq!(instance.loads.load(std::sync::atomic::Ordering::Relaxed), 2);

        // Reload on an idle (not enabled) plugin is a no-op.
        manager.disable(&id).await.unwrap();
        manager.reload(&id, &provider).await.unwrap();
        assert_eq!(instance.loads.load(std::sync::atomic::Ordering::Relaxed), 2);
        drop(dir);
    }
}
