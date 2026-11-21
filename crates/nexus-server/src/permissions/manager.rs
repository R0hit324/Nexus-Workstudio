use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use nexus_core::cube::CubeDescriptor;
use nexus_core::error::Error;
use nexus_core::ids::{CubeId, UserId};
use nexus_core::permission::Permission;

use super::policy::{action_label, is_allowed, CubeAction, PermissionDenied};

/// Authoritative permission checks against Cube membership.
///
/// The manager caches the resolved role per (Cube, user); the owner always
/// trumps the stored role. Role re-evaluation happens whenever the cube
/// descriptor changes (join/leave) via [`PermissionManager::refresh`].
pub struct PermissionManager {
    roles: Mutex<HashMap<CubeId, HashMap<UserId, Permission>>>,
}

impl Default for PermissionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PermissionManager {
    pub fn new() -> Self {
        Self {
            roles: Mutex::new(HashMap::new()),
        }
    }

    /// Recomputes roles for a cube and caches them. Call whenever the
    /// membership set changes (join/leave).
    pub fn refresh(&self, cube: &CubeDescriptor) {
        let mut roles = self.roles.lock().unwrap_or_else(|p| p.into_inner());
        let mut mapping = HashMap::new();
        mapping.insert(cube.owner, Permission::Owner);
        for member in &cube.members {
            let role = if *member == cube.owner {
                Permission::Owner
            } else {
                Permission::Editor
            };
            mapping.insert(*member, role);
        }
        roles.insert(cube.id, mapping);
    }

    /// Drops all cached roles for a cube (used when a cube is destroyed).
    pub fn forget(&self, cube_id: CubeId) {
        self.roles
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&cube_id);
    }

    /// The role `user` holds in `cube`.
    ///
    /// Non-members resolve to [`Permission::Viewer`] (allowed, but every
    /// guarded cube operation is additionally gated on membership).
    pub fn role_for(&self, cube: &CubeDescriptor, user: UserId) -> Permission {
        self.roles
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&cube.id)
            .and_then(|roles| roles.get(&user).copied())
            .unwrap_or_else(|| {
                if cube.owner == user {
                    Permission::Owner
                } else if cube.contains_member(user) {
                    Permission::Editor
                } else {
                    Permission::Viewer
                }
            })
    }

    /// Returns `Ok(())` when `user` may perform `action` in `cube`, or an
    /// authorization error with a stable code otherwise.
    pub fn authorize(
        &self,
        cube: &CubeDescriptor,
        user: UserId,
        action: CubeAction,
    ) -> nexus_core::Result<()> {
        let role = self.role_for(cube, user);
        // Membership gate: even Viewer (the "can see it" role) must be an
        // actual member for cube-scoped operations.
        if !cube.contains_member(user) {
            return Err(Error::authorization("not a member of this cube")
                .with_internal(format!("user={user} role={role:?}")));
        }
        if is_allowed(role, action) {
            Ok(())
        } else {
            let denied = PermissionDenied::new(action, role);
            Err(Error::new(
                denied.to_error_code(),
                format!(
                    "permission denied: {} requires {:?}",
                    action_label(action),
                    role
                ),
            )
            .with_internal(format!("user={user} cube={} denied={denied:?}", cube.id)))
        }
    }

    /// Gets the permission for a user in a cube by cube_id.
    /// Requires a cube manager to look up the cube descriptor.
    pub async fn get_user_permission(
        &self,
        cube_manager: &Arc<dyn nexus_core::cube::CubeManager>,
        cube_id: CubeId,
        user_id: UserId,
    ) -> Permission {
        if let Ok(Some(cube)) = cube_manager.get_cube(cube_id).await {
            self.role_for(&cube, user_id)
        } else {
            Permission::Viewer
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::cube::CubeDescriptor;
    use nexus_core::error::ErrorCode;
    use nexus_core::ids::UserId;

    fn setup() -> (PermissionManager, CubeDescriptor, UserId, UserId) {
        let manager = PermissionManager::new();
        let owner = UserId::new();
        let editor = UserId::new();
        let cube = CubeDescriptor::new(CubeId::new(), "demo", owner);
        let updated = CubeDescriptor {
            members: vec![owner, editor],
            ..cube
        };
        manager.refresh(&updated);
        (manager, updated, owner, editor)
    }

    #[test]
    fn owner_has_elevated_rights() {
        let (manager, cube, owner, _) = setup();
        assert!(manager
            .authorize(&cube, owner, CubeAction::ManageMembers)
            .is_ok());
    }

    #[test]
    fn editor_has_full_workspace_access() {
        let (manager, cube, _, editor) = setup();
        for action in [
            CubeAction::ViewCube,
            CubeAction::ListWorkspace,
            CubeAction::ReadFile,
            CubeAction::CreateFile,
            CubeAction::WriteFile,
            CubeAction::DeleteFile,
            CubeAction::RenameFile,
        ] {
            assert!(
                manager.authorize(&cube, editor, action).is_ok(),
                "{action:?}"
            );
        }
        let err = manager
            .authorize(&cube, editor, CubeAction::ManageMembers)
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::Authorization);
    }

    #[test]
    fn non_member_is_denied_even_for_reading() {
        let (manager, cube, _, _) = setup();
        let stranger = UserId::new();
        let err = manager
            .authorize(&cube, stranger, CubeAction::ReadFile)
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::Authorization);
    }

    #[test]
    fn refresh_updates_roles() {
        let (manager, cube, owner, _) = setup();
        let stranger = UserId::new();
        let joined = CubeDescriptor {
            members: vec![owner, stranger],
            ..cube
        };
        manager.refresh(&joined);
        assert!(manager
            .authorize(&joined, stranger, CubeAction::WriteFile)
            .is_ok());
    }
}
