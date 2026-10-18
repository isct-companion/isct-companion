# Desktop 1.1.2 review checkpoint — 2026-10-18

Latest repair verification: [Calendar and course-code lookup](calendar-code-lookup-2026-10-18.md).

Prepared on `main` after pulling `db317e4`. Public release and version-tag push
remain pending user review.

## Exam behaviour

Exams use subscription records. Core resolves full course/class identity within
the academic year and quarter. Unmatched records remain outside personal agendas.
Lecture Desk's Exams tab defaults to linked exams; entering Filter searches all
published exams in that quarter. Selecting a result opens the shared editor in
the right pane. Personal date/time/room/link corrections stay in schedule overrides.

A new course association with existing exams requires confirmation of the current
binding keys. Core rechecks inside the transaction; confirmation preserves the
previous exams. Cancelling the dialog writes nothing. Source refreshes preserve
the personal correction and both links.

## Verification

- `yarn test`: 538 passed, 4 existing skips.
- `yarn test:types`, `yarn build`, release-version validation and `git diff --check`: passed.
- `cargo test --workspace --locked`: 463 passed, 4 existing skips, including the
  generated Swift/Kotlin-facing FFI contract. The final explicit-unlink assertion
  also passed in its focused Core regression.
- Native Developer ID signed 1.1.2 launched on the existing workspace. Both school
  sessions reached Signed in using existing credentials.
- Native 2026 2Q Exams showed no automatic matches; Filter `LAL.C202` found the
  published exams, and selection displayed date, times, room and course linkage
  in the right pane. No source or personal exam record was saved during this check.
- The workspace retained 62 LMS courses, 110 assignments and 231 exams. Personal
  override/plan/event tables stayed unchanged. Credential storage was retained.
- Notifications displayed independent class/exam/assignment controls and preserved
  the legacy 10-minute exam preference. OS notification permission remains off;
  actual OS delivery is recorded separately in `DESKTOP-REMINDER-TIMES`.

Public screenshots contain published exam data and notification controls only;
account identifiers, grades, private notes and course files are excluded.

## Release and documentation

Version tags trigger source checks and all three desktop builds, then publish
with repository signing secrets and a scoped GitHub token. Manual review builds
and reuse of successful build artifacts remain available. Version checks cover
packages, Tauri/macOS bundle versions and Cargo.lock, and tags require release
notes. Publisher tests exercise draft/upload/publish order, stable/prerelease
status and the versioned notes. Live GitHub publication has not run for 1.1.2.

English and Japanese READMEs now lead with features, download links and screenshots.
CONTRIBUTING.md and the docs index route contributors to setup, architecture and
verification. Ignored native design assets are generated outputs; the design
check continues to compare all versioned desktop/Core outputs against their source.

## Change accounting

Counts below exclude documentation, config and image files. No test cases were removed.
Shared exam projections and the editor replace duplicated matching and exam forms.

- production: +1191/−374, net +817 lines.
- tests: +444/−7, net +437 lines.
- generated: +41/−1, net +40 lines.

Installed final review executable: `/Applications/IsctCompanion.app`, version 1.1.2.
SHA-256: `da984c71988cbc383e05394b6a836a855cc9b28cd114f534b0d6a2673e3286e6`.
The local review bundle is Developer ID signed; release notarization runs in CI after approval.
