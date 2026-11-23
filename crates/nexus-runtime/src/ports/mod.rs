//! Port allocation for runtime containers.

use std::collections::HashMap;
use std::net::{TcpListener, UdpSocket};
use std::sync::Arc;

use nexus_core::CubeId;
use parking_lot::RwLock;
use tracing::debug;

/// Manages port allocation for Cube runtimes.
#[derive(Clone)]
pub struct PortAllocator {
    /// Allocated ports: (cube_id, container_port) -> host_port
    allocations: Arc<RwLock<HashMap<(CubeId, u16), u16>>>,
    /// Reserved port ranges
    reserved_ranges: Arc<RwLock<Vec<(u16, u16)>>>,
    /// Next available port (simple allocation strategy)
    next_port: Arc<RwLock<u16>>,
    /// Minimum port
    min_port: u16,
    /// Maximum port
    max_port: u16,
}

impl PortAllocator {
    /// Creates a new port allocator.
    pub fn new(min_port: u16, max_port: u16) -> Self {
        Self {
            allocations: Arc::new(RwLock::new(HashMap::new())),
            reserved_ranges: Arc::new(RwLock::new(Vec::new())),
            next_port: Arc::new(RwLock::new(min_port)),
            min_port,
            max_port,
        }
    }

    /// Creates a port allocator with default range (32768-65535).
    pub fn default() -> Self {
        Self::new(32768, 65535)
    }

    /// Reserves a port range (prevents allocation in this range).
    pub fn reserve_range(&self, start: u16, end: u16) {
        self.reserved_ranges.write().push((start, end));
    }

    /// Allocates a host port for a container port.
    pub async fn allocate(
        &self,
        cube_id: CubeId,
        container_port: u16,
    ) -> Result<u16, anyhow::Error> {
        let key = (cube_id, container_port);

        // Check if already allocated
        if let Some(port) = self.allocations.read().get(&key) {
            return Ok(*port);
        }

        // Find next available port
        let port = self.find_available_port().await?;

        self.allocations.write().insert(key, port);
        debug!(%cube_id, container_port, host_port = port, "Port allocated");
        Ok(port)
    }

    /// Finds the next available port.
    async fn find_available_port(&self) -> Result<u16, anyhow::Error> {
        let mut next = self.next_port.write();

        for _ in 0..(self.max_port - self.min_port) {
            let port = *next;
            *next = next.saturating_add(1);
            if *next > self.max_port {
                *next = self.min_port;
            }

            // Check if reserved
            let reserved = self
                .reserved_ranges
                .read()
                .iter()
                .any(|(start, end)| port >= *start && port <= *end);
            if reserved {
                continue;
            }

            // Check if already allocated
            if self.allocations.read().values().any(|&p| p == port) {
                continue;
            }

            // Test if port is actually available
            if self.is_port_free(port).await? {
                return Ok(port);
            }
        }

        Err(anyhow::anyhow!("No available ports in range"))
    }

    /// Checks if a port is free by attempting to bind to it.
    async fn is_port_free(&self, port: u16) -> Result<bool, anyhow::Error> {
        let tcp_result = TcpListener::bind(("127.0.0.1", port));
        let udp_result = UdpSocket::bind(("127.0.0.1", port));

        Ok(tcp_result.is_ok() && udp_result.is_ok())
    }

    /// Releases a port allocation.
    pub async fn release(&self, cube_id: CubeId, container_port: u16) -> Result<(), anyhow::Error> {
        let key = (cube_id, container_port);
        if self.allocations.write().remove(&key).is_some() {
            debug!(%cube_id, container_port, "Port released");
        }
        Ok(())
    }

    /// Releases all ports for a cube.
    pub async fn release_all_for_cube(&self, cube_id: CubeId) {
        let mut allocs = self.allocations.write();
        let keys: Vec<_> = allocs
            .keys()
            .filter(|(c, _)| *c == cube_id)
            .cloned()
            .collect();
        for key in keys {
            allocs.remove(&key);
        }
    }

    /// Gets the host port for a cube's container port.
    pub fn get_host_port(&self, cube_id: CubeId, container_port: u16) -> Option<u16> {
        self.allocations
            .read()
            .get(&(cube_id, container_port))
            .copied()
    }

    /// Gets all allocations for a cube.
    pub fn get_allocations(&self, cube_id: CubeId) -> Vec<(u16, u16)> {
        self.allocations
            .read()
            .iter()
            .filter(|((c, _), _)| *c == cube_id)
            .map(|((_, cp), hp)| (*cp, *hp))
            .collect()
    }
}

impl Default for PortAllocator {
    fn default() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_core::CubeId;

    #[tokio::test]
    async fn test_port_allocation() {
        let allocator = PortAllocator::new(40000, 40100);
        let cube_id = CubeId::new();

        let port1 = allocator.allocate(cube_id, 3000).await.unwrap();
        let port2 = allocator.allocate(cube_id, 3001).await.unwrap();

        assert_ne!(port1, port2);
        assert!(port1 >= 40000 && port1 <= 40100);
        assert!(port2 >= 40000 && port2 <= 40100);
    }

    #[tokio::test]
    async fn test_port_release() {
        let allocator = PortAllocator::new(40000, 40100);
        let cube_id = CubeId::new();

        let _port = allocator.allocate(cube_id, 3000).await.unwrap();
        allocator.release(cube_id, 3000).await.unwrap();

        // Should be able to reallocate (port may be different with simple allocator)
        let port2 = allocator.allocate(cube_id, 3000).await.unwrap();
        assert!(port2 >= 40000 && port2 <= 40100);
    }
}
