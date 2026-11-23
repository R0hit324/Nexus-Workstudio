# Operations and Monitoring

Part 15 keeps the deployment lifecycle operable: monitoring, metrics,
logging, rotation, troubleshooting, and diagnostic bundles.

## Monitoring and metrics endpoint

The deployment exposes operational telemetry:

- Server metrics (connections, sessions, events, latency) via the metrics
  surface used by the performance/QA layers.
- A dedicated metrics endpoint (Part 15 section 23) for external scrapers.

A failure in an optional subsystem (plugins, AI) must not mark the whole
server unhealthy; readiness gating is implemented in
`nexus-server::health`.

## Logging and rotation

- Structured logs via the configured log level (`NEXUS_LOG_LEVEL`).
- Rotation policy for long-running deployments (Part 15 section 21).

## Troubleshooting and diagnostic bundle

- `nexus-server` troubleshooting mode (Part 15 section 51) maps symptoms to
  checks.
- Diagnostic bundle (Part 15 section 52) collects: logs, network traces, event
  history, snapshots, workspace state, crash information, build output, and
  terminal output for support/incident response.

## Incident response and rollback

- Operational runbook and incident response procedures live with the release
  and deployment documentation.
- Rollback strategy: stop server, back up, install previous version, restore,
  validate storage, start server, health check.
