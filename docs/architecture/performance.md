# Performance Architecture

Part 13 of the Nexus roadmap introduces the performance and optimization layer.
Performance is treated as an architectural requirement, not a final cleanup phase.

## Performance philosophy

Optimization follows a measured cycle:

```text
Measure -> Profile -> Identify bottleneck -> Form hypothesis -> Optimize -> Benchmark -> Regression test
```

Never optimize based on assumptions.

Prefer:

```text
Simple + measurable + predictable
```

over:

```text
Complex + theoretically faster + difficult to maintain
```

## Performance budgets

Initial engineering budgets (targets, not guarantees):

| Signal | Budget |
|--------|--------|
| Typical LAN typing latency | < 15 ms |
| Build result broadcast | < 100 ms after result available |
| Client baseline memory | < 80 MB |
| Server baseline memory | < 150 MB |
| Typical reconnect | < 2 seconds |
| TUI input processing | < 5 ms |
| Normal UI frame | < 16 ms |
| Workspace event propagation | low-millisecond local overhead |

Budgets are measured continuously and allow platform-specific differences.

## Metrics

The central performance metrics system lives in `nexus-core::metrics`
(`PerformanceMetrics` / `PerformanceSnapshot`) and accumulates, via lock-free
atomics:

- latency (sum + average, milliseconds)
- CPU (sum + average, milliseconds)
- memory (allocated bytes)
- network (bytes sent/received, messages sent/received)
- throughput (operation counter)

Every subsystem should expose useful metrics so the budgets above can be
continuously measured.

## Definition of Done

Part 13 is complete when the performance layer is implemented, benchmarked, and
registered against the critical architecture rules (measure before optimizing,
bounded queues, no cross-Cube blocking, avoid global locks, never hold locks
across I/O, do not block the TUI, no full-file transmission, batching/coalescing,
bounded memory, bounded terminal output, bounded AI context/tool output, bounded
plugin execution, bounded build concurrency, incremental cache invalidation,
snapshots for replay, slow clients cannot block, AI must not affect collaboration
performance, builds must not consume unlimited host resources, every subsystem
exposes measurable data, every optimization has a measurable reason, every
important regression must be detectable, predictable performance over clever
micro-optimizations, and interactive editing path priority).
