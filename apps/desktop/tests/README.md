# Tests

Every JavaScript/TypeScript test in this repository lives under this directory. There are no
`*.test.ts` files beside production modules; a suite is grouped by the
**behaviour** it protects, not by the file it happens to exercise.

```
apps/desktop/tests/
  vitest.config.ts     one runner configuration for the whole tree
  harness/             the shared fixtures, fakes and mount helpers
  unit/                pure logic: parsers, scheduling, formatting, policy
  e2e/                 the real shell over a scripted host
  live/                opt-in, networked, hits the real school site
```

## Running

```sh
yarn test          # unit + e2e (this is the gate; there is nothing else to run)
yarn test:watch    # watch mode
yarn test:live     # the networked school-site suite (needs school network)
yarn backend:test  # Rust workspace tests
```

Suites declare their environment on line 1. A suite that touches the DOM starts
with `// @vitest-environment jsdom`; everything else runs in plain node.

## Where a test belongs

| The thing you are protecting                                     | Suite                                              |
| ---------------------------------------------------------------- | -------------------------------------------------- |
| Dates, quarters, timetable geometry, locales, text               | `unit/schedule-and-dates`, `unit/text-and-locales` |
| Course eligibility, finder ranking, editors                      | `unit/courses-and-eligibility`                     |     | LMS files, previews, lessons, assignments, OCR input | `unit/lms-content` |
| School hosts, login scripts, sessions, injected web scripts      | `unit/school-sites`                                |
| Public fixture provenance, bundled subscription data             | `unit/school-extractors`                           |
| Bridge contracts, capability policy, query caches, store, routes | `unit/workspace-store`                             |
| Build/release tooling, generated artifacts                       | `unit/tooling`                                     |
| Shell navigation, retention, shortcuts                           | `e2e/app-shell`                                    |
| Planner add/replace/undo, plan editor                            | `e2e/planner`                                      |
| LMS reading workspace, viewers, notes                            | `e2e/lms-workspace`                                |
| Settings pages                                                   | `e2e/settings`                                     |
| Today, lectures, grades, syllabus details                        | `e2e/today-and-lectures`                           |
| Scheduled tasks and their run records                            | `e2e/scheduler`                                    |
| The real syllabus site's current markup                          | `live/school-site`                                 |

Add a file only when a whole new area appears. Adding a test to an existing
area means adding a case to that suite.

`unit/` never mounts a React component. A suite that renders production JSX
belongs in `e2e/`, even when it mounts a single page rather than the shell — the
distinction is "does this test the app as it is wired" rather than "how much of
it does this mount".

## Writing a test

Two rules decide almost everything:

1. **Assert behaviour, not implementation.** Drive the real production module
   and read what a person would see. Do not assert that a helper was called, and
   do not re-implement the algorithm inside the test.
2. **Prefer the real thing.** `vi.mock` is a last resort — it replaces the code
   under test with a mock, so the suite stops proving anything about the app.
   Reach for the harness first:

   - module needs a platform fact → `installFakeCore` / `mountApp` options
   - module needs data → builders in `harness/fixtures.ts`
   - component needs a DOM → `mount` from `harness/render.tsx`

   Mock only what the test cannot otherwise own: a third-party renderer with no
   headless mode (`pdfjs-dist`), or a canvas/WebGL surface.

Keep a failing case alongside the passing one. A test that only proves the happy
path usually misses the bug it was written for.

## The harness

```ts
import {
  mountApp, // real shell + scripted host, returns { host, core, go, unmount }
  installFakeCore, // the scripted host alone, for non-UI suites
  installComponentHost, // scripted host + adopted snapshot, for one-component suites
  freshRevision, // a unique snapshot revision, for a suite that mounts twice
  mount,
  flush,
  advance,
  until,
  click,
  typeInto,
  textOf,
  buttonByText,
  watchConsoleErrors,
  seedWorkspace, // via workspaceFixture()
  offering,
  courseFor,
  planItemFor,
  workspaceFixture,
  overviewFor,
  planningWeekFor,
  scheduleViewFor,
  emptySchoolRecords,
  emptyCredentialStatus,
  repositoryRoot,
  readRepoFile,
} from "@tests/harness";
```

Import production code through the `@app` alias (`@app/lib/model`,
`@app/platform/desktop-bridge`) so the suite's location stays independent of the
module's.

### `installFakeCore(options)`

Replaces the Tauri host with an in-memory one: it records every command, applies
`mutate_workspace` intents, advances the revision, and answers read models from
a table. A commit answers the whole live store under every change name
(`core.commit()`, also for a `handle` that commits by itself).

```ts
const core = installFakeCore({
  workspace,                      // the snapshot Core would return
  queries: {                      // answer schedule_query by kind
    workspaceOverview: (_query, core) => overviewFor(core.workspace),
  },
  finder: (request) => ({ ... }), // answer finder_query
  handle: (command, args) => {    // any other command
    if (command === "catalog_offerings") return [offering];
    return undefined;             // fall through to null
  },
  apply: (intent, core) => {      // commit an accepted mutation
    if (intent.kind === "addCatalogCourses") core.publish(next, "revision:2");
  },
});
core.calls                        // every command sent, in order
core.intents                      // every accepted intent, in order
core.emit("workspace-changed", { source: "school-pull", changes: ["courses"] });
                                  // the shell patches through `workspace_records`
```

Answer read models from `core.workspace` rather than closing over a literal, or
the render will not follow a committed mutation.

### `mountApp(options, { route })`

Installs the scripted host and mounts the real shell. It clears browser storage,
seeds the hash route, builds a fresh router and disposes it on unmount.

```tsx
let app: MountedApp | undefined;
afterEach(async () => { await app?.unmount(); app = undefined; });

it("...", async () => {
  app = await mountApp({ workspace, queries: { ... } }, { route: paths.home() });
  const { host, core } = app;
  await until(() => expect(host.textContent).toContain("Placed"));
});
```

The shell captures its bridge at first import, so `mountApp` must be the first
thing that imports application code. Never `import App from "@app/app/App"` at
the top of a suite; `mountApp` owns that ordering.

## The live suite

`live/school-site` talks to the real Science Tokyo syllabus site. It is skipped
unless `ISCT_LIVE_SCHOOL_TESTS=1`, because CI has no campus network. It exists to
catch the thing fixtures cannot: the school changing its markup. Run it after any
syllabus site change and before changing a school parser.

```sh
yarn test:live
```

Its assertions must describe the site, not a snapshot of it: parse the live page
and check that fields the parser promises are actually populated.

## What is not a test

Some behaviour cannot be reached from here at all: a real student id and OTP, a
provider's consent screen, a signed bundle, a physical campus card, a native
shell, a physical device. Those belong in
[`docs/verification/manual-checks.yaml`](../../../docs/verification/manual-checks.yaml),
where each check names a persona, states why each step exists, says where to
observe the result, and records its outcome with evidence. Do not write an
end-to-end script that fakes its way to a flow like that — a script that mocks
the credential prompt is just a slower component test, and the real risk stays
uncovered.

The same goes for a result a person sees at a glance in the real app — a
layout, a split cell, a label, which page a click opens. Add a short check there
and name it to the user instead of scripting the screen here; test the logic
behind it at the layer that owns it.

```sh
yarn verify:manual stale           # what a pending change invalidated
yarn verify:manual sheet <ID>      # a run sheet to follow, by hand or by agent
yarn verify:manual record <ID> --result pass --by human --evidence <path>
```

The table is enforced from both sides: a step without a reason is rejected, a
check that covers only test files is rejected, and a recorded pass without
evidence is rejected. `stale` compares each check's recorded commit against the
commits that touched the paths it covers, so a change names the checks it
invalidated instead of leaving you to guess.

## Fixtures

Committed school HTML lives in `crates/app-core/tests/fixtures/`, because Core's
Rust tests embed it with `include_str!` and the frontend suites read the same
bytes. `content-provenance.json` pins every public page's URL, capture date and
SHA-256. Refresh a fixture by re-fetching the URL, replacing the file and
updating its `bytes`/`sha256` — never by editing the HTML by hand.

Fixtures contain only public catalogue text: no personal data, no grades, no
names that are not already published.

## Rust tests

Rust is the same shape, with one folder per crate:

```
crates/app-core/src/tests/        one folder for the whole crate
crates/app-ffi/src/tests/
crates/app-ocr/src/tests/
apps/desktop/src-tauri/src/tests/
```

Each file is named after the module whose behaviour it covers
(`schedule__timing.rs` is `schedule::timing`), and the declaration stays inside
that module:

```rust
// in crates/app-core/src/schedule/timing.rs
#[cfg(test)]
#[path = "../tests/schedule__timing.rs"]
mod tests;
```

The `#[path]` indirection is deliberate. `mod tests` remains a child of
`schedule::timing`, so `use super::*` and access to private items keep working
and no visibility had to be widened. Only the file moved.

Two rules keep this from growing:

- **No trivial tests.** If reading the name and body tells you nothing beyond
  what the code under test already says — one `assert_eq!` over an inline
  literal, a getter round-trip — delete it.
- **No test that a sibling already covers.** If another test in the crate
  exercises the same function and boundary, fold any distinctive assertions into
  that test and delete the narrower one. A deletion needs a named survivor.

Integration tests are the exception cargo dictates: `apps/desktop/src-tauri/tests/`
holds the ones that link the crate from outside, named for their behaviour
(`school_ingest_boundaries.rs`, `subscriptions_acceptance.rs`) rather than for
the plan item that produced them.
