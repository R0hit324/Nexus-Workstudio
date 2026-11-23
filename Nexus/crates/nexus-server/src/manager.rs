use std::collections::HashMap;
use std::sync::Mutex;

use nexus_core::cube::{
    CreateCubeRequest, CubeDescriptor, CubeManager, CubeStatus, JoinCubeRequest,
};
use nexus_core::error::{Error, ErrorCode, Result};
use nexus_core::ids::{CubeId, WorkspaceId};
use tracing::info;

/// An in-memory implementation of [`CubeManager`].
///
/// Part 1 implements lifecycle state and membership only; runtime orchestration
/// arrives in a later part. All mutation is serialized behind a mutex, which is
/// acceptable at this scale; partitioning by Cube happens in a later part.
pub struct InMemoryCubeManager {
    cubes: Mutex<HashMap<CubeId, CubeState>>,
    max_cubes: u32,
}

struct CubeState {
    descriptor: CubeDescriptor,
}

impl InMemoryCubeManager {
    /// Creates an empty manager with the given Cube limit.
    #[must_use]
    pub fn new(max_cubes: u32) -> Self {
        Self {
            cubes: Mutex::new(HashMap::new()),
            max_cubes,
        }
    }
}

#[async_trait::async_trait]
impl CubeManager for InMemoryCubeManager {
    async fn create_cube(&self, request: CreateCubeRequest) -> Result<CubeDescriptor> {
        let mut cubes = self
            .cubes
            .lock()
            .map_err(|_| Error::internal("cube store poisoned"))?;

        if cubes.len() >= self.max_cubes as usize {
            return Err(Error::new(
                ErrorCode::AlreadyExists,
                "maximum Cube count reached",
            ));
        }

        let mut cube = CubeDescriptor::new(CubeId::new(), request.name, request.owner);
        // Every Cube owns a dedicated workspace, sealed when the Cube is
        // created. Part 3 keys storage for that workspace under
        // `cubes/<cube_id>/workspace`.
        cube.workspace_id = Some(WorkspaceId::new());
        info!(cube_id = %cube.id, user_id = %cube.owner, "cube_created");
        cubes.insert(
            cube.id,
            CubeState {
                descriptor: cube.clone(),
            },
        );
        Ok(cube)
    }

    async fn start_cube(&self, cube_id: CubeId) -> Result<CubeDescriptor> {
        let mut cubes = self
            .cubes
            .lock()
            .map_err(|_| Error::internal("cube store poisoned"))?;
        let state = cubes
            .get_mut(&cube_id)
            .ok_or_else(|| Error::new(ErrorCode::NotFound, "Cube not found"))?;
        ensure_transition(&state.descriptor, CubeStatus::Running)?;
        state.descriptor.status = CubeStatus::Running;
        Ok(state.descriptor.clone())
    }

    async fn stop_cube(&self, cube_id: CubeId) -> Result<CubeDescriptor> {
        let mut cubes = self
            .cubes
            .lock()
            .map_err(|_| Error::internal("cube store poisoned"))?;
        let state = cubes
            .get_mut(&cube_id)
            .ok_or_else(|| Error::new(ErrorCode::NotFound, "Cube not found"))?;
        ensure_transition(&state.descriptor, CubeStatus::Stopped)?;
        state.descriptor.status = CubeStatus::Stopped;
        Ok(state.descriptor.clone())
    }

    async fn restart_cube(&self, cube_id: CubeId) -> Result<CubeDescriptor> {
        let mut cubes = self
            .cubes
            .lock()
            .map_err(|_| Error::internal("cube store poisoned"))?;
        let state = cubes
            .get_mut(&cube_id)
            .ok_or_else(|| Error::new(ErrorCode::NotFound, "Cube not found"))?;
        ensure_transition(&state.descriptor, CubeStatus::Running)?;
        state.descriptor.status = CubeStatus::Running;
        Ok(state.descriptor.clone())
    }

    async fn destroy_cube(&self, cube_id: CubeId) -> Result<()> {
        let mut cubes = self
            .cubes
            .lock()
            .map_err(|_| Error::internal("cube store poisoned"))?;
        let state = cubes
            .remove(&cube_id)
            .ok_or_else(|| Error::new(ErrorCode::NotFound, "Cube not found"))?;
        info!(cube_id = %cube_id, "cube_destroyed");
        // The descriptor is dropped with `state`; runtime teardown arrives later.
        drop(state);
        Ok(())
    }

    async fn get_cube(&self, cube_id: CubeId) -> Result<Option<CubeDescriptor>> {
        let cubes = self
            .cubes
            .lock()
            .map_err(|_| Error::internal("cube store poisoned"))?;
        Ok(cubes.get(&cube_id).map(|s| s.descriptor.clone()))
    }

    async fn list_cubes(&self) -> Result<Vec<CubeDescriptor>> {
        let cubes = self
            .cubes
            .lock()
            .map_err(|_| Error::internal("cube store poisoned"))?;
        let mut all: Vec<_> = cubes.values().map(|s| s.descriptor.clone()).collect();
        all.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(all)
    }

    async fn join_cube(&self, request: JoinCubeRequest) -> Result<CubeDescriptor> {
        let mut cubes = self
            .cubes
            .lock()
            .map_err(|_| Error::internal("cube store poisoned"))?;
        let state = cubes
            .get_mut(&request.cube_id)
            .ok_or_else(|| Error::new(ErrorCode::NotFound, "Cube not found"))?;

        if !state.descriptor.contains_member(request.user_id) {
            // New joiners are Editors by default; the owner retains Owner.
            state.descriptor.members.push(request.user_id);
            info!(cube_id = %request.cube_id, user_id = %request.user_id, "user_joined_cube");
        }

        Ok(state.descriptor.clone())
    }

    async fn leave_cube(&self, request: JoinCubeRequest) -> Result<()> {
        let mut cubes = self
            .cubes
            .lock()
            .map_err(|_| Error::internal("cube store poisoned"))?;
        let state = cubes
            .get_mut(&request.cube_id)
            .ok_or_else(|| Error::new(ErrorCode::NotFound, "Cube not found"))?;

        if state.descriptor.owner == request.user_id {
            return Err(Error::authorization(
                "the Cube owner cannot leave; destroy the Cube instead",
            ));
        }

        let before = state.descriptor.members.len();
        state.descriptor.members.retain(|m| *m != request.user_id);
        if state.descriptor.members.len() == before {
            return Err(Error::new(ErrorCode::NotFound, "user is not a member"));
        }
        info!(cube_id = %request.cube_id, user_id = %request.user_id, "user_left_cube");
        Ok(())
    }
}

/// Validates that the descriptor can move to the requested status, updating it
/// only when legal (`Error` and `Recovering` handling refines later).
fn ensure_transition(descriptor: &CubeDescriptor, to: CubeStatus) -> Result<()> {
    let from = descriptor.status;
    if from.can_transition_to(to) {
        return Ok(());
    }
    Err(Error::new(
        ErrorCode::InvalidRequest,
        format!("invalid Cube state transition: {from:?} -> {to:?}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::ids::UserId;

    #[tokio::test]
    async fn create_list_get_roundtrip() {
        let manager = InMemoryCubeManager::new(10);
        let owner = UserId::new();
        let cube = manager
            .create_cube(CreateCubeRequest {
                name: "demo".into(),
                owner,
            })
            .await
            .unwrap();
        assert_eq!(cube.owner, owner);
        assert_eq!(manager.list_cubes().await.unwrap().len(), 1);
        assert_eq!(manager.get_cube(cube.id).await.unwrap(), Some(cube.clone()));
        assert_eq!(cube.status, CubeStatus::Creating);
    }

    #[tokio::test]
    async fn enforces_max_cubes() {
        let manager = InMemoryCubeManager::new(2);
        let owner = UserId::new();
        manager
            .create_cube(CreateCubeRequest {
                name: "a".into(),
                owner,
            })
            .await
            .unwrap();
        manager
            .create_cube(CreateCubeRequest {
                name: "b".into(),
                owner,
            })
            .await
            .unwrap();
        let err = manager
            .create_cube(CreateCubeRequest {
                name: "c".into(),
                owner,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::AlreadyExists);
    }

    #[tokio::test]
    async fn join_and_leave_membership() {
        let manager = InMemoryCubeManager::new(10);
        let owner = UserId::new();
        let bob = UserId::new();
        let cube = manager
            .create_cube(CreateCubeRequest {
                name: "demo".into(),
                owner,
            })
            .await
            .unwrap();

        let joined = manager
            .join_cube(JoinCubeRequest {
                cube_id: cube.id,
                user_id: bob,
            })
            .await
            .unwrap();
        assert!(joined.contains_member(bob));
        assert_eq!(joined.members.len(), 2);

        // Joining again is idempotent.
        let joined = manager
            .join_cube(JoinCubeRequest {
                cube_id: cube.id,
                user_id: bob,
            })
            .await
            .unwrap();
        assert_eq!(joined.members.len(), 2);

        manager
            .leave_cube(JoinCubeRequest {
                cube_id: cube.id,
                user_id: bob,
            })
            .await
            .unwrap();
        let after = manager.get_cube(cube.id).await.unwrap().unwrap();
        assert!(!after.contains_member(bob));
    }

    #[tokio::test]
    async fn owner_cannot_leave_cube() {
        let manager = InMemoryCubeManager::new(10);
        let owner = UserId::new();
        let cube = manager
            .create_cube(CreateCubeRequest {
                name: "demo".into(),
                owner,
            })
            .await
            .unwrap();
        let err = manager
            .leave_cube(JoinCubeRequest {
                cube_id: cube.id,
                user_id: owner,
            })
            .await
            .unwrap_err();
        assert_eq!(err.code(), ErrorCode::Authorization);
    }

    #[tokio::test]
    async fn lifecycle_transitions_are_enforced() {
        let manager = InMemoryCubeManager::new(10);
        let owner = UserId::new();
        let cube = manager
            .create_cube(CreateCubeRequest {
                name: "demo".into(),
                owner,
            })
            .await
            .unwrap();

        // Creating -> Running is illegal; Creating -> Ready is legal.
        let err = manager.start_cube(cube.id).await.unwrap_err();
        assert_eq!(err.code(), ErrorCode::InvalidRequest);

        manager.get_cube(cube.id).await.unwrap().unwrap();
        manager.stop_cube(cube.id).await.unwrap();
        let stopped = manager.get_cube(cube.id).await.unwrap().unwrap();
        assert_eq!(stopped.status, CubeStatus::Stopped);

        manager.restart_cube(cube.id).await.unwrap();
        let running = manager.get_cube(cube.id).await.unwrap().unwrap();
        assert_eq!(running.status, CubeStatus::Running);

        manager.destroy_cube(cube.id).await.unwrap();
        assert_eq!(manager.get_cube(cube.id).await.unwrap(), None);
    }
}
