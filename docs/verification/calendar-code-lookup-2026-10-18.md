# Calendar and course-code lookup — 2026-10-18

This checkpoint follows the initial 1.1.2 review build. The app release and tag
remain pending user review.

## Sources and ownership

School data was pulled again through the signed native application's authenticated
read-only LMS and 教務Web sessions. No previous database dump, cached research
file or mock school data was used as audit evidence. Eight available quarters
were checked, from 2025 1Q to 2026 4Q.

The subscription audit used a fresh clone and fresh official undergraduate and
graduate PDFs. All 22 PDF pages were visually reviewed. The source correction
was pushed to `isct-companion/Isct-Data` as `9afb0d8`: three English labels were
cleaned, and the shared LAS.B102 A–H room pool and LMS confirmation note were
preserved for all eight rows. No active exam row was missing: 215 undergraduate
and 16 graduate exams remain. An additional superseded PDF row stays excluded.

A fresh download from the published main branch matched both index hashes.
The undergraduate canonical hash is
`636ccd6744fecec3920cb596925ef9e969b8d055515275ef2c68e37f6c48f44c`.

Code is the shared search key. Internal course IDs and personal overrides retain
their existing identities. Search includes canonical codes, original LMS codes,
compound-code aliases and bilingual text. Automatic exam association uses the
canonical full code within the academic year and quarter, or an explicit
offering identity; fuzzy and historical-alias results remain search candidates.
Live verification confirmed that syllabus can expose a base code while the
linked LMS shortname retains the numeric class suffix. The projection therefore
keeps the full LMS code for that exact offering/source identity, preserves the
syllabus code as a search alias, and leaves manual courses unchanged.

The freshly pulled account has no exact course-code match against the published
2026 2Q exam feed. A linked Calendar exam cannot be demonstrated with this account
without inventing an association. LMS-only exam announcements are not imported
as subscription exams.

## Changes

- Calendar courses carry their source quarter into Lecture Desk; exams navigate
  by their exact source key and quarter into the Exams tab.
- Filter clears on selection, tab changes and quarter changes, and has a clear
  button. One Core query handles the lecture, assignment, exam and global course
  filters, including full-width input, reordered words and Latin-word typos.
- Course codes remain visible in narrow Calendar cards and beside lecture-list
  meeting information. Missing syllabus details are fetched even when LMS
  already supplied an English title; uncached official offering IDs are valid
  detail locators.
- Exam source notes are shown alongside the room candidates and in the editor.

## Checks

- Frontend build and TypeScript checks passed.
- `cargo check --workspace --locked --offline` passed, including the FFI boundary.
- The manual-check definition validates and its rendered table was updated.
- A one-off live check downloaded both published feeds, passed them through the
  production validator and ingestion code, and ran the production Core search.
  All 231 source records were found by original, lowercase and punctuation-free
  code queries: 693 successful lookups. It used an isolated in-memory database
  with actual downloaded records, no school cache or invented course links.
- Native checks found all 67 quarter/course entries across the eight quarters
  through their displayed code, with zero missing results. This counts a course
  again when it belongs to more than one quarter.
- Native Filter checks passed for a full LMS class code after syllabus metadata
  enrichment, a Latin-word transposition, reordered Japanese words and a code
  without punctuation. The clear button, item selection and all three tab
  buttons cleared the query.
- Native Calendar restored the room and course-code text. Entering a 3Q course
  from a previously open 2Q exam selected 3Q, Lectures and that exact course.
- Selecting a real published exam opened its durable key/quarter route, selected
  Exams, cleared Filter and displayed its complete room pool and source note.

No existing tests were removed. Source-audit artifacts and the live-query check
are under the ignored `artifacts/subscription-audit-20261018/` directory.

## Installed checkpoint

The final Developer ID signed 1.1.2 review bundle is installed at
`/Applications/IsctCompanion.app`. Signature and desktop-bundle isolation checks
passed after launch. Executable SHA-256:
`14bbfe22f9b3f807880300404129311aed398944a3dcfc43726d3d9ee5239356`.

The final executable repeated all 67 quarter/course code lookups with zero
failures. Full numeric class suffixes survived syllabus enrichment, and both
compound-code alternatives plus the syllabus base code found their real course.
The exam route, empty Filter, complete shared room pool and LMS note were checked
again. Both school sessions remained signed in; no personal association or exam
edit was saved.

Change accounting excludes documentation, config and images:

- production: +438/−64, net +374 lines.
- tests: +0/−0, net +0 lines.
- generated: +10/−0, net +10 lines.
