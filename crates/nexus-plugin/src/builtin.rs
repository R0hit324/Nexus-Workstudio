//! Built-in reference plugins.
//!
//! These are the platform's canonical example integrations (Part 11 §72):
//! they demonstrate the full plugin API — lifecycle hooks, command
//! registration through the scoped context, and storage usage — and ship
//! registered as trusted built-ins by hosts.

use std::sync::Arc;

use async_trait::async_trait;

use crate::commands::{CommandArgument, CommandDescriptor, CommandHandler, CommandOutput};
use crate::context::PluginContext;
use crate::error::{PluginError, PluginResult};
use crate::id::PluginId;
use crate::plugin::Plugin;
use crate::storage::PluginStorageBackend;

fn descriptor_for(
    owner: &PluginId,
    id: &str,
    name: &str,
    description: &str,
    arguments: Vec<CommandArgument>,
    execution_permissions: Vec<crate::permission::PluginPermission>,
) -> CommandDescriptor {
    CommandDescriptor {
        id: String::from(id),
        name: String::from(name),
        description: String::from(description),
        arguments,
        execution_permissions,
        owner: owner.clone(),
    }
}

fn services(ctx: &PluginContext) -> PluginResult<&crate::context::PluginServices> {
    ctx.services().ok_or_else(|| {
        PluginError::new(
            Some(ctx.plugin_id().clone()),
            "builtin",
            "plugin context has no services facade",
        )
    })
}

// ---- Git built-in --------------------------------------------------------

struct GitStatusHandler;

#[async_trait]
impl CommandHandler for GitStatusHandler {
    async fn invoke(
        &self,
        _caller: &PluginContext,
        _command: &CommandDescriptor,
        _arguments: &[String],
    ) -> PluginResult<CommandOutput> {
        Ok(CommandOutput {
            text: String::from("git: working tree clean (builtin stub)"),
            failed: false,
        })
    }

    async fn complete(
        &self,
        _caller: &PluginContext,
        _command: &CommandDescriptor,
        _index: usize,
        _current: &str,
    ) -> Vec<String> {
        Vec::new()
    }
}

struct GitLogHandler;

#[async_trait]
impl CommandHandler for GitLogHandler {
    async fn invoke(
        &self,
        _caller: &PluginContext,
        _command: &CommandDescriptor,
        arguments: &[String],
    ) -> PluginResult<CommandOutput> {
        let count = arguments
            .first()
            .map_or(10, |text| text.parse().unwrap_or(10));
        Ok(CommandOutput {
            text: format!("git: showing {count} recent commits (builtin stub)"),
            failed: false,
        })
    }

    async fn complete(
        &self,
        _caller: &PluginContext,
        _command: &CommandDescriptor,
        _index: usize,
        _current: &str,
    ) -> Vec<String> {
        Vec::new()
    }
}

/// Git hub integration: exposes `git.*` commands.
pub struct GitPlugin {
    id: PluginId,
}

impl GitPlugin {
    /// The built-in git plugin instance.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: PluginId::new("org.nexus.git").expect("valid builtin id"),
        }
    }
}

impl Default for GitPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for GitPlugin {
    fn id(&self) -> PluginId {
        self.id.clone()
    }

    async fn on_load(&self, ctx: &PluginContext) -> PluginResult<()> {
        let services = services(ctx)?;
        services
            .commands
            .register(
                descriptor_for(
                    &self.id,
                    "git.status",
                    "git status",
                    "Summarizes working tree state.",
                    Vec::new(),
                    Vec::new(),
                ),
                Arc::new(GitStatusHandler),
            )
            .await?;
        services
            .commands
            .register(
                descriptor_for(
                    &self.id,
                    "git.log",
                    "git log",
                    "Shows recent commit history.",
                    vec![CommandArgument {
                        name: "count".to_owned(),
                        description: "Number of commits to show.".to_owned(),
                        required: false,
                    }],
                    Vec::new(),
                ),
                Arc::new(GitLogHandler),
            )
            .await
    }

    async fn on_enable(&self, _ctx: &PluginContext) -> PluginResult<()> {
        Ok(())
    }

    async fn on_disable(&self, _ctx: &PluginContext) -> PluginResult<()> {
        Ok(())
    }

    async fn on_unload(&self, _ctx: &PluginContext) -> PluginResult<()> {
        Ok(())
    }
}

// ---- Rust language built-in ----------------------------------------------

struct RustCheckHandler {
    owner: PluginId,
    storage: Arc<dyn PluginStorageBackend>,
}

#[async_trait]
impl CommandHandler for RustCheckHandler {
    async fn invoke(
        &self,
        _caller: &PluginContext,
        _command: &CommandDescriptor,
        _arguments: &[String],
    ) -> PluginResult<CommandOutput> {
        let counter = "checks-run";
        let current = self
            .storage
            .get(&self.owner, counter)
            .await?
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|text| text.parse::<u32>().ok())
            .unwrap_or(0);
        self.storage
            .set(
                &self.owner,
                counter,
                (current + 1).to_string().into_bytes(),
                crate::storage::StorageQuota::default_plugin(),
            )
            .await
            .map_err(|error| {
                PluginError::new(Some(self.owner.clone()), "rust.check", error.message)
            })?;
        Ok(CommandOutput {
            text: format!("cargo check finished (run #{})", current + 1),
            failed: false,
        })
    }

    async fn complete(
        &self,
        _caller: &PluginContext,
        _command: &CommandDescriptor,
        _index: usize,
        _current: &str,
    ) -> Vec<String> {
        Vec::new()
    }
}

/// Rust language support builtin: exposes `rust.*` commands and counts runs
/// through its storage namespace.
pub struct RustPlugin {
    id: PluginId,
}

impl RustPlugin {
    /// The built-in rust plugin instance.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: PluginId::new("org.nexus.rust").expect("valid builtin id"),
        }
    }
}

impl Default for RustPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for RustPlugin {
    fn id(&self) -> PluginId {
        self.id.clone()
    }

    async fn on_load(&self, ctx: &PluginContext) -> PluginResult<()> {
        let services = services(ctx)?;
        services
            .commands
            .register(
                descriptor_for(
                    &self.id,
                    "rust.check",
                    "rust check",
                    "Runs cargo check for the workspace.",
                    Vec::new(),
                    Vec::new(),
                ),
                Arc::new(RustCheckHandler {
                    owner: self.id.clone(),
                    storage: Arc::clone(&services.storage),
                }),
            )
            .await
    }

    async fn on_enable(&self, _ctx: &PluginContext) -> PluginResult<()> {
        Ok(())
    }

    async fn on_disable(&self, _ctx: &PluginContext) -> PluginResult<()> {
        Ok(())
    }

    async fn on_unload(&self, _ctx: &PluginContext) -> PluginResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CommandRegistry, CommandRegistryApi};
    use crate::context::{PluginScope, PluginServices};
    use crate::events::PluginEventHub;
    use crate::permission::{GrantedPermissions, PluginPermission};
    use crate::storage::{InMemoryStorage, PluginStorageBackend};

    fn context_with_services(id: &str) -> PluginContext {
        let commands: Arc<dyn CommandRegistryApi> =
            Arc::new(CommandRegistry::new()) as Arc<dyn CommandRegistryApi>;
        let events: Arc<dyn crate::events::PluginEventApi> =
            Arc::new(PluginEventHub::new(true, true, 240))
                as Arc<dyn crate::events::PluginEventApi>;
        let storage: Arc<dyn PluginStorageBackend> =
            Arc::new(InMemoryStorage::new()) as Arc<dyn PluginStorageBackend>;
        let services = PluginServices {
            storage,
            commands,
            events,
        };
        PluginContext::new(
            PluginId::new(id).unwrap(),
            PluginScope::Server,
            GrantedPermissions::from_set([PluginPermission::UiRegister]),
        )
        .with_services(services)
    }

    #[tokio::test]
    async fn git_builtin_registers_commands() {
        let git = GitPlugin::new();
        let ctx = context_with_services("org.nexus.git");
        git.on_load(&ctx)
            .await
            .expect("git builtin installs cleanly");
        let host = context_with_services("org.nexus.guest");
        let out = ctx
            .services()
            .unwrap()
            .commands
            .execute(&host, "git.status", &[])
            .await
            .unwrap();
        assert!(out.text.contains("git:"));
    }

    #[tokio::test]
    async fn git_log_counts_argument() {
        let git = GitPlugin::new();
        let ctx = context_with_services("org.nexus.git");
        git.on_load(&ctx).await.unwrap();
        let host = context_with_services("org.nexus.guest");
        let out = ctx
            .services()
            .unwrap()
            .commands
            .execute(&host, "git.log", &["7".to_owned()])
            .await
            .unwrap();
        assert!(out.text.contains("7"), "output: {}", out.text);
    }

    #[tokio::test]
    async fn rust_builtin_counts_runs_in_storage() {
        let rust = RustPlugin::new();
        let ctx = context_with_services("org.nexus.rust");
        rust.on_load(&ctx).await.unwrap();
        let host = context_with_services("org.nexus.guest");
        let api = &ctx.services().unwrap().commands;
        api.execute(&host, "rust.check", &[]).await.unwrap();
        let out = api.execute(&host, "rust.check", &[]).await.unwrap();
        assert!(out.text.contains("run #2"), "output: {}", out.text);
    }
}
