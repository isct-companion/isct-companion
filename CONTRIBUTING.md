# Contributing

Anyone can take part on GitHub; there is no club to join and no selection.
Issues may be written in Japanese, English or Chinese. Code, documentation and review are in English.

## Before you contribute

- We do not forbid AI assistance. AI may help write an issue or a pull request.
- Before you submit, align with it: read what it wrote, confirm that it matches what you actually saw and what you intended,
  and be able to explain the problem and the reason for the change in your own words when asked.
- Pull requests require basic programming knowledge. Say in the pull request template whether AI assisted you, and where.
- Wrong or missing data (academic calendar, exams, exam-week classes, departments, important dates) belongs in
  [Isct-Data issues](https://github.com/isct-companion/Isct-Data/issues). That data is updated by an AI workflow that checks the official pages every day.
- Issues and pull requests that do not meet these expectations may be closed without further explanation.

## Design and quality bar

Proposals and interface changes are judged against the project's written principles, not against "it works".
A change that works but conflicts with them is declined, with a pointer to the principle.

Before proposing a feature or a large change, read the product and design decisions and the
[design tokens](design/README.md). The decisions are kept in the project's slotledger: install it with
`npx slotledger init`, or read the generated [STATUS.md](STATUS.md). Start from `goal/product-promise`,
`goal/out-of-scope` and, for the phones, the `approach/mobile-*` keys.
A proposal starts from a problem in one of the product's primary jobs. Items in `goal/out-of-scope` and the things the
product decisions rule out (for example a local `Mark complete` that can contradict the LMS, project management or tag taxonomies) are not accepted.
Open an issue before a large change so the direction is agreed first.

Interface changes:

- **Design tokens only.** Colours, radii and workspace metrics come from `design/tokens/`; change a role there and run
  `yarn design:tokens` instead of writing literal values or page-specific copies. Follow the token rules in
  [design/README.md](design/README.md#rules), including the two uses of course colour.
- **Every state.** Hover, pressed, selected, disabled, loading and empty states are designed, not left to defaults,
  in light and dark appearance.
- **Each platform's conventions.**
  - Phones keep the system frame: tab bars, navigation bars, sheets, switches, pickers, system fonts and gestures come
    from the platform, and layouts survive the largest text sizes (`approach/mobile-moment-first` and `approach/mobile-gestures`
    in [STATUS.md](STATUS.md)).
  - Desktop is quiet and compact, like software that ships with the system: system font and desktop density, context menus,
    focus rings only during keyboard use, shadows only on floating layers, and motion only for continuity
    (150–250 ms, honouring reduced motion). This is `goal/desktop-polish-direction` in [STATUS.md](STATUS.md).
  - Platform differences are separated at compile time ([AGENTS.md §3](AGENTS.md#3-isolate-native-differences-at-compile-time-not-with-runtime-checks)).
- **Content before decoration.** Layout and spacing before cards, borders or shadows; short utility labels, no tutorial
  copy or promotional sentences (`goal/desktop-polish-direction` and `approach/settings-layering` in [STATUS.md](STATUS.md)).
- **Screenshots.** Include screenshots of each affected platform and list the states they cover.

## Open an issue

Report a reproducible problem or propose a focused improvement through [GitHub Issues](https://github.com/isct-companion/isct-companion/issues).
Include the app version, operating system, steps and expected behaviour. Use sample data in screenshots and logs; remove account identifiers and credentials before sharing.

## Make a change

1. Read [AGENTS.md](AGENTS.md) and the [development guide](docs/development.md).
2. Create a branch from the current `main`, install dependencies with `yarn install --immutable`, and run `yarn desktop:dev`.
3. Keep domain rules, persistence and mutations in `crates/app-core`; the desktop interface renders Core read models and forwards intents.
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
