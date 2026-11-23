# ADR-007: Part 3 Server Core

- Status: accepted
- Date: 2026-09

## Context

Part 1/2 established the wire protocol, domain types, and `nexus-server`
scaffolding (accept loop, sessions-as-`SessionDescriptor`, Cube lifecycle,
heartbeats). Part 3 must turn `nexus-server` into the authoritative core:
real session identity, per-Cube workspaces, an enforced permission model,
capability-negotiated handshakes, event fan-out, and a graceful, observable
shutdown path — all on the constraint that the server remains the single
source of truth for workspace/session/permission state.

The machine this was developed on cannot execute freshly built binaries
(Windows App Control, `os error 4551`), so Part 3 was implemented and
reviewed statically; runtime verification is deferred to `scripts/verify.ps1`
on a policy-compliant machine.

## Decisions

### One task per connection; the connection task owns the transport

Each accepted socket spawns a dedicated task that owns the boxed
`Transport` end-to-end (handshake → authentication → dispatch → cleanup).
The task `select!`s four sources:

1. inbound messages (`transport.receive()`),
2. the connection's bounded outbound queue (`mpsc::Receiver`),
3. an idle deadline (heartbeat timeout),
4. the cooperative shutdown signal.

Request *replies* are written directly on the transport from the same task —
never pushed through the bounded event queue — so a peer with a full event
queue can never stall its own request/response cycle (no self-send deadlock).
Broadcast events use the event queue with a **drop-when-full** policy
(`Delivery::Try`), counting drops in `Metrics`, so one slow consumer never
backs pressure into the producers.

### Sessions are server-side records, separate from connections

`SessionManager` owns `ServerSession`s (id, user, connection, cube, state)
with a validated `SessionState` machine (`Created → Authenticated → Active`
with graceful `Disconnecting → Disconnected`). A connection is the network
channel; a session is the logical user interaction. Reconnection semantics
(Part 4+) can restore a session without pretending the socket never failed.

### Workspaces are per-Cube, version-bumped, storage-agnostic

Every Cube owns one workspace scoped to `cubes/<cube-id>/workspace`. A
`Storage` trait (existence / read / write / delete / mkdir / list / tree
remove) is implemented twice: `InMemoryStorage` (tests, `Server::new`) and
`FilesystemStorage` (production, rooted at `config.data_dir`, canonical-
containment checked against symlink escape). Mutations bump the workspace
`Version` so diverging clients can detect staleness (full CRDT/OT in a later
part). Paths are validated twice: at the workspace layer and defensively in
the storage key checker (rejects `..`, absolute paths, drive letters, control
characters, NUL; backslashes normalized).

### Permission matrix with a membership gate

`nexus_core::permission::Permission` (Owner/Editor/Viewer) is the role
vocabulary; `permissions/policy.rs` adds the CubeAction matrix and
`PermissionManager` caches roles per (cube, user) and re-evaluates on every
join/leave. Every cube-scoped operation is gated on **membership first**
(a non-member is denied even reads) and then on the role matrix. Workspace
operations additionally require **join isolation**: a session may only touch
the Cube it is currently `Active` in.

### Capability-negotiated handshake

The handshake validates versions (`PROTOCOL_VERSION`) and reports a
`ProtocolMismatch`; capabilities are the **intersection** of client and
server sets (`Presence`, `Chat`, `Synchronization`, `Snapshots` today). The
client protocol already requires `Presence`, which the server advertises.

### Trust-based authentication, rate-limited per peer

Part 3 ships `TrustAuthenticator`: a valid username *is* the identity
(LAN-first), registered idempotently in `UserRegistry`. A fixed-window rate
limiter bounds brute-force/DoS per `SocketAddr` on the pre-authentication
path. The `Authenticator` trait is the seam for tokens/SSO in later parts.

### Event fan-out skeleton

`EventBus` keeps a per-Cube `broadcast` channel (capacity 256); `EventRouter`
publishes every event to the bus (subscribers arrive in Part 4) and fans the
corresponding `Message::Event` to cube members' outbound queues, excluding
the originator's session where appropriate (join presence events, file
mutation events).

### Observable runtime state and graceful shutdown

`ServerState` exposes `active_connections` / `active_sessions` atomic
counters (matching the Part 1 contract consumed by `tests/integration`).
`ServerLifecycle` (`Starting → Initializing → Listening → Running →
Stopping → Stopped/Failed`) is crossed forward-only. A cooperative `Shutdown`
signal (triggered by `Ctrl+C`/SIGTERM via `spawn_signal_task`, or by
`RunningServer::request_shutdown`) is awaited by the accept loop and every
connection task, which flush a `Disconnect` and close.

## Consequences

- Connection handling scales by simple task-per-socket; no shared socket
  state or multiplexer in Part 3 (partitioning by Cube is possible later).
- Lost broadcast events (queue full) are counted, not buffered — acceptable
  because the client re-synchronizes from authoritative reads.
- Static-review-only delivery on this machine; `verify.ps1` must run on a
  compliant machine before declaring Part 3 complete.