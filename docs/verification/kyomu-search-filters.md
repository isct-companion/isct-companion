# Official course finder filters — 17 October 2026

## Source ownership

The production finder already derives requirement marks from `offering.kyomu`,
not from the subscription's `recommendedCourses`. The defect was that Recommended
remained a local matcher: it filtered the incomplete cached catalogue instead of
requesting the university's 推奨 search. Positive recommendation/requirement
conditions now travel to official search, and official result IDs are not rejected
for missing local recommendation metadata. The UI no longer calls these pills local.

The public controls of the authenticated search form supply level, category,
requirement, weekday/special delivery, starting-period, teaching-language and exact quarter-span
options. Only those controls can be submitted. ASP.NET checkboxes carry `on`, so
the hidden adapter uses their actual names/values rather than guessing URL enum
values. It posts only the search control or result pagination on the approved
search route. Single-valued category alternatives fan out to bounded searches.

Form metadata lives in the device-local reference cache, excluded from cloud and
user backups. Its checkpoint is renewed after 30 days or when the form changes;
ordinary searches validate controls against the actual form and can refresh it
sooner. There is no separate eager metadata crawl. Page cursors are scoped to the
URL and complete official filter selection. The existing 100-row paging,
1,000-result limit, virtual rendering and focus-to-activate search remain intact.

The verified undergraduate account offers 100, 200 and 300. A real UI
search with 200 and 推奨 checked returned 200-level rows. Its recommendation
handler selected R/A/B/C/D/E/L and left V/outside-standard-curriculum unchecked.
No registration controls were submitted. The user subsequently supplied a
graduate screenshot showing 400/500/600, R/A/B/C/D/F/I/V, periods through 12,
teaching languages and additional delivery choices. The screenshot establishes
visible choices, not HTML control IDs or submitted values. Graduate POST coverage
therefore uses explicitly synthetic controls; a real graduate session remains
unverified.

## Stable keyboard categories, dynamic choices

Core defines nine keyboard categories: level, quarter, day/delivery, starting
period, course category, requirement category, teaching language, official
recommendations and other official filters. Only categories with live options
are offered. Enter/Tab enters a category without committing a search condition;
selecting an option commits the exact captured control ID/value. Searching within
a category never turns its prefix into course-title text.

The reader discovers enabled checkbox/radio/select controls preceding the search
action, excluding result selectors. Known group identities classify semantics;
they do not enumerate the available levels, requirement letters or options.
Additional controls use the stable Other category. Typed quarter/day/period/
language shortcuts are resolved against live labels, replacing the former URL
enum map and period-10 limit. Recommendation defaults are supplied by the live
page's own filter handler rather than checkbox positions. Explicit requirement
choices take precedence. No eager search was added.

This checkpoint passed 529 frontend tests (4 skipped), 373 Core tests and the
desktop TypeScript check. Existing tests now cover entering a category with
Enter, selecting a dynamic option with Tab, changed graduate control values,
12th-period/Sunday shortcuts, overlapping quarter spans, unknown controls,
disabled/result-control exclusion and page-owned recommendation defaults.

## Credit trace

The 0.5 is an application allocation, not an official per-quarter value:
`declared_term_credits` and `earned_credits_by_term` divide a course's effective
credit total by its number of quarters. A 1-credit 1–2Q course therefore contributes
0.5 to each Q; a 6-credit 1–2Q course contributes 3 to each. No grade, stored
credit or allocation rule was changed in this checkpoint. The user explicitly
confirmed retaining equal allocation by quarter; the existing 0.5 contribution
is therefore intentional. The academic Details panel now states this rule in
English, Japanese and Chinese.

## Earlier checkpoint validation

- Frontend full regression: 520 passed, 4 skipped.
- Core library: 366 passed.
- TypeScript type check passed.
- Extended existing school extractor coverage: parsed public controls, actual
  checkbox form POST, unsupported level rejection, registration-control rejection.
- Extended Core finder coverage: no subscription recommendation dependency,
  dynamically available graduate levels, bounded category OR search, unsupported
  exclusion rejection. Historical trace expectations changed only for the new
  official recommendation/requirement request contract and remote counts.
- The source-removal handoff is `../Isct-Data/HANDOFF.md` relative to the app
  repository. Subscription datasets were not removed or published.

No native package, installed-app replacement, push or remote release in this change.
