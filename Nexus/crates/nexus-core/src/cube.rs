use serde::{Deserialize, Serialize};

use crate::ids::{CubeId, UserId, WorkspaceId};
use crate::timestamp::Timestamp;
use crate::version::Version;

/// Lifecycle state of a Cube. Actual runtimes are orchestrated in a later
/// part; Part 1 defines the state machine and transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CubeStatus {
    Creating,
    Ready,
    Running,
    Stopping,
    Stopped,
    Recovering,
    Error,
    Destroyed,
}

impl CubeStatus {
    /// Returns `true` when a transition from `self` to `to` is permitted.
    #[must_use]
    pub fn can_transition_to(self, to: CubeStatus) -> bool {
        use CubeStatus::*;
        matches!(
            (self, to),
            (Creating, Ready)
                | (Creating, Stopped)
                | (Ready, Running)
                | (Ready, Stopping)
                | (Ready, Recovering)
                | (Ready, Destroyed)
                | (Running, Stopping)
                | (Running, Recovering)
                | (Stopping, Stopped)
                | (Stopping, Error)
                | (Stopped, Ready)
                | (Stopped, Running)
                | (Stopped, Destroyed)
                | (Recovering, Ready)
                | (Recovering, Running)
                | (Recovering, Error)
                | (Error, Ready)
                | (Error, Recovering)
                | (Error, Destroyed)
                | (Destroyed, Destroyed)
        )
    }

    /// Returns `true` when the Cube is usable by members.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Ready | Self::Running | Self::Recovering)
    }
}

/// Read-only description of a Cube as visible to clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CubeDescriptor {
    pub id: CubeId,
    pub name: String,
    pub owner: UserId,
    pub status: CubeStatus,
    pub members: Vec<UserId>,
    /// The workspace backing this Cube. Populated once the Cube is created.
    pub workspace_id: Option<WorkspaceId>,
    pub created_at: Timestamp,
    pub version: Version,
}

impl CubeDescriptor {
    #[must_use]
    pub fn new(id: CubeId, name: impl Into<String>, owner: UserId) -> Self {
        Self {
            id,
            name: name.into(),
            owner,
            status: CubeStatus::Creating,
            members: vec![owner],
            workspace_id: None,
            created_at: Timestamp::now(),
            version: Version::new(1),
        }
    }

    /// Returns `true` when `user` is a member of the Cube.
    #[must_use]
    pub fn contains_member(&self, user: UserId) -> bool {
        self.members.contains(&user)
    }
}

/// Request to create a Cube.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCubeRequest {
    pub name: String,
    pub owner: UserId,
}

/// Request to join or leave a Cube.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JoinCubeRequest {
    pub cube_id: CubeId,
    pub user_id: UserId,
}

/// The domain service that owns Cube lifecycle and membership. The concrete
/// runtime implementation arrives in a later part; Part 1 only defines the
/// contract.
#[async_trait::async_trait]
pub trait CubeManager: Send + Sync {
    async fn create_cube(&self, request: CreateCubeRequest) -> crate::Result<CubeDescriptor>;
    async fn start_cube(&self, cube_id: CubeId) -> crate::Result<CubeDescriptor>;
    async fn stop_cube(&self, cube_id: CubeId) -> crate::Result<CubeDescriptor>;
    async fn restart_cube(&self, cube_id: CubeId) -> crate::Result<CubeDescriptor>;
    async fn destroy_cube(&self, cube_id: CubeId) -> crate::Result<()>;
    async fn get_cube(&self, cube_id: CubeId) -> crate::Result<Option<CubeDescriptor>>;
    async fn list_cubes(&self) -> crate::Result<Vec<CubeDescriptor>>;
    async fn join_cube(&self, request: JoinCubeRequest) -> crate::Result<CubeDescriptor>;
    async fn leave_cube(&self, request: JoinCubeRequest) -> crate::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::CubeStatus::*;
    use super::*;

    #[test]
    fn valid_transitions_are_allowed() {
        assert!(Creating.can_transition_to(Ready));
        assert!(Ready.can_transition_to(Running));
        assert!(Running.can_transition_to(Stopping));
        assert!(Stopping.can_transition_to(Stopped));
        assert!(Stopped.can_transition_to(Running));
        assert!(Recovering.can_transition_to(Ready));
        assert!(Error.can_transition_to(Ready));
    }

    #[test]
    fn invalid_transitions_are_rejected() {
        assert!(!Creating.can_transition_to(Destroyed));
        assert!(!Ready.can_transition_to(Error));
        assert!(!Stopped.can_transition_to(Stopping));
        assert!(!Running.can_transition_to(Ready));
    }

    #[test]
    fn status_activity() {
        assert!(Ready.is_active());
        assert!(Running.is_active());
        assert!(!Stopped.is_active());
        assert!(!Destroyed.is_active());
    }

    #[test]
    fn descriptor_starts_with_creating_and_owner_member() {
        let owner = UserId::new();
        let cube = CubeDescriptor::new(CubeId::new(), "demo", owner);
        assert_eq!(cube.status, Creating);
        assert!(cube.contains_member(owner));
        assert_eq!(cube.members.len(), 1);
    }

    #[test]
    fn serializes_through_json() {
        let cube = CubeDescriptor::new(CubeId::new(), "demo", UserId::new());
        let encoded = serde_json::to_string(&cube).unwrap();
        let decoded: CubeDescriptor = serde_json::from_str(&encoded).unwrap();
        assert_eq!(cube, decoded);
    }
}
