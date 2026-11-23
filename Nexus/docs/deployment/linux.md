# Linux Deployment

Part 15 section 3 and the "Linux deployment works" release criterion.

## Prerequisites

- Rust-free server binary (`nexus-server`) built for Linux x86_64.
- Persistent data directory (defaults to `nexus-data` under the data dir).
- Log directory (defaults to a rotating writer under the system temp/log area).

## systemd service

```ini
[Unit]
Description=Nexus Collaboration Server
After=network.target

[Service]
Type=simple
User=nexus
WorkingDirectory=/opt/nexus
ExecStart=/opt/nexus/bin/nexus-server --bind 0.0.0.0 --port 7000
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

## Firewall / exposure

- Bind to `127.0.0.1` for local-only deployments; bind to `0.0.0.0` only behind a
  trusted network/LAN with firewall rules (critical deployment rule 5).
- Allow the client port (default `7000`) only from trusted peers.

## Operations

- Health checks: liveness + readiness via `nexus-server::health` and diagnostics.
- Logs: rotating writer configured via `NEXUS_LOG_MAX_BYTES` / `NEXUS_LOG_MAX_FILES`
  or `[logging]` in the config file.
- Backup before upgrade; upgrade via forward migrations; rollback per runbook.
