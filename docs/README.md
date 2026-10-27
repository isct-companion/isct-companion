# Documentation

Decisions, plans and handoffs are not kept here: they live in the project's
slotledger (`STATUS.md` at the root is generated from it). What remains in
`docs/` is what a script, a workflow, the app or a reader outside the project
needs as a file.

- [Download and features](../README.md) · [日本語](../README.ja.md)
- [Contributing](../CONTRIBUTING.md) · [Development and repository map](development.md)
- [Screenshots](screenshots/README.md) · [Release notes](releases/)

## For users

- [Data ownership](data-ownership.md): what is stored where, what syncs, what a reset removes.
- [Agent access](agent-access.md): local HTTP MCP setup and tool arguments.
- [Privacy policy](legal/PRIVACY.md) and [notices](legal/NOTICE.md).

## Building, releasing and providers

- [Native Core SDK](core-ffi.md): building and integrating the Swift and Kotlin bindings.
- [Desktop release](desktop-release.md), [mobile release](mobile-release.md) and [distribution](distribution/README.md).
- [Cloud provider setup](cloud-provider-setup.md).
- [LMS mobile API](lms-api/README.md): the Moodle token endpoints the app reads, with its OpenAPI description.

## Verification

- [Manual acceptance](verification/manual-acceptance.md): the behaviour CI cannot reach, rendered from [`verification/manual-checks.yaml`](verification/manual-checks.yaml). Run `yarn verify:manual stale` to see which checks a change invalidated.
- [Desktop Core checkpoint](verification/core-consolidation.md) and [native SDK checkpoint](verification/native-sdk.md): evidence that manual checks cite.

Automated tests live in one place; see [apps/desktop/tests/README.md](../apps/desktop/tests/README.md) for
the suites, the shared harness and where a new test belongs.
