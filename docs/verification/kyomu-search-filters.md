# Official course finder filters — 17 October 2026

## Source ownership

The production finder already derives requirement marks from `offering.kyomu`,
not from the subscription's `recommendedCourses`. The defect was that Recommended
remained a local matcher: it filtered the incomplete cached catalogue instead of
requesting the university's 推奨 search. Positive recommendation/requirement
conditions now travel to official search, and official result IDs are not rejected
for missing local recommendation metadata. The UI no longer calls these pills local.

The public controls of the authenticated search form supply level, category,
requirement, weekday/special delivery, starting-period and exact quarter-span
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

Current live account: undergraduate form offers 100, 200 and 300. A real UI
search with 200 and 推奨 checked returned 200-level rows. Its recommendation
handler selected R/A/B/C/D/E/L and left V/outside-standard-curriculum unchecked.
No registration controls were submitted. The account has no graduate search
entrance; graduate page/route validation awaits the user's separate screenshots.
The option parser does not impose an undergraduate level range. Synthetic
400-level metadata tests are parser/contract coverage, not graduate live evidence.

## Credit trace

The 0.5 is an application allocation, not an official per-quarter value:
`declared_term_credits` and `earned_credits_by_term` divide a course's effective
credit total by its number of quarters. A 1-credit 1–2Q course therefore contributes
0.5 to each Q; a 6-credit 1–2Q course contributes 3 to each. No grade, stored
credit or allocation rule was changed in this checkpoint. The user was asked
whether quarter summaries should show full course credits or retain allocation.

## Validation

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
