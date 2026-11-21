//! Build service: bridges the `nexus-build` manager to protocol messages.
//!
//! The service owns the [`BuildManager`], routes build events onto client
//! connections (fan-out to every connection inside the originating Cube), and
//! keeps the Cube↔build association needed for that routing.

use std::collections::HashMap;
use std::sync::Arc;

use nexus_build::BuildManager;
use nexus_core::ids::{BuildId, CubeId, UserId};
use nexus_core::{BuildConfiguration, BuildRequest, BuildTarget, Timestamp};
use tokio::sync::RwLock;

use crate::connection::manager::ConnectionManager;
use nexus_protocol::message::Message;

/// Server-facing facade over the build system.
pub struct BuildService {
    manager: Arc<BuildManager>,
    /// Maps each build to the Cube it belongs to, used by the event pump for
    /// fan-out routing.
    cubes: Arc<RwLock<HashMap<BuildId, CubeId>>>,
}

impl BuildService {
    /// Creates the service, wiring its event pump to `connection_manager`.
    ///
    /// The internal pump only lives as long as the connection manager Arc; it
    /// exits when the channel closes (i.e., the manager is dropped).
    #[must_use]
    pub fn new(
        config: nexus_build::BuildManagerConfig,
        connection_manager: Arc<ConnectionManager>,
    ) -> Self {
        let (manager, mut events) = BuildManager::new(config);
        let manager = Arc::new(manager);
        let cubes: Arc<RwLock<HashMap<BuildId, CubeId>>> = Arc::new(RwLock::new(HashMap::new()));

        // Event pump: build lifecycle events become protocol messages.
        let pump_cubes = cubes.clone();
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                let message = match event {
                    nexus_build::BuildEvent::Started { build_id, .. } => {
                        let cube_id = pump_cubes.read().await.get(&build_id).copied();
                        match cube_id {
                            Some(cube_id) => Message::BuildStarted { build_id, cube_id },
                            None => continue,
                        }
                    }
                    nexus_build::BuildEvent::Diagnostic {
                        build_id,
                        diagnostic,
                    } => Message::BuildDiagnostics {
                        build_id,
                        diagnostics: vec![diagnostic],
                    },
                    nexus_build::BuildEvent::Finished { build_id, result } => {
                        Message::BuildFinished { build_id, result }
                    }
                    nexus_build::BuildEvent::Queued { .. }
                    | nexus_build::BuildEvent::Cancelled { .. } => continue,
                };

                // Output/diagnostics routes through the build's Cube.
                let cube_id = pump_cubes
                    .read()
                    .await
                    .get(&message_build_id(&message))
                    .copied();
                if let Some(cube_id) = cube_id {
                    connection_manager.broadcast_to_cube(cube_id, message);
                }
            }
        });

        Self { manager, cubes }
    }

    /// Queues a build on behalf of a user; returns the assigned build id.
    pub async fn submit(
        &self,
        cube_id: CubeId,
        user_id: UserId,
        target: BuildTarget,
        configuration: BuildConfiguration,
    ) -> anyhow::Result<BuildId> {
        let build_id = nexus_core::ids::BuildId::new();
        let request = BuildRequest {
            build_id,
            cube_id,
            user_id,
            target,
            configuration,
            queued_at: Timestamp::now(),
        };
        self.cubes.write().await.insert(build_id, cube_id);
        self.manager.submit_build(request).await?;
        Ok(build_id)
    }

    /// Cancels a build (queued ones are skipped by the manager today).
    pub async fn cancel(&self, build_id: BuildId) -> anyhow::Result<()> {
        self.manager
            .cancel_build(build_id, "client_requested".to_string())
            .await
    }

    /// Number of queued builds (for diagnostics + queue position reporting).
    pub async fn queued_count(&self) -> usize {
        self.manager.queued_count().await
    }
}

/// Extracts the build id from a build-related protocol message.
fn message_build_id(message: &Message) -> BuildId {
    match message {
        Message::BuildStarted { build_id, .. }
        | Message::BuildDiagnostics { build_id, .. }
        | Message::BuildFinished { build_id, .. } => *build_id,
        _ => BuildId::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn submit_routes_to_manager() {
        let service = BuildService::new(
            nexus_build::BuildManagerConfig::default(),
            Arc::new(ConnectionManager::new()),
        );
        let build_id = service
            .submit(
                CubeId::new(),
                UserId::new(),
                nexus_core::BuildTarget {
                    target_type: nexus_core::BuildTargetType::Build,
                    profile: nexus_core::BuildProfile::Debug,
                    target_name: None,
                    working_directory: String::new(),
                    environment: vec![],
                    timeout_seconds: 60,
                    resource_limits: nexus_core::ResourceLimits::default(),
                },
                nexus_core::BuildConfiguration {
                    language: "unknown".into(),
                    toolchain: None,
                    build_command: None,
                    test_command: None,
                    check_command: None,
                    lint_command: None,
                    format_command: None,
                    environment: vec![],
                    working_directory: String::new(),
                    timeout_seconds: 60,
                    resource_limits: nexus_core::ResourceLimits::default(),
                },
            )
            .await
            .unwrap();
        assert_eq!(service.queued_count().await, 1);
        let _ = build_id;
    }
}
