# Documentation

- [Download and features](../README.md) · [日本語](../README.ja.md)
- [Contributing](../CONTRIBUTING.md) · [Development and repository map](development.md)
- [Screenshots](screenshots/README.md) · [Release notes](releases/1.1.2.md)

## Implemented contracts

- [Shared Core API and protocol](core-api.md): desktop/native transport mapping, records, intents, queries, errors, lifecycle and MCP mapping. Swift/Kotlin SDK details are in the native integration reference.
- [Native Core SDK](core-ffi.md): Swift/Kotlin bindings, lifecycle, ports, packaging and verification.
- [Data ownership](data-ownership.md) and [data architecture](data-architecture.md): persistence and school imports.
- [School browser ports](core-school-browser.md), [downloads](core-downloads.md), [notes/backup/cloud](core-backup-sync.md): operation and recovery contracts.
- [Agent access](agent-access.md): local HTTP MCP setup and tool arguments.

## Verification and plans

- [Manual acceptance](verification/manual-acceptance.md): the behaviour CI cannot reach, rendered from [`verification/manual-checks.yaml`](verification/manual-checks.yaml). Each check names a persona, states why each step exists, and records its result with evidence. Run `yarn verify:manual stale` to see which checks a change invalidated.
- [Desktop Core checkpoint](verification/core-consolidation.md): final evidence, coverage limits and code-size accounting; replaces per-commit run logs.
- [Native SDK checkpoint](verification/native-sdk.md): host and packaged-target verification, acceptance boundary and cleanup accounting.
- [Foreign runtime spike](verification/ffi-runtime.md): historical findings; replaced by production SDK tests.
- [Core consolidation roadmap](core-consolidation.md) and [mobile UX](ux/mobile.md): future work and design decisions; use implemented API references for current call signatures.
- `tasks/`: original feature specifications, not the current transport/API reference.

Automated tests live in one place; see [apps/desktop/tests/README.md](../apps/desktop/tests/README.md) for
the suites, the shared harness and where a new test belongs.

## Release and providers

- [Desktop release](desktop-release.md) and [distribution](distribution/README.md).
- [Retired Tauri mobile implementation](../deprecated/tauri-mobile/README.md): archived adapters and release notes; native mobile apps remain planned.
- [Cloud provider setup](cloud-provider-setup.md) and [subscription repository](subscription-repository.md).

See the root [repository map](development.md#repository-map) for current paths.
Historical consolidation plans and verification receipts retain their original paths.

Local research/build evidence belongs under ignored `artifacts/`, outside Git tracking.
