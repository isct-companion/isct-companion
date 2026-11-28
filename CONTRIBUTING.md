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

## Triage labels

Use `area:*` to identify the affected feature or code, such as `area:exam-schedule` for upcoming exam dates
and `area:past-exams` for the past exam library. Add `platform:*` when the report identifies a platform or
the change touches platform-specific files. Shared shell changes use `area:desktop`, `area:ios` or
`area:android`. For example, `is:issue label:area:school-auth label:platform:ipados` finds iPad sign-in
reports and `is:pr label:area:school-auth label:area:core` finds changes to shared sign-in logic.

New reports and proposals start with `needs-triage` and get area and platform labels from the form choices.
During triage, correct those labels as needed and remove `needs-triage` after the first review. Use
`needs-info` when the reporter needs to answer a specific question and `needs-repro` when someone needs to
reproduce the behaviour. Use impact labels such as `regression` and `data-loss` once supported by the report
or investigation. Use `release-blocker` when the release must wait for a fix.

PRs get area labels from changed paths. The `fix/`, `feat/`, `enhance/`, `refactor/` and `release/` branch
prefixes can supply a change kind. `documentation` and `kind:tests` are applied automatically when all
changed paths are in scope. Maintainers can add labels for impacts that cross shared code and remove stale
labels when the scope changes.

The [label catalog](.github/labels.yml) stores label descriptions and colours. The `Triage labels` workflow
syncs it when the catalog changes on `main`. When moving a feature, update its paths in the
[PR rules](.github/labeler.yml). When changing an issue form option, update the
[issue rules](.github/issue-labeler.yml) alongside both forms.

## Make a change

1. Read [AGENTS.md](AGENTS.md) and the [development guide](docs/development.md).
2. Create a branch from the current `main`, install dependencies with `yarn install --immutable`, and run `yarn desktop:dev`.
3. Keep domain rules, persistence and mutations in `crates/app-core`; the desktop interface renders Core read models and forwards intents.
4. Open a draft pull request to `main` early to get checks on the proposed merge. Explain the user-visible result and verification. Include screenshots for visible interface changes. Its title and description become the commit message.
5. Use the `Platform checks` result to choose any narrower local checks you need. Try UI changes in the affected apps. See the [test guide](apps/desktop/tests/README.md) for the CI suites.
6. Fix problems on the same branch until `Platform checks` passes, updating the branch if `main` moved. The pull request is then squashed into one commit on `main`.

A branch without a PR gets quick checks on push. Once a PR is open, its checks cover subsequent pushes. A PR can also reuse a successful push plan for the same source tree. Core and FFI changes select their Rust contracts. Android app and SDK changes select Kotlin compilation with the arm64 SDK. Run the full suite, including native integration and simulators, on the release candidate.

For work spread over several branches:

- Unrelated changes get a pull request each.
- To land several features as one commit, merge their branches into one branch and open a single pull request. That commit can only be reverted as a whole.
- To try branches together without landing them, merge them into a throwaway `try/*` branch and delete it afterwards.
- When B builds on A, branch B from A and rebase it onto `main` once A has landed.

All frontend tests live under `apps/desktop/tests/`; extend the existing behaviour suite.
Native checks requiring real credentials or signed packages belong in [`docs/verification/manual-checks.yaml`](docs/verification/manual-checks.yaml); the [manual verification record](docs/verification/manual-acceptance.md) is rendered from it.
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
