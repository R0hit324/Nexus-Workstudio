//! AI permissions (`§38`, `§39`).
//!
//! Permissions are checked *server-side* before any tool runs (`§32`): the
//! TUI can warn, but a malicious or modified client must not be able to
//! bypass the gate. AI can never escalate beyond the set it was granted.

use std::collections::BTreeSet;

/// One grantable AI capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AiPermission {
    /// Read files from the Cube workspace.
    ReadWorkspace,
    /// List/search the workspace tree.
    EnumerateWorkspace,
    /// Inspect diagnostics, builds, tests, terminal output, runtime info.
    ReadTelemetry,
    /// Propose edits — still requires approval before application (`§30`).
    ProposeEdit,
    /// Create new files.
    CreateFiles,
    /// Rename files.
    RenameFiles,
    /// Delete files. Stronger approval than reading (`§30`).
    DeleteFiles,
    /// Execute commands inside the Cube runtime (`§31`).
    RunCommands,
    /// Outbound network for provider traffic only (`§67`).
    NetworkEgress,
}

impl AiPermission {
    /// Whether exercising this permission may destroy data (`§32`).
    #[must_use]
    pub const fn is_destructive(&self) -> bool {
        matches!(self, Self::DeleteFiles | Self::RunCommands)
    }

    /// Destructive permissions demand a human approval step on top of the
    /// plain grant (`§39` approval system).
    #[must_use]
    pub const fn requires_approval(&self) -> bool {
        self.is_destructive()
    }
}

/// The permission set granted to one AI session.
#[derive(Debug, Clone, Default)]
pub struct AiPermissionSet {
    granted: BTreeSet<AiPermission>,
}

impl AiPermissionSet {
    #[must_use]
    pub fn new(granted: impl IntoIterator<Item = AiPermission>) -> Self {
        Self {
            granted: granted.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn allows(&self, permission: AiPermission) -> bool {
        self.granted.contains(&permission)
    }

    /// For a tool requiring `required`: checks the grant AND flags the
    /// approval requirement. Returns `Ok(())` when the call may proceed
    /// with approval already satisfied.
    pub fn authorize(&self, required: AiPermission) -> Result<(), AiPermissionFailure> {
        if !self.allows(required) {
            return Err(AiPermissionFailure::NotGranted(required));
        }
        if required.requires_approval() {
            return Err(AiPermissionFailure::NeedsApproval(required));
        }
        Ok(())
    }
}

/// Outcome of a tool permission check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiPermissionFailure {
    /// The session lacks the grant entirely.
    NotGranted(AiPermission),
    /// Granted, but the operation is destructive and no approval was given.
    NeedsApproval(AiPermission),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructive_kinds_are_classified() {
        assert!(AiPermission::DeleteFiles.is_destructive());
        assert!(AiPermission::RunCommands.is_destructive());
        for gentle in [
            AiPermission::ReadWorkspace,
            AiPermission::EnumerateWorkspace,
            AiPermission::ReadTelemetry,
            AiPermission::ProposeEdit,
            AiPermission::CreateFiles,
            AiPermission::RenameFiles,
            AiPermission::NetworkEgress,
        ] {
            assert!(!gentle.is_destructive(), "{gentle:?}");
        }
    }

    #[test]
    fn authorization_demands_approval_for_destructive_operations() {
        let set = AiPermissionSet::new([AiPermission::ReadWorkspace, AiPermission::DeleteFiles]);
        assert_eq!(set.authorize(AiPermission::ReadWorkspace), Ok(()));
        assert_eq!(
            set.authorize(AiPermission::DeleteFiles),
            Err(AiPermissionFailure::NeedsApproval(
                AiPermission::DeleteFiles
            ))
        );
        assert_eq!(
            set.authorize(AiPermission::RunCommands),
            Err(AiPermissionFailure::NotGranted(AiPermission::RunCommands))
        );
    }
}
