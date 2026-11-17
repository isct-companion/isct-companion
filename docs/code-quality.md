# Code quality: reuse and cohesion

What a change, and a code quality review, checks beyond "does it work".
[AGENTS.md](../AGENTS.md) states the project rules; this page spells out how
to apply them to design tokens, shared UI and where logic lives.

## 1. Design tokens

Tokens are defined once in [`design/tokens`](../design/README.md) (W3C
[Design Tokens Format 2025.10](https://www.w3.org/community/reports/design-tokens/CG-FINAL-format-20251028/))
and generated for desktop CSS, Swift, Kotlin and Core. Edit the token files,
never the generated outputs.

- **Components use role tokens, not raw values.** Colour, radius, spacing,
  font size, shadow and motion come from a token (`var(--accent)`,
  `var(--radius-control)`, `calc(var(--type-row-size) * var(--text))`). A raw
  hex, `rgb()`, `px` radius or font size in component CSS, `style={{}}`,
  SwiftUI or Compose code is a finding.
- **Allowed raw values:** `0`, `50%` and `999px` for a circle or pill,
  `1px` hairlines, `inherit` / `currentColor`, and values that are geometry
  rather than style (an icon's viewBox, a measured layout size).
- **No feature-local token sets that restate global roles.** A
  `--lms-radius-sm` that means the same as `--radius-control` is replaced by
  the global token. A feature may keep a local custom property only for a
  value that exists nowhere else, and it is derived from global tokens
  (`--row-hover: color-mix(in srgb, var(--accent) 8%, transparent)`), not
  from literals.
- **A new role goes into `design/tokens`, for every platform.** If a value is
  needed and no token fits, add a role token with a `$description`, rebuild
  with `yarn design:tokens`, and use it on all shells. Do not add a second
  name for an existing role, and do not add a token for a one-off.
- **Spacing and sizes follow the scale.** Use the spacing and workspace
  tokens; a new value between two steps of the scale is a finding unless it
  is justified in a comment.
- **Dark mode and themes come from the token files.** A component never
  branches on the theme to pick a colour.

## 2. Shared UI

- One implementation per UI pattern (chip, button, card, empty state,
  confirm dialog, form row). Before writing markup and CSS for one, look in
  `apps/desktop/src/components/ui`; extend the existing component rather
  than restyling a copy.
- Radix and the other existing libraries come first (AGENTS.md §1).

## 3. Where logic lives

The project rules already decide most of this: domain decisions in Core
(AGENTS.md §4), one interface for shared behaviour (§2), native differences at
compile time (§3). Review for the two smells that break them
([Fowler's change preventers](https://refactoring.guru/smells/shotgun-surgery)):

- **Shotgun surgery — one concept, many places.** Changing one rule
  (a deadline, a status, a validation, a label mapping) needs edits in
  several files, or in desktop and iOS and Android. Move it to one owner,
  usually Core, and let the others call it.
- **Divergent change — one file, many reasons.** A file edited for unrelated
  features (a large page, the bridge, `App.tsx`, `styles.css`) is split along
  the reasons it changes, so each part has one owner.
- **Shells do not decide.** TypeScript, Swift and Kotlin that compute what is
  due, what is valid or what to show next duplicate Core. Move the decision
  to Core and render its result.
- **Repeated boundary code goes through one helper.** For example, every
  bridge command validates its native result the same way, through a shared
  validator, not a hand-written check per command.

## 4. When not to merge duplicates

Prefer duplication over the wrong abstraction
([AHA](https://kentcdodds.com/blog/aha-programming)). Merge two pieces of code
only when they are the same concept and must change together, not because
they look alike. Two copies that would diverge on the next product change
stay separate. An abstraction that grows flags and special cases for each
caller is a finding too; inline it back.

## 5. Review checklist

1. Raw colour, radius, spacing or font size outside `design/tokens`?
2. A feature-local token or component that restates a global one?
3. The same rule implemented in more than one file or shell?
4. A shell making a decision Core should own?
5. A file that changes for unrelated reasons?
6. Unused files, exports or dependencies left behind?
7. Is each merge of duplicates the same concept (§4)?

A finding names the place, the rule above it breaks and the owner it should
move to.
