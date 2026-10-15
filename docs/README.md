# Documentation

## Implemented contracts

- [Shared Core API and protocol](core-api.md): desktop/native transport mapping, records, intents, queries, errors, lifecycle and MCP mapping. Application Swift/Kotlin bindings remain deferred.
- [Data ownership](data-ownership.md) and [data architecture](data-architecture.md): persistence and school imports.
- [School browser ports](core-school-browser.md), [downloads](core-downloads.md), [notes/backup/cloud](core-backup-sync.md): operation and recovery contracts.
- [Agent access](agent-access.md): local HTTP MCP setup and tool arguments.

## Verification and plans

- [Desktop Core checkpoint](verification/core-consolidation.md): final evidence, coverage limits and code-size accounting; replaces per-commit run logs.
- [Foreign runtime spike](verification/ffi-runtime.md): isolated binding experiment, not production FFI.
- [Core consolidation roadmap](core-consolidation.md) and [mobile UX](ux/mobile.md): future work and design decisions; use implemented API references for current call signatures.
- `tasks/`: original feature specifications, not the current transport/API reference.

## Release and providers

- [Desktop release](desktop-release.md) and [distribution](distribution/README.md).
- [Retired Tauri mobile implementation](../deprecated/tauri-mobile/README.md): archived adapters and release notes; native mobile apps remain planned.
- [Cloud provider setup](cloud-provider-setup.md) and [subscription repository](subscription-repository.md).

See the root [repository map](../README.md#repository-map) for current paths.
Historical consolidation plans and verification receipts retain their original paths.

Local research/build evidence belongs under ignored `artifacts/`, outside Git tracking.
