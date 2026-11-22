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
| Chaos | Failure injection and recovery behavior (crashes, drops, restarts) | planned | Pending |
| Security | Authentication/authorization boundaries, protocol abuse resistance | planned | Pending |
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

- [ ] Unit test coverage exists for all state machines and pure logic shipped
      so far.
- [ ] Integration tests cover the operational surface (handshake, auth,
      cubes, membership, presence, heartbeats, plugins, errors).
- [ ] End-to-end tests run a real server and real client over TCP.
- [ ] Performance regressions fail the suite via benchmark baselines.
- [ ] Chaos tests cover restart persistence and failure recovery.
- [ ] Security tests cover authn/authz boundaries and abuse cases.
- [ ] Stress tests cover multi-user concurrency and slow-client behavior.
- [ ] Release checklist is documented and followed.

## Relationship to performance (Part 13)

Performance budgets (`docs/architecture/performance.md`) are enforced as
regression tests: any measured regression beyond the configured budget fails
the QA suite. Optimization without a regression test is incomplete work.
