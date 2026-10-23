# Contributing

Report a reproducible problem or propose a focused improvement through [GitHub Issues](https://github.com/isct-companion/isct-companion/issues).
Include the app version, operating system, steps and expected behaviour. Use sample data in screenshots and logs; remove account identifiers and credentials before sharing.

## Make a change

1. Read [AGENTS.md](AGENTS.md) and the [development guide](docs/development.md).
2. Create a branch from the current `main`, install dependencies with `yarn install --immutable`, and run `yarn desktop:dev`.
3. Keep domain rules, persistence and mutations in `crates/isct-core`; the desktop interface renders Core read models and forwards intents.
4. Run `yarn test`, `yarn test:types`, `yarn backend:test` and `yarn build` for changes that cross the UI/Core boundary. See the [test guide](apps/desktop/tests/README.md) for narrower checks and existing suites.
5. Open a pull request explaining the user-visible result and verification. Include screenshots for visible interface changes.

All frontend tests live under `apps/desktop/tests/`; extend the existing behaviour suite.
Native checks requiring real credentials or signed packages belong in the [manual verification record](docs/verification/manual-acceptance.md).
Run `yarn core:types` after changing exported Rust contracts. Generate design outputs with `yarn design:tokens`.

## Documentation

Keep [README.md](README.md) and [README.ja.md](README.ja.md) aligned on features and setup.
Use `docs/` for development, API and release details, with links from [docs/README.md](docs/README.md).
Public screenshots live in `docs/screenshots/`; their [capture notes](docs/screenshots/README.md) explain how to reproduce them.

## Releases

A maintainer reviews the changes and updates all version fields before pushing a version tag.
The [desktop release workflow](docs/desktop-release.md) verifies the source, builds installers,
uses repository signing secrets and publishes a complete GitHub Release. Contributors need no local signing keys to submit a pull request.

IsctCompanion is licensed under the [GNU GPL v3 or later](LICENSE) with the additional terms in
[docs/legal/NOTICE.md](docs/legal/NOTICE.md). By submitting a contribution you license it under the same
terms, including the app-store permission, so the project can keep shipping on app stores.
Third-party components retain their own licenses.
