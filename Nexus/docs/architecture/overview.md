# Nexus Architecture Overview

## Purpose

Nexus is a LAN-first, multi-user, real-time collaborative TUI development
platform. A small team runs one self-hosted server; each member drives a
terminal UI against it. The server is the authoritative source of truth for
workspace state, sessions, permissions, Cubes, events, and collaboration state.

## Principles

- **Server-authoritative.** Clients keep synchronized local representations
  but never become independent sources of truth.
- **Relational by design.** The model is best expressed relationally
  (workspaces, projects, files, sessions, Cubes), not as a document tree.
- **Reactive reads.** Reads are "belief-tree and annotative": every client view
  is derived from an event stream plus local annotations; the UI re-renders
  from that derived state.
- **No HTTP, no REST.** A custom compact wire protocol exists because the
  platform's real-time, stateful, multi-channel needs match poorly with
  REST/HTTP semantics.
- **Strict capability boundaries.** Each crate owns one concern (ADR-002).
  Domain code depends on `nexus-core` types and the `Transport` abstraction,
  never on TCP.

## Runtime topology

```text
                 +-------------------------------+
                 |           Server              |
   clients --->  |  sessions  cubes  events ...  |
                 +-------------------------------+
        |                  |                  |
   nexus-tui         nexus-client         nexus-runtime
   (local UI)     (protocol client)    (Cube isolation)
```

## Part 1 crate layout

| Crate          | Responsibility (Part 1)                    |
|----------------|--------------------------------------------|
| `nexus-core`   | Config, logging, error ladder, ids, events, workspace & session vocabulary |
| `nexus-protocol` | Messages, postcard + length-prefixed frames, transport trait, TCP transport, capabilities |
| `nexus-server` | Accept loop, sessions, Cube lifecycle/membership, heartbeat protocol |
| `nexus-client` | Handshake, authentication, Cube operations, heartbeats |
| `tests/*`      | integration, protocol conformance, performance smoke |
| others         | reserved stubs until their parts          |

## Part 2 crate layout (TUI client architecture)

| Crate          | Responsibility (Part 2)                    |
|----------------|--------------------------------------------|
| `nexus-tui`    | ratatui render layer + application (App, panels, editor, keymap, commands) |
| `nexus-client` | Service facade: `SessionService`, `WorkspaceService`, event sink, offline simulation |
| `nexus-protocol` | unchanged |
| `nexus-server` | unchanged |

`nexus-tui` is split into two strict layers (see `nexus-tui/src/lib.rs`):

1. **Application layer** (terminal-free, unit-tested): `App`, document editor
   (`GapBuffer` gap-buffer text storage, cursor/selection, incremental search,
   undo, syntax highlighting, remote cursors), tabs/splits, command palette,
   notifications, dialogs, keymap, actions/commands, and per-panel state
   (explorer, users, chat, terminal, build).
2. **Render layer** (thin, ratatui-only): `render_frame` in `app.rs` converts
   the application snapshot into widgets. All panel accessors return terminal-
   independent data; the renderer never talks to the network or the editor.

Input flows one way: crossterm `Event` → `InputTranslator` → `UiEvent` →
`App::handle_event` → commands/actions. Server data flows: `SessionService`
emits `NetworkEvent`s into an event sink; `App::tick` drains the sink and
updates panels. Widgets never reach across layers.

`nexus-client` does not depend on `nexus-tui`. In Part 2 its `SessionService`
is a faithful *offline simulation*: operations update local `ClientState` and
emit the exact `NetworkEvent`s the real transport will, so the UI binds to the
service once and a live transport can replace it later without UI changes.

## Part 3 crate layout (server core)

Part 3 turns `nexus-server` into a real, authorized, layered network server on
top of the Part 1/2 protocol and domain types. The layout inside
`crates/nexus-server/src`:

| Module        | Responsibility                                |
|---------------|----------------------------------------------|
| `server`      | `Server`/`RunningServer`/`ServerState`; accept loop, connection tasks, graceful shutdown (`ServerInner`) |
| `session`     | `SessionState` machine + authoritative `SessionManager` (one session per connection) |
| `connection`  | `ConnectionManager`: registration, session binding, cube binding, bounded outbound queues with `Delivery::{Await,Try}` |
| `protocol`    | handshake (version + capability negotiation) and the request `Dispatcher` + permission/join-isolation guards |
| `permissions` | Owner/Editor/Viewer policy matrix (`policy.rs`) with membership gate (`PermissionManager`) |
| `workspace`   | per-Cube `CubeWorkspace` over `Storage`, path validation, version-bumped mutations |
| `storage`     | `Storage` trait + `InMemoryStorage` (tests) and `FilesystemStorage` (production, containment-checked) |
| `events`      | per-Cube `EventBus` + `EventRouter` scaling events to live connections |
| `users`/`auth`| `UserRegistry` (idempotent username → user) and `TrustAuthenticator` with per-peer rate limiting |
| `lifecycle`   | `ServerLifecycle` state machine (`Starting → … → Stopped/Failed`) |
| `shutdown`    | cooperative `Shutdown` signal (`Ctrl+C`/SIGTERM) |
| `metrics`     | lock-free counters exposed via `ServerState` and a `Metrics` handle |

Connection handling is one task per connection owning the `Transport`. The
task `select!`s inbound messages, the connection's bounded outbound event
queue, an idle (heartbeat) deadline, and the shutdown signal. Request replies
are written directly on the transport (never through the bounded event queue,
so a slow peer cannot deadlock its own requests); broadcast events use a
drop-when-full policy so one slow consumer never blocks the producers.
Workspace mutations are additionally guarded by join isolation (a session may
only touch the Cube it is currently in) and by the permission matrix.

### Part 3 data flow

1. Handshake: `ClientHello` → version check → capability negotiation →
   `ServerHello`.
2. Authentication: `Authenticate` → `TrustAuthenticator` (rate-limited) →
   `SessionDescriptor`.
3. Request/response over the protocol surface (Cubes, workspace CRUD); events
   (presence, file edits) fan out via `EventRouter` → connection outbound
   queues.
4. Idle heartbeat deadline and cooperative shutdown bound every connection.

## Data flow

1. A client connects; the server verifies the handshake and protocol version.
2. The client authenticates and receives a `SessionDescriptor`.
3. The client issues Cube operations as request/response messages.
4. Heartbeats keep the connection alive; the server enforces an idle
   deadline per session.
5. Event fan-out (the reactive stream every client renders from) arrives with
   the event engine; Part 3 ships the bus, router, and drop-when-full fan-out
   skeleton, codified in ADR-007.

## Configuration and logging

- Precedence: config file → environment variables → CLI flags.
- Config is TOML; see `crates/nexus-core/src/config.rs` for keys and the
  `NEXUS_*` environment variables.
- Logging uses `tracing` with text or JSON formats and an env-driven filter.

## Decision records

See `docs/decisions/` for the ADRs covering language/tooling, decomposition,
wire format, configuration, error/logging conventions, text-buffer storage
(ADR-006), and the Part 3 server-core design (ADR-007).