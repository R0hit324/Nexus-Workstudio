# ADR-003: Serialization and Framing

- **Status:** Accepted
- **Decided:** Part 1
- **Applies to:** Part 4 (events), all future parts

## Context

The real-time, stateful, multi-channel protocol is unsuitable for HTTP/REST.
The wire format must be compact (LAN but also occasionally WAN), fast to
encode/decode, versionable, and safe against malformed input.

## Decision

- **`postcard`** (serde-based, no_std-compatible, compact binary) for message
  serialization.
- **Length-prefixed frames**: `u32` big-endian length + payload, with a hard
  maximum payload of 16 MiB.
- Messages are a typed `Message` enum (protocol-envelope style), enabling
  request/response correlation via `RequestId` and forward-compatible variant
  addition.
- Capabilities and a protocol version are exchanged during the handshake so
  peers negotiate a compatible feature set.

## Consequences

- Message schema changes are ordinary serde updates; version bumps require both
  peers because `Version` is checked during the handshake.
- `postcard`'s compactness reduces LAN bandwidth and avoids JSON escaping
  pathologies for the event stream.
- Oversized or malformed frames are rejected at the framing layer before any
  deserialization work happens.
- No REST endpoints, no HTTP semantics anywhere in the protocol surface.