# Core and desktop normalization checkpoint

This checkpoint preserves the active desktop flows and stored-data format.
Mobile code, `deprecated/`, and its Cargo configuration are unchanged; the
mobile shells will be rewritten natively.

## Boundaries

- Workspace mutations are split into course, catalog, LMS merge, and calendar
  modules. `WorkspaceService` still owns the revision check and transaction.
- Store defaults, preferences, record helpers, catalog tables, LMS tables,
  snapshot projection, and backup/import operations have separate files. There
  is still one `DataStore` and one persistence implementation.
- Pull queue policy is separate from coordinator execution and task lifetime.
- Rust exports the first-launch fixture alongside TypeScript contracts.
  `seedWorkspace()` clones it and supplies launch identity, term, and timestamps.
  The built-in tasks and settings are no longer handwritten in TypeScript.
- `courseDetail` projects ordered material, section, assignment, exam, and
  offering IDs. CourseDetails resolves those IDs against its snapshot.
  Existing `taskView` already supplies Today and LectureDesk with shared
  deadline, submission, and hidden-assignment decisions; it is retained.
- Finder contracts join the existing ts-rs exporter. Finder rows and suggestions
  use named Rust result types. The five formerly opaque schedule query inputs
  now use typed Rust request fields. The FFI generator follows the new source
  files and preserves the SDK's existing JSON representation for stored plan
  and academic-profile inputs.
- App delegates pull display, lifecycle facts, runtime subscriptions, and
  appearance to hooks. LMS rich content, note operations, and reading-state
  persistence are separate UI modules.

## Size accounting

Raw line counts include comments and whitespace. Moving functions and tests is
not counted as deleting their implementations.

| Entry file | Before | After |
| --- | ---: | ---: |
| `workspace.rs` | 3830 | 729 |
| `data/mod.rs` | 3440 | 1158 |
| `pull_coordinator.rs` | 2379 | 1035 |
| `App.tsx` | 1922 | 1698 |
| `LmsWorkspace.tsx` | 1968 | 1678 |

Across the changed Core, FFI-generator, and desktop source files, including new
modules but excluding this document:

| Category | Before | After | Net |
| --- | ---: | ---: | ---: |
| Production source | 13538 | 13950 | +412 |
| Tests (including relocated inline Rust tests) | 4767 | 4845 | +78 |
| Generated TypeScript | 1208 | 1462 | +254 |

The removed duplicates are the TypeScript workspace/task defaults, finder
contract declarations, and course-detail projection. Typed contracts and the
new query add source overall; the large entry-file reductions are primarily
redistribution, not a net production-code reduction.

## Validation

- Core: 341 tests pass with the `ts` feature, including course-detail ordering
  and visibility, malformed query inputs, existing finder trace equivalence,
  transactional mutations, persistence, and backup compatibility.
- Desktop: TypeScript project build and all 299 tests pass (52 files).
- Shared FFI: `cargo check` and all 16 library tests pass.
- The source-safety acceptance helper now reads Rust `include_str!` constants,
  matching the existing extracted 教務 script without changing its safety checks.
