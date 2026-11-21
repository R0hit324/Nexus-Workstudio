use std::time::Duration;

use nexus_core::version::Version;
use nexus_protocol::{Capability, Message, Transport, PROTOCOL_VERSION};
use tracing::{debug, info, warn};

/// Outcome of the version/capability greeting exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    pub client_name: String,
    pub protocol_version: Version,
    /// Capabilities agreed by both peers (intersection of what each side
    /// advertises).
    pub capabilities: Vec<Capability>,
}

/// Performs the greeting exchange on a fresh connection.
///
/// Order (per protocol):
/// 1. wait for `ClientHello`, enforcing `timeout`;
/// 2. validate the protocol version — a mismatch is answered with a
///    `ProtocolMismatch` error and the connection is closed;
/// 3. negotiate capabilities and answer with `ServerHello`.
///
/// # Errors
/// Returns an error when the peer missed the handshake, spoke a different
/// protocol version, or the underlying transport failed.
pub async fn perform_handshake(
    transport: &mut dyn Transport,
    timeout: Duration,
    server_name: &str,
) -> anyhow::Result<Handshake> {
    let first = match tokio::time::timeout(timeout, transport.receive()).await {
        Ok(Ok(Some(message))) => message,
        Ok(Ok(None)) => return Err(anyhow::anyhow!("peer closed during handshake")),
        Ok(Err(err)) => return Err(anyhow::anyhow!("handshake receive failed: {err}")),
        Err(_) => {
            warn!("handshake_timed_out");
            return Err(anyhow::anyhow!("handshake timed out"));
        }
    };

    let Message::ClientHello {
        client_name,
        protocol_version,
        capabilities,
    } = first
    else {
        warn!(
            first_message = ?std::mem::discriminant(&first),
            "unexpected_first_message"
        );
        let _ = transport
            .send(&Message::error(
                None,
                nexus_protocol::ErrorCode::ProtocolViolation,
                "expected ClientHello",
            ))
            .await;
        return Err(anyhow::anyhow!("expected ClientHello, got {first:?}"));
    };

    if protocol_version != PROTOCOL_VERSION {
        info!(
            client = %client_name,
            client_version = %protocol_version,
            "protocol_version_mismatch"
        );
        let _ = transport
            .send(&Message::protocol_mismatch(
                protocol_version,
                PROTOCOL_VERSION,
            ))
            .await;
        return Err(anyhow::anyhow!(
            "incompatible protocol: client={protocol_version} server={PROTOCOL_VERSION}"
        ));
    }

    let negotiated = capability_intersection(&capabilities, &capabilities_list());
    debug!(
        client = %client_name,
        offered = ?capabilities,
        negotiated = ?negotiated,
        "handshake_complete"
    );
    transport
        .send(&Message::server_hello(server_name, negotiated.clone()))
        .await?;
    Ok(Handshake {
        client_name,
        protocol_version,
        capabilities: negotiated,
    })
}

/// The full set of capabilities this server can offer.
#[must_use]
pub fn capabilities_list() -> Vec<Capability> {
    vec![
        Capability::Presence,
        Capability::Chat,
        Capability::Synchronization,
        Capability::Snapshots,
    ]
}

/// The capability set both peers agree on: what the server offers and the
/// client also declares.
#[must_use]
pub fn capability_intersection(client: &[Capability], server: &[Capability]) -> Vec<Capability> {
    let mut negotiated = Vec::new();
    for capability in server {
        if client.contains(capability) {
            negotiated.push(*capability);
        }
    }
    negotiated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersection_produces_shared_set() {
        let client = vec![
            Capability::Presence,
            Capability::Chat,
            Capability::Snapshots,
        ];
        let negotiated = capability_intersection(&client, &capabilities_list());
        assert!(negotiated.contains(&Capability::Presence));
        assert!(negotiated.contains(&Capability::Chat));
        assert!(negotiated.contains(&Capability::Snapshots));
        assert!(
            !negotiated.contains(&Capability::Synchronization),
            "missing from client"
        );
    }

    #[test]
    fn empty_negotiations_are_allowed() {
        let negotiated = capability_intersection(&[], &capabilities_list());
        assert!(negotiated.is_empty());
    }
}
