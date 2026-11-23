//! Plugin permissions with least-privilege helpers.
//!
//! Plugins declare the permissions they need; the platform grants at most
//! what is declared, and *sensitive* permissions additionally require
//! explicit administrator/user approval before activation.

/// A capability granted to a plugin after validation and approval.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum PluginPermission {
    /// Read files inside the granted workspace scope.
    FilesystemRead,
    /// Write files inside the granted workspace scope.
    FilesystemWrite,
    /// Outbound network access.
    Network,
    /// Spawn processes through the platform process manager.
    ProcessExecute,
    /// Control cube runtimes (start/stop/restart).
    RuntimeControl,
    /// Read cube metadata and workspace listings.
    CubeRead,
    /// Mutate cube contents.
    CubeWrite,
    /// Read terminal output.
    TerminalRead,
    /// Write terminal input.
    TerminalWrite,
    /// Read chat/presence.
    ChatRead,
    /// Send chat/presence.
    ChatWrite,
    /// Register UI panels, commands, keybindings, themes.
    UiRegister,
    /// Subscribe to platform events.
    EventsSubscribe,
    /// Publish plugin events (validated and rate limited).
    EventsPublish,
}

/// Permission categories for approval policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionGroup {
    Filesystem,
    Network,
    Process,
    Runtime,
    Cube,
    Terminal,
    Chat,
    Ui,
    Events,
}

impl PluginPermission {
    /// The canonical dotted token used in manifests and logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FilesystemRead => "filesystem.read",
            Self::FilesystemWrite => "filesystem.write",
            Self::Network => "network",
            Self::ProcessExecute => "process.execute",
            Self::RuntimeControl => "runtime.control",
            Self::CubeRead => "cube.read",
            Self::CubeWrite => "cube.write",
            Self::TerminalRead => "terminal.read",
            Self::TerminalWrite => "terminal.write",
            Self::ChatRead => "chat.read",
            Self::ChatWrite => "chat.write",
            Self::UiRegister => "ui.register",
            Self::EventsSubscribe => "events.subscribe",
            Self::EventsPublish => "events.publish",
        }
    }

    /// Parses a dotted token.
    ///
    /// # Errors
    /// Returns `None`-like error text through the caller; this returns
    /// `Option` so manifests can reject unknown tokens with context.
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        let all = [
            (Self::FilesystemRead, "filesystem.read"),
            (Self::FilesystemWrite, "filesystem.write"),
            (Self::Network, "network"),
            (Self::ProcessExecute, "process.execute"),
            (Self::RuntimeControl, "runtime.control"),
            (Self::CubeRead, "cube.read"),
            (Self::CubeWrite, "cube.write"),
            (Self::TerminalRead, "terminal.read"),
            (Self::TerminalWrite, "terminal.write"),
            (Self::ChatRead, "chat.read"),
            (Self::ChatWrite, "chat.write"),
            (Self::UiRegister, "ui.register"),
            (Self::EventsSubscribe, "events.subscribe"),
            (Self::EventsPublish, "events.publish"),
        ];
        all.iter()
            .find(|(_, text)| *text == token)
            .map(|(permission, _)| *permission)
    }

    /// The broad group this permission belongs to.
    #[must_use]
    pub const fn group(self) -> PermissionGroup {
        match self {
            Self::FilesystemRead | Self::FilesystemWrite => PermissionGroup::Filesystem,
            Self::Network => PermissionGroup::Network,
            Self::ProcessExecute => PermissionGroup::Process,
            Self::RuntimeControl => PermissionGroup::Runtime,
            Self::CubeRead | Self::CubeWrite => PermissionGroup::Cube,
            Self::TerminalRead | Self::TerminalWrite => PermissionGroup::Terminal,
            Self::ChatRead | Self::ChatWrite => PermissionGroup::Chat,
            Self::UiRegister => PermissionGroup::Ui,
            Self::EventsSubscribe | Self::EventsPublish => PermissionGroup::Events,
        }
    }

    /// Sensitive permissions require explicit administrator/user approval
    /// before a plugin is enabled, even when the manifest declares them.
    #[must_use]
    pub const fn requires_approval(self) -> bool {
        matches!(
            self,
            Self::FilesystemWrite | Self::Network | Self::ProcessExecute | Self::RuntimeControl
        )
    }
}

/// The granted set for one plugin: always a subset of what the manifest
/// declared (least privilege), never a superset.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GrantedPermissions {
    granted: std::collections::BTreeSet<PluginPermission>,
}

impl GrantedPermissions {
    /// Grants only the requested permissions; every value outside the
    /// manifest's request is dropped.
    #[must_use]
    pub fn grant(
        requested: &[PluginPermission],
        approved: impl IntoIterator<Item = PluginPermission>,
    ) -> Self {
        let mut granted: std::collections::BTreeSet<PluginPermission> =
            std::collections::BTreeSet::new();
        let ok: std::collections::BTreeSet<PluginPermission> = approved.into_iter().collect();
        for permission in requested {
            if ok.contains(permission) {
                granted.insert(*permission);
            }
        }
        Self { granted }
    }

    /// Creates a grant from an explicit set without dual filtering; callers
    /// must have already enforced the manifest subset themselves.
    #[must_use]
    pub fn from_set(granted: impl IntoIterator<Item = PluginPermission>) -> Self {
        Self {
            granted: granted.into_iter().collect(),
        }
    }

    /// Whether a permission is granted.
    #[must_use]
    pub fn allows(&self, permission: PluginPermission) -> bool {
        self.granted.contains(&permission)
    }

    /// Iterates granted permissions.
    pub fn iter(&self) -> impl Iterator<Item = PluginPermission> + '_ {
        self.granted.iter().copied()
    }

    /// Number of granted permissions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.granted.len()
    }

    /// Whether nothing is granted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.granted.is_empty()
    }

    /// The approved-requirements list: declared permissions still missing
    /// admin/user approval. Returns the requested subset that requires
    /// approval but is not present in `granted`.
    #[must_use]
    pub fn missing_approvals(
        requested: &[PluginPermission],
        granted: &Self,
    ) -> Vec<PluginPermission> {
        requested
            .iter()
            .copied()
            .filter(|p| p.requires_approval() && !granted.allows(*p))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_round_trip() {
        for permission in [
            PluginPermission::FilesystemRead,
            PluginPermission::Network,
            PluginPermission::ProcessExecute,
            PluginPermission::UiRegister,
            PluginPermission::EventsPublish,
        ] {
            assert_eq!(
                PluginPermission::parse(permission.as_str()),
                Some(permission)
            );
        }
        assert_eq!(PluginPermission::parse("nope"), None);
    }

    #[test]
    fn sensitive_permissions_require_approval() {
        assert!(PluginPermission::Network.requires_approval());
        assert!(PluginPermission::ProcessExecute.requires_approval());
        assert!(PluginPermission::RuntimeControl.requires_approval());
        assert!(PluginPermission::FilesystemWrite.requires_approval());
        assert!(!PluginPermission::UiRegister.requires_approval());
        assert!(!PluginPermission::CubeRead.requires_approval());
    }

    #[test]
    fn grants_are_least_privilege() {
        let requested = vec![PluginPermission::FilesystemRead, PluginPermission::Network];
        let granted = GrantedPermissions::grant(&requested, [PluginPermission::FilesystemRead]);
        assert!(granted.allows(PluginPermission::FilesystemRead));
        assert!(
            !granted.allows(PluginPermission::Network),
            "unapproved permission must not be granted"
        );
        let missing = GrantedPermissions::missing_approvals(&requested, &granted);
        assert_eq!(missing, vec![PluginPermission::Network]);
    }

    #[test]
    fn granting_unknown_permissions_is_impossible() {
        let granted = GrantedPermissions::grant(&[], [PluginPermission::Network]);
        assert!(granted.is_empty());
    }
}
