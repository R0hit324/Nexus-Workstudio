# Testing and Quality Architecture

Part 14 of the Nexus roadmap introduces the testing and quality assurance
layer. QA is treated as a first-class architectural phase, not a late cleanup
step: every capability delivered in Parts 1-13 is expected to be verifiable,
and every new capability lands with tests that prove it.

## Test layers

| Layer | Purpose | Primary location | Status |
|-------|---------|------------------|--------|
| Unit | Logic of a single module in isolation (state machines, codecs, stores, queues) | `crates/*/src/**` (`#[cfg(test)]`) | Active |
| Integration | Cross-crate behavior through public APIs (server, client, protocol, events) | `tests/integration` | Active |
| End-to-end | Real server + real client over the wire on an ephemeral port | `tests/integration`, `tests/performance` | Active |
| Performance regression | Benchmark runs compared against baselines (`nexus-core::bench::is_regressed`) | `tests/performance`, `crates/nexus-core/src/bench.rs` | Active |
| Chaos | Failure injection and recovery behavior (crashes, drops, restarts) | `tests/support` (FaultInjector), `tests/regression`, `tests/security`, `tests/integration` | Active |
| Security | Authentication/authorization boundaries, protocol abuse resistance | `tests/security` | Active |
| Fuzzing | Deterministic fuzz harnesses over frames/messages | `tests/fuzz` | Active |
| Property-based | CRDT convergence and versioning properties | `tests/property` | Active |
| Snapshot integrity | Checksum, state, and schema validation of snapshots | `crates/nexus-snapshot/src/validation.rs` | Active |
| Stress | High load, many users, many cubes, slow clients | planned | Pending |
| Release | Pre-release checklist (versioning, docs, migration, sign-off) | `docs/architecture/release.md` | Active |

## Quality gates

The canonical local verification pipeline is `scripts/verify.ps1`
(formatting via rustfmt, clippy via `RUSTC_WORKSPACE_WRAPPER`, tests, build).
On policy-compliant machines it runs the full set:

```text
cargo fmt --check
cargo clippy --workspace --all-targets
cargo test --workspace
cargo build --workspace
```

Performance-sensitive changes additionally run the benchmark + regression
suite from `tests/performance`, which asserts measured throughput against
regression budgets.

## Definition of Done (Part 14)

Part 14 is complete when:

- [x] Unit test coverage exists for all state machines and pure logic shipped
      so far.
- [x] Integration tests cover the operational surface (handshake, auth,
      cubes, membership, presence, heartbeats, plugins, errors).
- [x] End-to-end tests run a real server and real client over TCP.
- [x] Performance regressions fail the suite via benchmark baselines.
- [x] Chaos tests cover restart persistence and failure recovery.
- [x] Security tests cover authn/authz boundaries and abuse cases.
- [ ] Stress tests cover multi-user concurrency and slow-client behavior.
- [x] Release checklist is documented and followed.
- [x] Terminal stress tests exist for large ANSI input streams.

## Delivered Part 14 layers

The following layers are implemented and verified in the current branch:

- QA layer strategy and test inventory (`docs/architecture/testing.md`).
- Deterministic fixtures and isolated temp environments
      (`tests/support`).
- Failure injection (`tests/support`: `FaultInjector`, `FaultyEventStore`).
- Permanent regression suite with stable IDs (`tests/regression`).
- Security boundary suite incl. malicious-client protocol abuse
      (`tests/security`).
- Property-based synchronization convergence tests (`tests/property`).
- Deterministic fuzz harnesses for frames and messages (`tests/fuzz`).
- Snapshot integrity, corruption, and validation tests
      (`crates/nexus-snapshot/src/validation.rs`).
- Performance regression detection (`nexus-core::bench`, `tests/performance`).
- Network fault simulation: abrupt client disconnects are cleaned up without
      corrupting server state (`tests/integration`).
- Terminal stress: 100k-byte ANSI input streams never panic
      (`crates/nexus-terminal/src/parser.rs`).
- CI pipeline definition (`docs/ci`, activation pending PAT scope).
- Release testing checklist, compatibility rules, and failure artifacts
      (`docs/architecture/release.md`).

## Failure artifacts and QA reporting

When a QA gate fails, the reproduction artifacts are the ones listed in
`docs/architecture/release.md` (logs, network traces, event history, snapshot,
workspace state, crash information, build output, terminal output, AI tool
history where safe). QA runs record the failing gate, suite, commit, and
environment; confirmed defects enter the permanent regression suite through
their stable regression IDs.

## Relationship to performance (Part 13)

Performance budgets (`docs/architecture/performance.md`) are enforced as
regression tests: any measured regression beyond the configured budget fails
the QA suite. Optimization without a regression test is incomplete work.
