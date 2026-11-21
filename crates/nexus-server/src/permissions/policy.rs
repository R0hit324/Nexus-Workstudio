use nexus_core::error::ErrorCode;
use nexus_core::permission::Permission;

/// Every guarded operation a client can attempt against a Cube.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CubeAction {
    ViewCube,
    JoinCube,
    LeaveCube,
    ListWorkspace,
    ReadFile,
    CreateFile,
    WriteFile,
    DeleteFile,
    RenameFile,
    ManageMembers,
    RunBuild,
    CreateTerminal,
    SendTerminalInput,
}

/// The Part 3 policy matrix, layered on `nexus_core::permission::Permission`.
///
/// | Action         | Owner | Editor | Viewer |
/// |----------------|:-----:|:------:|:------:|
/// | ViewCube       | ✓     | ✓      | ✓      |
/// | JoinCube       | ✓     | ✓      | ✓      |
/// | LeaveCube      | ✓     | ✓      | ✓      |
/// | ListWorkspace  | ✓     | ✓      | ✓      |
/// | ReadFile       | ✓     | ✓      | ✓      |
/// | CreateFile     | ✓     | ✓      | ✗      |
/// | WriteFile      | ✓     | ✓      | ✗      |
/// | DeleteFile     | ✓     | ✓      | ✗      |
/// | RenameFile     | ✓     | ✓      | ✗      |
/// | ManageMembers  | ✓     | ✗      | ✗      |
#[must_use]
pub const fn is_allowed(permission: Permission, action: CubeAction) -> bool {
    use CubeAction::*;
    match action {
        ViewCube | JoinCube | LeaveCube | ListWorkspace | ReadFile => {
            permission.can_view_workspace()
        }
        CreateFile => permission.can_create_files(),
        WriteFile | RenameFile => permission.can_modify_workspace(),
        DeleteFile => permission.can_delete_files(),
        ManageMembers => permission.can_manage_cube(),
        // Build + terminal operations mutate the Cube's environment, so they
        // carry the same bar as workspace edits.
        RunBuild | CreateTerminal | SendTerminalInput => permission.can_modify_workspace(),
    }
}

/// Unambiguous, stable label for the action (diagnostics and error messages).
#[must_use]
pub const fn action_label(action: CubeAction) -> &'static str {
    use CubeAction::*;
    match action {
        ViewCube => "view cube",
        JoinCube => "join cube",
        LeaveCube => "leave cube",
        ListWorkspace => "list workspace",
        ReadFile => "read file",
        CreateFile => "create file",
        WriteFile => "write file",
        DeleteFile => "delete file",
        RenameFile => "rename file",
        ManageMembers => "manage members",
        RunBuild => "run build",
        CreateTerminal => "create terminal",
        SendTerminalInput => "send terminal input",
    }
}

/// Describes a denial; used for diagnostics and authorization errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionDenied {
    pub action: CubeAction,
    pub role: Permission,
}

impl PermissionDenied {
    #[must_use]
    pub const fn new(action: CubeAction, role: Permission) -> Self {
        Self { action, role }
    }

    #[must_use]
    pub const fn to_error_code(self) -> ErrorCode {
        ErrorCode::Authorization
    }
}

#[cfg(test)]
mod tests {
    use super::CubeAction::*;
    use super::*;

    #[test]
    fn full_matrix() {
        for action in [ViewCube, JoinCube, LeaveCube, ListWorkspace, ReadFile] {
            assert!(is_allowed(Permission::Owner, action));
            assert!(is_allowed(Permission::Editor, action));
            assert!(is_allowed(Permission::Viewer, action), "{action:?}");
        }
        for action in [CreateFile, WriteFile, DeleteFile, RenameFile] {
            assert!(is_allowed(Permission::Owner, action));
            assert!(is_allowed(Permission::Editor, action));
            assert!(!is_allowed(Permission::Viewer, action), "{action:?}");
        }
        assert!(is_allowed(Permission::Owner, ManageMembers));
        assert!(!is_allowed(Permission::Editor, ManageMembers));
        assert!(!is_allowed(Permission::Viewer, ManageMembers));
    }
}
