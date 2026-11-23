# Part 15 Roadmaps (Remaining Scope)

Part 15 sections 60-90 and the final DoD entry "roadmap" items: plugin
ecosystem, language adapters, debugger, AI, remote Cubes, WAN, distributed
builds, and security roadmaps.

## Plugin ecosystem

Delivered: plugin host, registry, managers, builtins, wire + TUI surface, plugin
API version, lifecycle/migration hooks.

Roadmap: stable third-party plugin ABI, review cadence, signing, hot-reload
policy, third-party registry.

## Language adapter ecosystem

Delivered: build/runtime adapter ports.

Roadmap: additional language adapters (the delivered ones cover the release),
adapter conformance suite, sandboxed execution policy per adapter.

## Debugger roadmap

Roadmap: remote debugging protocol, session attach, breakpoint surface over the
wire, replay tooling. Delivery precedes the debugger item in stable release.

## AI roadmap

Delivered: AI subsystem, provider registry, mock provider, security/permission
gating.

Roadmap: agent loop milestones, tool approval flows, evaluation harness, model
routing policies, cost budgets.

## Remote Cube architecture

Delivered: collaboration/Cube state.

Roadmap: WAN transport, relay/backoff design, offline-first merge.

## WAN roadmap

Roadmap: relay infrastructure, TLS defaults, identity federation, multi-region
deployment. Local-first/LAN remains the initial deployment mode; WAN is the
documented future mode.

## Distributed build roadmap

Delivered: build manager, adapters, queue.

Roadmap: multi-server build distribution, artifact caching, coordination.

## Security roadmap

Delivered: security boundary suites, authn/authz, protocol abuse, fault
injection, fuzz, regression.

Roadmap: supply-chain signing, secret management, network encryption
(transport security), formal threat modeling.

## Status

The above items are tracked as roadmap documentation; they are not blockers for
the current deployment baseline. Each roadmap item lands with its own spec slice
before the corresponding stable-release criterion is evaluated.
