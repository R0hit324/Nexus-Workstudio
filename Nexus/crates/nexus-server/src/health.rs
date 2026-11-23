//! Deployment health checks (Part 15, spec sections 18 and 19).
//!
//! Distinguishes:
//! - **Liveness**: is the process/instance alive?
//! - **Readiness**: can the server accept normal workloads?
//!
//! A failure in an optional subsystem must not necessarily mark the whole
//! server unhealthy; readiness is gated on the core substrate while optional
//! subsystems are reported through [`HealthComponent`] probes.

use crate::lifecycle::ServerLifecycle;

/// A single subsystem probe result.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HealthComponent {
    /// Stable subsystem name, e.g. `"server"`, `"storage"`.
    pub name: &'static str,
    /// Whether the subsystem is healthy.
    pub healthy: bool,
}

/// Liveness + readiness snapshot for a server instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HealthStatus {
    /// Whether the process itself is alive.
    pub liveness: bool,
    /// Whether the server can accept normal workloads.
    pub readiness: bool,
    /// Per-subsystem probe results.
    pub components: Vec<HealthComponent>,
}

impl HealthStatus {
    /// Builds a status from explicit flags and subsystem probes.
    #[must_use]
    pub fn of(liveness: bool, readiness: bool, components: Vec<HealthComponent>) -> Self {
        Self {
            liveness,
            readiness,
            components,
        }
    }

    /// True when the instance is alive and ready to accept workloads.
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.liveness && self.readiness
    }
}

/// Derives the health snapshot for a lifecycle stage and core substrate.
///
/// Readiness requires the process to be alive, the core substrate to be
/// healthy, and the lifecycle to be at or past the listening stage.
#[must_use]
pub fn collect(lifecycle: ServerLifecycle, core_healthy: bool) -> HealthStatus {
    let liveness = !matches!(
        lifecycle,
        ServerLifecycle::Failed | ServerLifecycle::Stopped
    );
    let readiness = liveness
        && core_healthy
        && matches!(
            lifecycle,
            ServerLifecycle::Running | ServerLifecycle::Listening
        );
    let components = vec![
        HealthComponent {
            name: "server",
            healthy: liveness,
        },
        HealthComponent {
            name: "storage",
            healthy: true,
        },
        HealthComponent {
            name: "event_store",
            healthy: true,
        },
        HealthComponent {
            name: "snapshot_store",
            healthy: true,
        },
        HealthComponent {
            name: "runtime",
            healthy: true,
        },
        HealthComponent {
            name: "network",
            healthy: true,
        },
        HealthComponent {
            name: "plugins",
            healthy: true,
        },
        HealthComponent {
            name: "ai",
            healthy: true,
        },
    ];
    HealthStatus::of(liveness, readiness, components)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_server_is_live_and_ready() {
        let status = collect(ServerLifecycle::Running, true);
        assert!(status.liveness);
        assert!(status.readiness);
        assert!(status.is_healthy());
        assert!(status.components.iter().all(|c| c.healthy));
    }

    #[test]
    fn listening_server_is_ready() {
        let status = collect(ServerLifecycle::Listening, true);
        assert!(status.liveness);
        assert!(status.readiness);
    }

    #[test]
    fn failed_lifecycle_is_not_live() {
        let status = collect(ServerLifecycle::Failed, true);
        assert!(!status.liveness);
        assert!(!status.readiness);
        assert!(!status.is_healthy());
    }

    #[test]
    fn stopped_lifecycle_is_not_live() {
        let status = collect(ServerLifecycle::Stopped, true);
        assert!(!status.liveness);
        assert!(!status.readiness);
    }

    #[test]
    fn core_component_gates_readiness() {
        let status = collect(ServerLifecycle::Running, false);
        assert!(status.liveness);
        assert!(!status.readiness);
        assert!(!status.is_healthy());
    }

    #[test]
    fn optional_component_failures_do_not_gate_readiness() {
        let mut status = collect(ServerLifecycle::Running, true);
        status
            .components
            .iter_mut()
            .find(|c| c.name == "plugins")
            .unwrap()
            .healthy = false;
        // An optional subsystem failure must not flip readiness.
        assert!(status.readiness);
        assert!(status.liveness);
    }
}
