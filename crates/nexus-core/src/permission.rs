use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// The role a user holds within a Cube.
///
/// Roles are hierarchical: an Owner trivially holds every permission an Editor
/// or Viewer holds, and an Editor trivially holds every Viewer permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Permission {
    /// Full control of the Cube: manages users, permissions, runtime, builds,
    /// and the workspace.
    Owner,
    /// Can modify the workspace, run authorized terminals, and build/run code.
    Editor,
    /// Read-only: can view files, collaboration state, and build output.
    Viewer,
}

impl PartialOrd for Permission {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Permission {
    /// Orders roles by hierarchy rather than declaration order
    /// (`Owner` > `Editor` > `Viewer`).
    fn cmp(&self, other: &Self) -> Ordering {
        self.rank().cmp(&other.rank())
    }
}

impl Permission {
    /// Rank used to compare roles (`Owner` > `Editor` > `Viewer`).
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Permission::Owner => 2,
            Permission::Editor => 1,
            Permission::Viewer => 0,
        }
    }

    /// Returns `true` when `self` is at least the given role.
    #[must_use]
    pub fn at_least(self, other: Permission) -> bool {
        self.rank() >= other.rank()
    }

    /// Whether the role may manage the Cube (Owner only).
    #[must_use]
    pub const fn can_manage_cube(self) -> bool {
        matches!(self, Permission::Owner)
    }

    /// Whether the role may manage users and change permissions (Owner only).
    #[must_use]
    pub const fn can_manage_users(self) -> bool {
        matches!(self, Permission::Owner)
    }

    /// Whether the role may change permissions (Owner only).
    #[must_use]
    pub const fn can_change_permissions(self) -> bool {
        matches!(self, Permission::Owner)
    }

    /// Whether the role may modify workspace content (Owner or Editor).
    #[must_use]
    pub const fn can_modify_workspace(self) -> bool {
        matches!(self, Self::Owner | Self::Editor)
    }

    /// Whether the role may create files (Owner or Editor).
    #[must_use]
    pub const fn can_create_files(self) -> bool {
        matches!(self, Self::Owner | Self::Editor)
    }

    /// Whether the role may delete files (Owner or Editor).
    #[must_use]
    pub const fn can_delete_files(self) -> bool {
        matches!(self, Self::Owner | Self::Editor)
    }

    /// Whether the role may use authorized terminals (Owner or Editor).
    #[must_use]
    pub const fn can_use_terminal(self) -> bool {
        matches!(self, Self::Owner | Self::Editor)
    }

    /// Whether the role may run builds/tests (Owner or Editor).
    #[must_use]
    pub const fn can_build(self) -> bool {
        matches!(self, Self::Owner | Self::Editor)
    }

    /// Whether the role may participate in collaboration (chat, presence).
    #[must_use]
    pub const fn can_participate_in_collaboration(self) -> bool {
        matches!(self, Self::Owner | Self::Editor)
    }

    /// Whether the role may view workspace content (any role).
    #[must_use]
    pub const fn can_view_workspace(self) -> bool {
        true
    }

    /// Whether the role may manage the runtime (Owner only).
    #[must_use]
    pub const fn can_manage_runtime(self) -> bool {
        matches!(self, Permission::Owner)
    }
}

#[cfg(test)]
mod tests {
    use super::Permission::*;

    #[test]
    fn hierarchy_is_ordered() {
        assert!(Owner > Editor);
        assert!(Editor > Viewer);
        assert!(Owner.at_least(Viewer));
        assert!(Editor.at_least(Editor));
        assert!(!Viewer.at_least(Editor));
    }

    #[test]
    fn viewer_is_read_only() {
        assert!(Viewer.can_view_workspace());
        assert!(!Viewer.can_modify_workspace());
        assert!(!Viewer.can_use_terminal());
        assert!(!Viewer.can_build());
        assert!(!Viewer.can_manage_cube());
    }

    #[test]
    fn editor_can_modify_but_not_manage() {
        assert!(Editor.can_modify_workspace());
        assert!(Editor.can_build());
        assert!(Editor.can_create_files());
        assert!(!Editor.can_manage_cube());
        assert!(!Editor.can_change_permissions());
    }

    #[test]
    fn owner_can_do_everything() {
        assert!(Owner.can_manage_cube());
        assert!(Owner.can_manage_users());
        assert!(Owner.can_change_permissions());
        assert!(Owner.can_manage_runtime());
        assert!(Owner.can_modify_workspace());
    }
}
