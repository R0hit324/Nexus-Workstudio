//! Plugin commands: the `:`-command registry.
//!
//! Commands are namespaced (`git.status`, `docker.ps`, `debug.start`). A
//! plugin may only register command ids inside its own namespace, core
//! commands are reserved, and collisions with another plugin's namespace
//! are rejected rather than silently overwritten.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use crate::context::PluginContext;
use crate::error::{PluginError, PluginResult};
use crate::id::PluginId;
use crate::permission::{GrantedPermissions, PluginPermission};

/// Describes one command argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandArgument {
    pub name: String,
    pub description: String,
    pub required: bool,
}

/// Static description of a registered command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandDescriptor {
    /// Fully-qualified namespaced id, e.g. `git.status`.
    pub id: String,
    /// Human title.
    pub name: String,
    /// One-line description shown by help/completion UIs.
    pub description: String,
    /// Ordered argument specs.
    pub arguments: Vec<CommandArgument>,
    /// Permissions the caller must hold when executing.
    pub execution_permissions: Vec<PluginPermission>,
    /// The registering plugin.
    pub owner: PluginId,
}

impl CommandDescriptor {
    /// The namespace (text before the first dot).
    #[must_use]
    pub fn namespace(&self) -> Option<&str> {
        self.id.split('.').next()
    }

    /// The leaf command name.
    #[must_use]
    pub fn leaf(&self) -> &str {
        self.id.rsplit('.').next().unwrap_or(&self.id)
    }
}

/// Result of executing a plugin command.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandOutput {
    /// Plain-text result rendered to the command line.
    pub text: String,
    /// Whether the command errored (keeps text intact for display).
    pub failed: bool,
}

/// Async command handler implemented by plugins.
#[async_trait]
pub trait CommandHandler: Send + Sync {
    /// Executes the command with the caller context.
    ///
    /// # Errors
    /// [`PluginError`] on failure; the failure is scoped to the plugin.
    async fn invoke(
        &self,
        caller: &PluginContext,
        command: &CommandDescriptor,
        arguments: &[String],
    ) -> PluginResult<CommandOutput>;

    /// Returns completion candidates for `current` at `index`.
    async fn complete(
        &self,
        caller: &PluginContext,
        command: &CommandDescriptor,
        index: usize,
        current: &str,
    ) -> Vec<String>;
}

/// The registration/allocation facade exposed to plugins through
/// [`crate::context::PluginServices`].
#[async_trait]
pub trait CommandRegistryApi: Send + Sync {
    /// Registers a command owned by this plugin.
    ///
    /// # Errors
    /// Namespace/ownership/collision rules enforced.
    async fn register(
        &self,
        descriptor: CommandDescriptor,
        handler: Arc<dyn CommandHandler>,
    ) -> PluginResult<()>;

    /// Lists commands of this plugin's namespace.
    async fn own_commands(&self, owner: &PluginId) -> Vec<CommandDescriptor>;

    /// Executes a registered command (route behind permission checks).
    ///
    /// # Errors
    /// Unknown command, caller lacking permission, or handler failure.
    async fn execute(
        &self,
        caller: &PluginContext,
        command_id: &str,
        arguments: &[String],
    ) -> PluginResult<CommandOutput>;
}

struct Registration {
    descriptor: CommandDescriptor,
    handler: Arc<dyn CommandHandler>,
    core: bool,
}

/// The full server/client command registry.
#[derive(Default)]
pub struct CommandRegistry {
    by_id: RwLock<BTreeMap<String, Registration>>,
    /// Namespaces owned by plugins (`git` -> `org.nexus.git`).
    namespace_owner: RwLock<BTreeMap<String, PluginId>>,
}

/// Namespaces plugins may never claim.
pub const RESERVED_NAMESPACES: &[&str] = &[
    "core",
    "workspace",
    "snapshot",
    "runtime",
    "terminal",
    "build",
    "debug",
];

impl CommandRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a built-in (platform) command; plugins may not shadow it.
    ///
    /// # Errors
    /// [`PluginError`] if the id is malformed.
    pub fn register_core(
        &self,
        id: &str,
        name: &str,
        description: &str,
        handler: Arc<dyn CommandHandler>,
    ) -> PluginResult<()> {
        if id.split('.').count() != 1 && !id.contains('.') {
            return Err(PluginError::internal(format!(
                "core command id `{id}` must be plain (no namespace)"
            )));
        }
        let descriptor = CommandDescriptor {
            id: String::from(id),
            name: String::from(name),
            description: String::from(description),
            arguments: Vec::new(),
            execution_permissions: Vec::new(),
            owner: PluginId::new("org.nexus.core.command")?,
        };
        let entry = Registration {
            descriptor,
            handler,
            core: true,
        };
        self.by_id
            .write()
            .expect("command registry lock")
            .insert(String::from(id), entry);
        Ok(())
    }

    /// Executes a command by id.
    ///
    /// # Errors
    /// Unknown command, permission denied, re-dispatched handler error.
    pub async fn execute(
        &self,
        caller: &PluginContext,
        id: &str,
        arguments: &[String],
    ) -> PluginResult<CommandOutput> {
        let (descriptor, handler) = {
            let by_id = self.by_id.read().expect("command registry lock");
            let entry = by_id.get(id).ok_or_else(|| {
                PluginError::new(
                    Some(caller.plugin_id().clone()),
                    "command.execute",
                    format!("unknown command `{id}`"),
                )
            })?;
            (entry.descriptor.clone(), entry.handler.clone())
        };
        for permission in &descriptor.execution_permissions {
            if !caller.allows(*permission) {
                return Err(PluginError::new(
                    Some(caller.plugin_id().clone()),
                    "command.execute",
                    format!("caller lacks `{}` for `{id}`", permission.as_str()),
                ));
            }
        }
        handler.invoke(caller, &descriptor, arguments).await
    }

    /// Namespace-completion candidates for a typed prefix.
    #[must_use]
    pub fn complete(&self, typed: &str) -> Vec<String> {
        let by_id = self.by_id.read().expect("command registry lock");
        let mut candidates = Vec::new();
        for value in by_id.values() {
            let descriptor = &value.descriptor;
            if descriptor.id.starts_with(typed) {
                candidates.push(descriptor.id.clone());
            }
        }
        candidates.sort();
        candidates
    }

    /// All descriptors visible to the given caller.
    #[must_use]
    pub fn list(&self) -> Vec<CommandDescriptor> {
        let by_id = self.by_id.read().expect("command registry lock");
        by_id.values().map(|r| r.descriptor.clone()).collect()
    }

    /// Descriptors owned by one plugin.
    #[must_use]
    pub fn list_owned(&self, owner: &PluginId) -> Vec<CommandDescriptor> {
        let by_id = self.by_id.read().expect("command registry lock");
        by_id
            .values()
            .filter(|r| r.descriptor.owner == *owner)
            .map(|r| r.descriptor.clone())
            .collect()
    }

    /// Unregisters every command owned by `owner` (plugin cleanup, §63).
    pub fn unregister_plugin(&self, owner: &PluginId) {
        let mut by_id = self.by_id.write().expect("command registry lock");
        let mut namespace_owner = self.namespace_owner.write().expect("owner lock");
        let owned: Vec<String> = by_id
            .values()
            .filter(|r| r.descriptor.owner == *owner && !r.core)
            .map(|r| r.descriptor.id.clone())
            .collect();
        for id in owned {
            if let Some(removed) = by_id.remove(&id) {
                if let Some(namespace) = removed.descriptor.namespace() {
                    if namespace_owner.get(namespace) == Some(owner) {
                        namespace_owner.remove(namespace);
                    }
                }
            }
        }
    }
}

#[async_trait]
impl CommandRegistryApi for CommandRegistry {
    async fn register(
        &self,
        descriptor: CommandDescriptor,
        handler: Arc<dyn CommandHandler>,
    ) -> PluginResult<()> {
        if descriptor.id.contains('.') {
            let namespace = descriptor.namespace().unwrap_or_default();
            if RESERVED_NAMESPACES.contains(&namespace) {
                return Err(PluginError::new(
                    Some(descriptor.owner.clone()),
                    "command.register",
                    format!("namespace `{namespace}` is reserved"),
                ));
            }
            let mut namespace_owner = self.namespace_owner.write().expect("owner lock");
            if let Some(existing) = namespace_owner.get(namespace) {
                if existing != &descriptor.owner {
                    return Err(PluginError::new(
                        Some(descriptor.owner.clone()),
                        "command.register",
                        format!("namespace `{namespace}` already owned by {existing}"),
                    ));
                }
            } else {
                namespace_owner.insert(String::from(namespace), descriptor.owner.clone());
                drop(namespace_owner);
            }
        } else if !descriptor.id.is_empty() {
            return Err(PluginError::new(
                Some(descriptor.owner.clone()),
                "command.register",
                "plugin command ids must be namespaced (`vendor.plugin.leaf` or `namespace.leaf`)",
            ));
        }

        let by_id = self.by_id.read().expect("command registry lock");
        if let Some(existing) = by_id.get(&descriptor.id) {
            if existing.core || existing.descriptor.owner != descriptor.owner {
                drop(by_id);
                return Err(PluginError::new(
                    Some(descriptor.owner.clone()),
                    "command.register",
                    format!("`{}` already registered or reserved", descriptor.id),
                ));
            }
            drop(by_id);
            return Ok(());
        }
        drop(by_id);

        let validated = validate_descriptor_shape(&descriptor)?;
        let mut by_id = self.by_id.write().expect("command registry lock");
        if by_id.contains_key(&descriptor.id) {
            return Err(PluginError::new(
                Some(descriptor.owner.clone()),
                "command.register",
                format!("`{}` just registered concurrently", descriptor.id),
            ));
        }
        let entry = Registration {
            descriptor: validated,
            handler,
            core: false,
        };
        by_id.insert(descriptor.id.clone(), entry);
        Ok(())
    }

    async fn own_commands(&self, owner: &PluginId) -> Vec<CommandDescriptor> {
        self.list_owned(owner)
    }

    async fn execute(
        &self,
        caller: &PluginContext,
        command_id: &str,
        arguments: &[String],
    ) -> PluginResult<CommandOutput> {
        self.execute(caller, command_id, arguments).await
    }
}

fn validate_descriptor_shape(descriptor: &CommandDescriptor) -> PluginResult<CommandDescriptor> {
    if descriptor.name.trim().is_empty() {
        return Err(PluginError::new(
            Some(descriptor.owner.clone()),
            "command.register",
            format!("command `{}` needs a display name", descriptor.id),
        ));
    }
    if !descriptor
        .id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-' || c == '_')
    {
        return Err(PluginError::new(
            Some(descriptor.owner.clone()),
            "command.register",
            format!("command id `{}` must be lowercase", descriptor.id),
        ));
    }
    Ok(descriptor.clone())
}

/// Convenience: builds a [`CommandDescriptor`] in one call.
#[must_use]
pub fn descriptor(
    owner: &PluginId,
    id: &str,
    name: &str,
    description: &str,
    arguments: &[(&str, &str, bool)],
    execution_permissions: &[PluginPermission],
) -> CommandDescriptor {
    CommandDescriptor {
        id: String::from(id),
        name: String::from(name),
        description: String::from(description),
        arguments: arguments
            .iter()
            .map(|(name, description, required)| CommandArgument {
                name: String::from(*name),
                description: String::from(*description),
                required: *required,
            })
            .collect(),
        execution_permissions: execution_permissions.to_vec(),
        owner: owner.clone(),
    }
}

/// Whether the caller meets a descriptor's execution permissions.
#[must_use]
pub fn caller_allowed(granted: &GrantedPermissions, descriptor: &CommandDescriptor) -> bool {
    descriptor
        .execution_permissions
        .iter()
        .all(|p| granted.allows(*p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::PluginId;

    struct HelloCommand;

    #[async_trait::async_trait]
    impl CommandHandler for HelloCommand {
        async fn invoke(
            &self,
            _caller: &PluginContext,
            _command: &CommandDescriptor,
            arguments: &[String],
        ) -> PluginResult<CommandOutput> {
            Ok(CommandOutput {
                text: format!(
                    "hello {}",
                    arguments.first().map(String::as_str).unwrap_or("world")
                ),
                failed: false,
            })
        }

        async fn complete(
            &self,
            _caller: &PluginContext,
            _command: &CommandDescriptor,
            _index: usize,
            current: &str,
        ) -> Vec<String> {
            ["alpha", "beta"]
                .iter()
                .filter(|c| c.starts_with(current))
                .map(|c| String::from(*c))
                .collect()
        }
    }

    fn git_id() -> PluginId {
        PluginId::new("org.nexus.git").unwrap()
    }

    fn rust_id() -> PluginId {
        PluginId::new("org.nexus.rust").unwrap()
    }

    fn git_context() -> PluginContext {
        let permissions = GrantedPermissions::from_set([PluginPermission::UiRegister]);
        PluginContext::new(git_id(), crate::context::PluginScope::Server, permissions)
    }

    #[tokio::test]
    async fn registers_and_executes() {
        let registry = CommandRegistry::new();
        let ctx = git_context();
        registry
            .register(
                descriptor(&git_id(), "git.status", "Status", "git status", &[], &[]),
                Arc::new(HelloCommand),
            )
            .await
            .unwrap();
        let out = registry
            .execute(&ctx, "git.status", &["now".to_owned()])
            .await
            .unwrap();
        assert_eq!(out.text, "hello now");
    }

    #[tokio::test]
    async fn core_commands_are_reserved() {
        let registry = CommandRegistry::new();
        registry
            .register_core("list", "List", "list cubes", Arc::new(HelloCommand))
            .unwrap();
        let conflict = registry
            .register(
                descriptor(&git_id(), "list", "List", "shadow", &[], &[]),
                Arc::new(HelloCommand),
            )
            .await;
        assert!(
            conflict.is_err(),
            "plugins must not overwrite core commands"
        );
    }

    #[tokio::test]
    async fn namespace_owned_exclusively() {
        let registry = CommandRegistry::new();
        registry
            .register(
                descriptor(&git_id(), "git.status", "Status", "s", &[], &[]),
                Arc::new(HelloCommand),
            )
            .await
            .unwrap();
        let conflict = registry
            .register(
                descriptor(&rust_id(), "git.clone", "Clone", "c", &[], &[]),
                Arc::new(HelloCommand),
            )
            .await
            .unwrap_err();
        assert!(conflict.message.contains("already owned"));
    }

    #[tokio::test]
    async fn reserved_namespace_rejected() {
        let registry = CommandRegistry::new();
        let err = registry
            .register(
                descriptor(&git_id(), "runtime.stop", "Stop", "s", &[], &[]),
                Arc::new(HelloCommand),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("reserved"));
    }

    #[tokio::test]
    async fn plugin_command_requires_namespace() {
        let registry = CommandRegistry::new();
        let err = registry
            .register(
                descriptor(&git_id(), "status", "Status", "s", &[], &[]),
                Arc::new(HelloCommand),
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("must be namespaced"));
    }

    #[tokio::test]
    async fn unregister_removes_namespace() {
        let registry = CommandRegistry::new();
        registry
            .register(
                descriptor(&git_id(), "git.status", "Status", "s", &[], &[]),
                Arc::new(HelloCommand),
            )
            .await
            .unwrap();
        assert_eq!(registry.list_owned(&git_id()).len(), 1);
        registry.unregister_plugin(&git_id());
        assert_eq!(registry.list_owned(&git_id()).len(), 0);
        assert!(registry.complete("git.").is_empty());
    }

    #[tokio::test]
    async fn execution_permissions_enforced() {
        let registry = CommandRegistry::new();
        let ctx = PluginContext::new(
            rust_id(),
            crate::context::PluginScope::Server,
            GrantedPermissions::default(),
        );
        registry
            .register(
                descriptor(
                    &git_id(),
                    "git.commit",
                    "Commit",
                    "c",
                    &[],
                    &[PluginPermission::CubeWrite],
                ),
                Arc::new(HelloCommand),
            )
            .await
            .unwrap();
        let err = registry.execute(&ctx, "git.commit", &[]).await.unwrap_err();
        assert!(err.message.contains("cube.write"));
    }

    #[tokio::test]
    async fn completion_delegate() {
        let registry = CommandRegistry::new();
        registry
            .register(
                descriptor(&git_id(), "git.checkout", "Checkout", "c", &[], &[]),
                Arc::new(HelloCommand),
            )
            .await
            .unwrap();
        assert!(registry
            .complete("git.ch")
            .contains(&String::from("git.checkout")));
    }
}
