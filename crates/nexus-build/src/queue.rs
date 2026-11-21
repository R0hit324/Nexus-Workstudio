//! Build queue management.

use std::collections::VecDeque;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;

use nexus_core::{BuildPriority, BuildRequest, Timestamp};

/// Build queue configuration.
#[derive(Debug, Clone)]
pub struct BuildQueueConfig {
    /// Maximum queue size.
    pub max_size: usize,
    /// Default priority.
    pub default_priority: BuildPriority,
    /// Requeue failed builds.
    pub requeue_failed: bool,
    /// Max retries for failed builds.
    pub max_retries: u32,
}

impl Default for BuildQueueConfig {
    fn default() -> Self {
        Self {
            max_size: 1000,
            default_priority: BuildPriority::Normal,
            requeue_failed: false,
            max_retries: 3,
        }
    }
}

/// An entry in the build queue.
#[derive(Debug, Clone)]
pub struct BuildQueueEntry {
    pub request: BuildRequest,
    pub priority: BuildPriority,
    pub enqueued_at: Timestamp,
    pub retry_count: u32,
}

/// Build queue for managing build requests.
pub struct BuildQueue {
    config: BuildQueueConfig,
    queue: Arc<Mutex<VecDeque<BuildQueueEntry>>>,
    high_priority: Arc<Mutex<VecDeque<BuildQueueEntry>>>,
    low_priority: Arc<Mutex<VecDeque<BuildQueueEntry>>>,
}

impl BuildQueue {
    pub fn new(config: BuildQueueConfig) -> Self {
        Self {
            config,
            queue: Arc::new(Mutex::new(VecDeque::new())),
            high_priority: Arc::new(Mutex::new(VecDeque::new())),
            low_priority: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Enqueues a build request.
    pub async fn enqueue(&self, entry: BuildQueueEntry) -> Result<()> {
        let mut queue = match entry.priority {
            BuildPriority::Interactive => self.high_priority.lock().await,
            BuildPriority::Background => self.low_priority.lock().await,
            _ => self.queue.lock().await,
        };

        if queue.len() >= self.config.max_size {
            return Err(anyhow::anyhow!("Build queue is full"));
        }

        queue.push_back(entry);
        Ok(())
    }

    /// Dequeues the next build request.
    pub async fn dequeue(&self) -> Result<Option<BuildQueueEntry>> {
        // Check high priority first
        if let Some(entry) = self.high_priority.lock().await.pop_front() {
            return Ok(Some(entry));
        }

        // Then normal priority
        if let Some(entry) = self.queue.lock().await.pop_front() {
            return Ok(Some(entry));
        }

        // Then low priority
        if let Some(entry) = self.low_priority.lock().await.pop_front() {
            return Ok(Some(entry));
        }

        Ok(None)
    }

    /// Gets the current queue length.
    pub async fn len(&self) -> usize {
        let normal = self.queue.lock().await.len();
        let high = self.high_priority.lock().await.len();
        let low = self.low_priority.lock().await.len();
        normal + high + low
    }

    /// Checks if the queue is empty.
    pub async fn is_empty(&self) -> bool {
        self.len().await == 0
    }

    /// Peeks at the next entry without removing it.
    pub async fn peek(&self) -> Option<BuildQueueEntry> {
        if let Some(entry) = self.high_priority.lock().await.front() {
            return Some(entry.clone());
        }
        if let Some(entry) = self.queue.lock().await.front() {
            return Some(entry.clone());
        }
        self.low_priority.lock().await.front().cloned()
    }

    /// Clears the queue.
    pub async fn clear(&self) {
        self.queue.lock().await.clear();
        self.high_priority.lock().await.clear();
        self.low_priority.lock().await.clear();
    }
}

impl Default for BuildQueue {
    fn default() -> Self {
        Self::new(BuildQueueConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::ids::{BuildId, CubeId, UserId};

    #[tokio::test]
    async fn test_queue_enqueue_dequeue() {
        let queue = BuildQueue::default();
        let entry = BuildQueueEntry {
            request: nexus_core::BuildRequest {
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
                    language: "rust".to_string(),
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
            },
            priority: nexus_core::BuildPriority::Normal,
            enqueued_at: Timestamp::now(),
            retry_count: 0,
        };

        queue.enqueue(entry.clone()).await.unwrap();
        let dequeued = queue.dequeue().await.unwrap();
        assert!(dequeued.is_some());
        assert_eq!(dequeued.unwrap().request.build_id, entry.request.build_id);
    }
}
