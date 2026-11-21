# Nexus

Nexus is a LAN-first, multi-user, real-time collaborative TUI development
platform for small teams. It brings together workspace state synchronization, a
terminal user interface with remote cube management, shared terminals, builds,
snapshots, plugins, and an AI worker into a single self-hosted tool.

Design is specified across fifteen sequential parts in `..# Nexus — Part N …`
files. This repository implements them part by part.

## Current status

| Phase | Scope | Status |
|-------|-------|--------|
| Part 1 | Project Foundation | Done |
| Part 2 | Text buffer & synchronous TUI | Done |
| Part 3 | Server core | Done |
| Part 4 | Event system | Done |
| Part 5–10 | Snapshots, terminals, build, runtime, sync | Done |
| Part 11 | Plugin system (host, registry, managers, builtins, wire+TUI surface) | Done |

## Repository layout

```text
crates/
  nexus-core        foundational types: config, logging, error ladder, ids, events
  nexus-protocol    wire protocol: messages, postcard frames, transport abstraction
  nexus-server      authoritative server: sessions, Cubes, lifecycle, heartbeats
  nexus-client      client library + CLI
  nexus-tui         terminal UI            (reserved in Part 1)
  nexus-workspace   workspace model        (reserved until Part 2)
  nexus-events      event engine: sequencing, storage, bus, replay, projections
  nexus-snapshot    snapshots/versioning   (reserved until Part 5)
  nexus-terminal    terminals              (reserved until Part 6)
  nexus-build       build integration      (reserved until Part 7)
  nexus-runtime     Cube runtimes          (reserved until Part 8)
  nexus-plugin      plugin system          (Part 11: host, registry, builtins, wire surface)
  nexus-ai          AI subsystem           (reserved until Part 12)
tests/
  integration       end-to-end server+client tests
  protocol          wire-conformance tests
  performance       performance smoke tests
docs/
  architecture      design overview
  protocol          wire-protocol documentation
  development       contributing & coding standards
  decisions         architecture decision records (ADRs)
scripts/
  verify.ps1        workspace-wide verification
```

## Building and testing

Requires a Rust stable toolchain (rustup default). Everything is a cargo
workspace:

```powershell
cargo build --workspace
cargo test    --workspace
```

See `docs/development/contributing.md` for the verification pipeline used on
this machine (including the clippy/fmt workarounds required there).

## Running

Start the server (binds `0.0.0.0:7000` by default):

```powershell
cargo run -p nexus-server
```

Exercise it from the client CLI in a second terminal:

```powershell
cargo run -p nexus-client -- list
cargo run -p nexus-client -- create my-cube
```

Configuration may come from a config file, environment variables, or CLI
flags; see `crates/nexus-core/src/config.rs` and `docs/architecture/overview.md`.

## License

MIT. See `LICENSE`.