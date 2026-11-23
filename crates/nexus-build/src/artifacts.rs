//! Build artifact tracking.
//!
//! Artifacts are reported by builds as lightweight descriptors (path, kind,
//! size). The manager indexes them per build and per cube; on-disk storage of
//! artifact payloads is delegated to whatever produced them (the runtime
//! backend), keeping this crate free of filesystem assumptions.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use nexus_core::{ArtifactId, BuildArtifact, BuildId, CubeId, Timestamp};
use tokio::sync::RwLock;

/// Artifact storage configuration.
#[derive(Debug, Clone)]
pub struct ArtifactManagerConfig {
    /// Root directory advertised for artifact download URLs.
    pub root: PathBuf,
    /// Maximum artifacts retained per build.
    pub max_per_build: usize,
    /// Maximum artifacts retained per cube.
    pub max_per_cube: usize,
}

impl Default for ArtifactManagerConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("/var/lib/nexus/artifacts"),
            max_per_build: 100,
            max_per_cube: 1000,
        }
    }
}

/// Indexes artifacts per build and per cube.
pub struct ArtifactManager {
    config: ArtifactManagerConfig,
    artifacts: Arc<RwLock<HashMap<ArtifactId, BuildArtifact>>>,
    build_index: Arc<RwLock<HashMap<BuildId, Vec<ArtifactId>>>>,
    cube_index: Arc<RwLock<HashMap<CubeId, Vec<ArtifactId>>>>,
}

impl ArtifactManager {
    /// Creates a new artifact manager.
    pub fn new(config: ArtifactManagerConfig) -> Self {
        Self {
            config,
            artifacts: Arc::new(RwLock::new(HashMap::new())),
            build_index: Arc::new(RwLock::new(HashMap::new())),
            cube_index: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// The configured artifact root.
    pub fn root(&self) -> &PathBuf {
        &self.config.root
    }

    /// Registers artifacts produced by a build.
    pub async fn store_artifacts(
        &self,
        build_id: BuildId,
        cube_id: CubeId,
        artifacts: &[BuildArtifact],
    ) -> usize {
        let mut ids = Vec::with_capacity(artifacts.len());
        {
            let mut store = self.artifacts.write().await;
            for mut artifact in artifacts.to_vec() {
                if artifact.artifact_id == ArtifactId::default() {
                    artifact.artifact_id = ArtifactId::new();
                }
                if artifact.created_at == Timestamp::from_millis(0) {
                    artifact.created_at = Timestamp::now();
                }
                ids.push(artifact.artifact_id);
                store.insert(artifact.artifact_id, artifact);
            }
        }

        {
            let mut build_index = self.build_index.write().await;
            let entry = build_index.entry(build_id).or_default();
            entry.extend(ids.iter().copied());
            if entry.len() > self.config.max_per_build {
                let excess = entry.len() - self.config.max_per_build;
                let dropped: Vec<ArtifactId> = entry.drain(..excess).collect();
                let mut store = self.artifacts.write().await;
                for id in dropped {
                    store.remove(&id);
                }
            }
        }

        {
            let mut cube_index = self.cube_index.write().await;
            let entry = cube_index.entry(cube_id).or_default();
            entry.extend(ids);
            if entry.len() > self.config.max_per_cube {
                let excess = entry.len() - self.config.max_per_cube;
                let dropped: Vec<ArtifactId> = entry.drain(..excess).collect();
                let mut store = self.artifacts.write().await;
                for id in dropped {
                    store.remove(&id);
                }
            }
        }

        artifacts.len()
    }

    /// Gets an artifact by ID.
    pub async fn get_artifact(&self, artifact_id: ArtifactId) -> Option<BuildArtifact> {
        self.artifacts.read().await.get(&artifact_id).cloned()
    }

    /// Gets all artifacts recorded for a build.
    pub async fn get_artifacts_for_build(&self, build_id: BuildId) -> Vec<BuildArtifact> {
        let ids = self
            .build_index
            .read()
            .await
            .get(&build_id)
            .cloned()
            .unwrap_or_default();
        let store = self.artifacts.read().await;
        ids.iter().filter_map(|id| store.get(id).cloned()).collect()
    }

    /// Gets all artifacts recorded for a cube.
    pub async fn get_artifacts_for_cube(&self, cube_id: CubeId) -> Vec<BuildArtifact> {
        let ids = self
            .cube_index
            .read()
            .await
            .get(&cube_id)
            .cloned()
            .unwrap_or_default();
        let store = self.artifacts.read().await;
        ids.iter().filter_map(|id| store.get(id).cloned()).collect()
    }

    /// Deletes an artifact from the index.
    pub async fn delete_artifact(&self, artifact_id: ArtifactId) -> bool {
        let removed = self.artifacts.write().await.remove(&artifact_id).is_some();
        if removed {
            let mut build_index = self.build_index.write().await;
            for ids in build_index.values_mut() {
                ids.retain(|id| *id != artifact_id);
            }
            let mut cube_index = self.cube_index.write().await;
            for ids in cube_index.values_mut() {
                ids.retain(|id| *id != artifact_id);
            }
        }
        removed
    }

    /// Removes artifacts older than the given number of milliseconds, using
    /// wall-clock timestamps rather than process uptime.
    pub async fn cleanup_old_artifacts(&self, max_age_ms: u64) -> usize {
        let now = Timestamp::now().as_millis();
        let stale: Vec<ArtifactId> = {
            let store = self.artifacts.read().await;
            store
                .values()
                .filter(|a| now.saturating_sub(a.created_at.as_millis()) > max_age_ms)
                .map(|a| a.artifact_id)
                .collect()
        };

        let count = stale.len();
        for id in stale {
            self.delete_artifact(id).await;
        }
        count
    }

    /// Total size across all indexed artifacts.
    pub async fn total_size(&self) -> u64 {
        self.artifacts.read().await.values().map(|a| a.size).sum()
    }

    /// Number of indexed artifacts.
    pub async fn len(&self) -> usize {
        self.artifacts.read().await.len()
    }
}

impl Default for ArtifactManager {
    fn default() -> Self {
        Self::new(ArtifactManagerConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::ids::ArtifactId;

    fn artifact(id: ArtifactId, path: &str, size: u64, age_ms: u64) -> BuildArtifact {
        BuildArtifact {
            artifact_id: id,
            path: path.to_string(),
            size,
            kind: nexus_core::ArtifactKind::Binary,
            created_at: Timestamp::from_millis(Timestamp::now().as_millis() - age_ms),
        }
    }

    #[tokio::test]
    async fn stores_and_retrieves_artifacts() {
        let manager = ArtifactManager::default();
        let build_id = BuildId::new();
        let cube_id = CubeId::new();

        let artifacts = vec![artifact(ArtifactId::new(), "target/debug/app", 1024, 0)];
        let stored = manager.store_artifacts(build_id, cube_id, &artifacts).await;
        assert_eq!(stored, 1);

        let from_build = manager.get_artifacts_for_build(build_id).await;
        assert_eq!(from_build.len(), 1);
        assert_eq!(from_build[0].path, "target/debug/app");

        let from_cube = manager.get_artifacts_for_cube(cube_id).await;
        assert_eq!(from_cube.len(), 1);
    }

    #[tokio::test]
    async fn cleanup_removes_stale() {
        let manager = ArtifactManager::default();
        let old = vec![artifact(ArtifactId::new(), "old.bin", 10, 10_000)];
        manager
            .store_artifacts(BuildId::new(), CubeId::new(), &old)
            .await;
        let removed = manager.cleanup_old_artifacts(1_000).await;
        assert_eq!(removed, 1);
        assert_eq!(manager.len().await, 0);
    }
}
