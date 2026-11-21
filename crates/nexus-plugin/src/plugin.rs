//! Runtime plugin contract: lifecycle hooks every plugin implements.
//!
//! Hooks are kept deliberately small (`on_load`, `on_enable`, `on_disable`,
//! `on_unload`); broader integration happens through the event system and
//! the capability sub-traits (language, build, runtime, ...) so the platform
//! does not grow per-feature hooks.

use async_trait::async_trait;

use crate::context::PluginContext;
use crate::error::PluginResult;
use crate::id::PluginId;
use crate::version::PluginVersion;

/// A loaded plugin implementation.
///
/// Implementations receive a scoped [`PluginContext`] for every hook; the
/// context is revoked (commands unregistered, subscriptions dropped,
/// storage namespace rotated) when the plugin is disabled or unloaded.
#[async_trait]
pub trait Plugin: Send + Sync {
    /// The stable identity, matching the manifest.
    fn id(&self) -> PluginId;

    /// `on_load(context)` — allocate resources, register nothing yet.
    /// A timeout-bounded call; failure marks the plugin [`crate::lifecycle::PluginState::Failed`].
    async fn on_load(&self, ctx: &PluginContext) -> PluginResult<()>;

    /// `on_enable(context)` — register commands, subscriptions, UI surfaces.
    async fn on_enable(&self, ctx: &PluginContext) -> PluginResult<()>;

    /// `on_disable(context)` — reverse `on_enable`: unregister everything.
    async fn on_disable(&self, ctx: &PluginContext) -> PluginResult<()>;

    /// `on_unload(context)` — release all remaining resources.
    async fn on_unload(&self, ctx: &PluginContext) -> PluginResult<()>;

    /// `on_update(context, from, to)` — version migration hook (spec §72),
    /// invoked with the new instance after the manifest version changed and
    /// the plugin activated successfully; the default ignores the bump.
    async fn on_update(
        &self,
        _ctx: &PluginContext,
        _from: &PluginVersion,
        _to: &PluginVersion,
    ) -> PluginResult<()> {
        Ok(())
    }
}
