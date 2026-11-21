//! Build manager - orchestrates the build queue, execution, and results.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::{Notify, RwLock, Semaphore};
use tracing::{debug, warn};

use nexus_core::{
    BuildArtifact, BuildId, BuildPriority, BuildRequest, BuildResult, CubeId, Diagnostic,
    Timestamp, UserId,
};

use crate::adapters::{
    CppAdapter, GoAdapter, JavaAdapter, JavaScriptAdapter, PythonAdapter, RustAdapter,
};
use crate::artifacts::{ArtifactManager, ArtifactManagerConfig};
use crate::diagnostics::DiagnosticParser;
use crate::queue::{BuildQueue, BuildQueueConfig, BuildQueueEntry};

/// Configuration for the build manager.
#[derive(Debug, Clone)]
pub struct BuildManagerConfig {
    /// Maximum concurrent builds server-wide.
    pub max_concurrent_builds: usize,
    /// Maximum concurrent builds per cube.
    pub max_concurrent_per_cube: usize,
    /// Maximum concurrent builds per user.
    pub max_concurrent_per_user: usize,
    /// Queue configuration.
    pub queue_config: BuildQueueConfig,
    /// Artifact storage configuration.
    pub artifact_config: ArtifactManagerConfig,
}

impl Default for BuildManagerConfig {
    fn default() -> Self {
        Self {
            max_concurrent_builds: 4,
            max_concurrent_per_cube: 1,
            max_concurrent_per_user: 2,
            queue_config: BuildQueueConfig::default(),
            artifact_config: ArtifactManagerConfig::default(),
        }
    }
}

/// Build events for the event bus.
#[derive(Debug, Clone)]
pub enum BuildEvent {
    Queued {
        build_id: BuildId,
        cube_id: CubeId,
        user_id: UserId,
    },
    Started {
        build_id: BuildId,
        timestamp: Timestamp,
    },
    Diagnostic {
        build_id: BuildId,
        diagnostic: Diagnostic,
    },
    Finished {
        build_id: BuildId,
        result: BuildResult,
    },
    Cancelled {
        build_id: BuildId,
        reason: String,
    },
}

/// A running build's cancellation handle.
struct RunningBuild {
    notify: Arc<Notify>,
    #[allow(dead_code)]
    started_at: Timestamp,
}

/// Manages the build lifecycle: queueing, execution, diagnostics, artifacts.
pub struct BuildManager {
    config: BuildManagerConfig,
    queue: Arc<BuildQueue>,
    adapters: Arc<nexus_core::LanguageAdapterRegistry>,
    #[allow(dead_code)]
    diagnostic_parser: Arc<DiagnosticParser>,
    artifact_manager: Arc<ArtifactManager>,
    running: Arc<RwLock<HashMap<BuildId, RunningBuild>>>,
    global_semaphore: Arc<Semaphore>,
    cube_semaphores: Arc<RwLock<HashMap<CubeId, Arc<Semaphore>>>>,
    user_semaphores: Arc<RwLock<HashMap<UserId, Arc<Semaphore>>>>,
    event_tx: tokio::sync::mpsc::UnboundedSender<BuildEvent>,
}

impl BuildManager {
    /// Creates a new build manager and its event channel.
    pub fn new(
        config: BuildManagerConfig,
    ) -> (Self, tokio::sync::mpsc::UnboundedReceiver<BuildEvent>) {
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();

        let mut adapters = nexus_core::LanguageAdapterRegistry::new();
        adapters.register(Box::new(RustAdapter));
        adapters.register(Box::new(JavaScriptAdapter));
        adapters.register(Box::new(PythonAdapter));
        adapters.register(Box::new(GoAdapter));
        adapters.register(Box::new(JavaAdapter));
        adapters.register(Box::new(CppAdapter));

        let mut diagnostic_parser = DiagnosticParser::new();
        diagnostic_parser.register(Box::new(crate::diagnostics::RustDiagnosticParser));

        let manager = Self {
            queue: Arc::new(BuildQueue::new(config.queue_config.clone())),
            adapters: Arc::new(adapters),
            diagnostic_parser: Arc::new(diagnostic_parser),
            artifact_manager: Arc::new(ArtifactManager::new(config.artifact_config.clone())),
            running: Arc::new(RwLock::new(HashMap::new())),
            global_semaphore: Arc::new(Semaphore::new(config.max_concurrent_builds)),
            cube_semaphores: Arc::new(RwLock::new(HashMap::new())),
            user_semaphores: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            config,
        };

        (manager, event_rx)
    }

    /// Submits a build request to the queue.
    pub async fn submit_build(&self, request: BuildRequest) -> Result<BuildId> {
        let build_id = request.build_id;
        let cube_id = request.cube_id;
        let user_id = request.user_id;

        self.queue
            .enqueue(BuildQueueEntry {
                request,
                priority: BuildPriority::Normal,
                enqueued_at: Timestamp::now(),
                retry_count: 0,
            })
            .await?;

        let _ = self.event_tx.send(BuildEvent::Queued {
            build_id,
            cube_id,
            user_id,
        });
        Ok(build_id)
    }

    /// Cancels a running build (or leaves queued ones for re-sizing later).
    pub async fn cancel_build(&self, build_id: BuildId, reason: String) -> Result<()> {
        let running = self.running.read().await;
        if let Some(entry) = running.get(&build_id) {
            entry.notify.notify_one();
            debug!(%build_id, reason = %reason, "build cancellation signalled");
        }
        Ok(())
    }

    /// Runs the build processing loop until the queue and running set are idle.
    pub async fn run(&self) {
        let mut interval = tokio::time::interval(Duration::from_millis(200));

        loop {
            interval.tick().await;

            if let Err(e) = self.try_start_next_build().await {
                warn!(error = %e, "failed to start next build");
            }

            if self.queue.is_empty().await && self.running.read().await.is_empty() {
                // Drain interval ticks to keep the timer cooperative; the
                // loop parks on a tick each time.
                debug!("build queue idle");
            }
        }
    }

    /// Starts one queued build if global capacity allows.
    async fn try_start_next_build(&self) -> Result<()> {
        // Only pop when there is spare capacity, otherwise builds starve behind
        // an ever-full semaphore while queued entries pile up.
        if self.global_semaphore.available_permits() == 0 {
            return Ok(());
        }

        let Some(entry) = self.queue.dequeue().await? else {
            return Ok(());
        };

        let request = entry.request;
        let build_id = request.build_id;

        let adapters = self.adapters.clone();
        let notify = Arc::new(Notify::new());

        self.running.write().await.insert(
            build_id,
            RunningBuild {
                notify: notify.clone(),
                started_at: Timestamp::now(),
            },
        );

        let _ = self.event_tx.send(BuildEvent::Started {
            build_id,
            timestamp: Timestamp::now(),
        });

        let language = request.configuration.language.clone();
        let cube_permit = self.get_cube_semaphore(request.cube_id).await;
        let user_permit = self.get_user_semaphore(request.user_id).await;
        let global = self.global_semaphore.clone();
        let running = self.running.clone();
        let artifact_manager = self.artifact_manager.clone();
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            let _cube = cube_permit.acquire().await;
            let _user = user_permit.acquire().await;
            let _global = global.acquire().await;

            let result = match adapters.find(&language) {
                Some(adapter) => {
                    if let Err(e) = adapter.prepare(&request).await {
                        warn!(%build_id, error = %e, "build prepare failed");
                        Err(e)
                    } else {
                        tokio::select! {
                            res = adapter.build(&request) => res,
                            _ = notify.notified() => Ok(cancelled_result(build_id)),
                        }
                    }
                }
                None => Err(nexus_core::Error::new(
                    nexus_core::ErrorCode::Build,
                    format!("no adapter for language: {language}"),
                )),
            };

            let result = result.unwrap_or_else(|e| {
                debug!(%build_id, error = %e, "recorded failed build");
                BuildResult {
                    build_id,
                    state: nexus_core::BuildState::Failed,
                    exit_code: None,
                    duration_ms: 0,
                    started_at: Timestamp::now(),
                    finished_at: Some(Timestamp::now()),
                    diagnostics: Vec::new(),
                    artifacts: Vec::new(),
                    resource_usage: nexus_core::ResourceUsage {
                        peak_memory_bytes: 0,
                        cpu_time_ms: 0,
                        wall_time_ms: 0,
                    },
                }
            });

            let _ = artifact_manager
                .store_artifacts(build_id, request.cube_id, &result_artifacts(&result))
                .await;

            for diagnostic in &result.diagnostics {
                let _ = event_tx.send(BuildEvent::Diagnostic {
                    build_id,
                    diagnostic: diagnostic.clone(),
                });
            }

            running.write().await.remove(&build_id);

            let _ = event_tx.send(BuildEvent::Finished { build_id, result });
        });

        Ok(())
    }

    async fn get_cube_semaphore(&self, cube_id: CubeId) -> Arc<Semaphore> {
        self.cube_semaphores
            .write()
            .await
            .entry(cube_id)
            .or_insert_with(|| Arc::new(Semaphore::new(self.config.max_concurrent_per_cube)))
            .clone()
    }

    async fn get_user_semaphore(&self, user_id: UserId) -> Arc<Semaphore> {
        self.user_semaphores
            .write()
            .await
            .entry(user_id)
            .or_insert_with(|| Arc::new(Semaphore::new(self.config.max_concurrent_per_user)))
            .clone()
    }

    /// Number of currently tracked running builds.
    pub async fn running_count(&self) -> usize {
        self.running.read().await.len()
    }

    /// Queue length.
    pub async fn queued_count(&self) -> usize {
        self.queue.len().await
    }
}

fn result_artifacts(result: &BuildResult) -> Vec<BuildArtifact> {
    result.artifacts.clone()
}

fn cancelled_result(build_id: BuildId) -> BuildResult {
    BuildResult {
        build_id,
        state: nexus_core::BuildState::Cancelled,
        exit_code: None,
        duration_ms: 0,
        started_at: Timestamp::now(),
        finished_at: Some(Timestamp::now()),
        diagnostics: Vec::new(),
        artifacts: Vec::new(),
        resource_usage: nexus_core::ResourceUsage {
            peak_memory_bytes: 0,
            cpu_time_ms: 0,
            wall_time_ms: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::ids::{BuildId, CubeId, UserId};

    fn sample_request() -> BuildRequest {
        BuildRequest {
            build_id: BuildId::new(),
            cube_id: CubeId::new(),
            user_id: UserId::new(),
            target: nexus_core::BuildTarget {
                target_type: nexus_core::BuildTargetType::Build,
                profile: nexus_core::BuildProfile::Debug,
                target_name: None,
                working_directory: String::new(),
                environment: Vec::new(),
                timeout_seconds: 60,
                resource_limits: nexus_core::ResourceLimits::default(),
            },
            configuration: nexus_core::BuildConfiguration {
                language: "unknown-language".to_string(),
                toolchain: None,
                build_command: None,
                test_command: None,
                check_command: None,
                lint_command: None,
                format_command: None,
                environment: Vec::new(),
                working_directory: String::new(),
                timeout_seconds: 60,
                resource_limits: nexus_core::ResourceLimits::default(),
            },
            queued_at: Timestamp::now(),
        }
    }

    #[tokio::test]
    async fn submit_queues_build_and_emits_event() {
        let (manager, mut events) = BuildManager::new(BuildManagerConfig::default());
        let request = sample_request();
        let build_id = request.build_id;

        manager.submit_build(request.clone()).await.unwrap();
        assert_eq!(manager.queued_count().await, 1);

        match events.recv().await.unwrap() {
            BuildEvent::Queued { build_id: id, .. } => assert_eq!(id, build_id),
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
