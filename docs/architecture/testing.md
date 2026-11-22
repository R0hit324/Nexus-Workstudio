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
| Chaos | Failure injection and recovery behavior (crashes, drops, restarts) | `tests/support` (FaultInjector), `tests/regression`, `tests/security` | Active |
| Security | Authentication/authorization boundaries, protocol abuse resistance | `tests/security` | Active |
| Stress | High load, many users, many cubes, slow clients | planned | Pending |
| Release | Pre-release checklist (versioning, docs, migration, sign-off) | `docs/architecture/release.md` | Active |
| Fuzzing | Deterministic fuzz harnesses over frames/messages | planned | Pending |

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

- [ ] Unit test coverage exists for all state machines and pure logic shipped
      so far.
- [ ] Integration tests cover the operational surface (handshake, auth,
      cubes, membership, presence, heartbeats, plugins, errors).
- [ ] End-to-end tests run a real server and real client over TCP.
- [ ] Performance regressions fail the suite via benchmark baselines.
- [ ] Chaos tests cover restart persistence and failure recovery.
- [ ] Security tests cover authn/authz boundaries and abuse cases.
- [ ] Stress tests cover multi-user concurrency and slow-client behavior.
- [x] Security tests cover authn/authz boundaries and abuse cases.
- [ ] Release checklist is documented and followed.
- [x] Security boundary and malicious-client suites are in place
      (`tests/security`); critical boundaries and handshake ordering are
      covered.

## Delivered Part 14 layers

The following layers are implemented and verified in the current branch:

- QA layer strategy and test inventory (`docs/architecture/testing.md`).
- Deterministic fixtures and isolated temp environments
      (`tests/support`).
- Failure injection (`tests/support`: `FaultInjector`, `FaultyEventStore`).
- Permanent regression suite with stable IDs (`tests/regression`).
- Security boundary suite (`tests/security`).
- Property-based synchronization tests (`tests/property`).
- Performance regression detection (`nexus-core::bench`, `tests/performance`).
- CI pipeline definition (`docs/ci`, activation pending PAT scope).
- Release testing checklist and compatibility rules (`docs/architecture/release.md`).

## Relationship to performance (Part 13)

Performance budgets (`docs/architecture/performance.md`) are enforced as
regression tests: any measured regression beyond the configured budget fails
the QA suite. Optimization without a regression test is incomplete work.
