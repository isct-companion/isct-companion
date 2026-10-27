# Design tokens

The shared visual language of every IsctCompanion surface: desktop, iOS,
Android and the values core needs (the course palette). Edit the tokens here,
never the generated files.

```bash
yarn design:tokens
```

## Layout

| Path | Content |
|---|---|
| `tokens/color.light.json`, `tokens/color.dark.json` | Neutrals, accent, ok / warn / danger pairs, shadow; one file per theme with the same names |
| `tokens/course.json` | Course palette (core picks the entry) and the two tint strengths |
| `tokens/shape.json` | Radius and spacing by role, the desktop type scale, desktop workspace metrics, font stack |
| `tokens/motion.json` | Three durations and the desktop easing curves |
| `platforms/mobile.json` | Phone values for the same roles (radius, gutter); never new names |
| `build.mjs` | Style Dictionary config and the few custom formats |

Tokens use the [W3C Design Tokens (DTCG) format](https://www.designtokens.org/tr/drafts/format/):
`$value`, `$type`, `$description`, groups by nesting.

## Outputs

| Output | Consumer |
|---|---|
| `apps/desktop/src/generated/design/tokens.css` | `styles.css` imports it; names are the public CSS variables (`--bg`, `--accent`, `--radius-chip`, …) that per-site CSS relies on |
| `apps/desktop/src/generated/design/tokens.dark.css` | Dark theme: chosen (`data-appearance="dark"`) or following the system |
| `crates/app-core/src/schedule/course_palette.rs` | `display_color` in core, included by `colors.rs` |
| `apps/ios/IsctCompanion/Design/Generated/IsctTokens*.swift` | SwiftUI shell (light constants + dark colours) |
| `apps/android/app/src/main/java/app/isct/design/IsctTokens*.kt` | Compose shell (light constants + dark colours) |

Generated files are committed so Cargo, Xcode and Gradle builds need no Node.
`yarn test` fails when one is stale (the "design tokens" suite in
`apps/desktop/tests/unit/tooling.test.mjs`).

## Rules

The visual rules themselves (block states, chips, buttons, motion, themes)
are decisions in slotledger (`approach/…`); where this file and slotledger
disagree, slotledger wins. The notes below only say how the tokens are meant.

- **Names are roles, not values**: `radius.chip`, not `radius.6`. A platform
  overrides a role's value in `platforms/`, never adds a name.
- **Course colour has two uses only**: a fill for something that occupies
  time (`course.tint.fill`, a timetable block) and a small square for "belongs
  to this course" in lists. A fill is the course colour mixed into the surface
  as a solid colour on every platform, and text on it is the course colour
  mixed into ink (`course.tint.ink`). An exam takes the deeper
  `course.tint.exam`; text on the solid course colour is `course.on-solid`.
- **One tint, meaning "now"**: the class in progress or next, or tonight's
  deadline, fills its whole card with a tint: the course (or danger) hue at the
  theme's `hero.lightness` with chroma capped at `hero.chroma` (OKLCH), so
  every course is equally quiet and dark mode gets its own tint instead of the
  light colours on black. It fades slightly from top to bottom, to `hero.fade`
  of the tint mixed into the surface in OKLab, and stays symmetric side to
  side: a corner gradient read as lopsided. A hairline at `course.tint.fill`
  outlines it. Desktop draws it on Today's focus row, iOS (`CourseTint`) and
  Android (`nowTint`) on the hero card. Nothing else is graded.
- **Icon tiles use roles, not hues**: a settings row's tile is accent
  (preferences), ok (data), warn or danger (caution) or muted (utilities);
  system rainbow colours do not appear.
- **System chrome is not tokenised**: tab bars, navigation bars, sheets,
  switches and system fonts on phones come from the platform. Tokens cover
  what we draw and the neutrals behind it.
- Changing a palette value changes every course's colour on every surface;
  core's tests pin the hash, not the colours.

## Desktop workspaces

The shared workspace treatment lives in
`apps/desktop/src/components/workspace-chrome.css`; `ResizableWorkspace` owns
column sizing and local layout persistence. Web and course readers share
`workspacePane`; readers head it with `workspaceToolPanel`, Web with its address toolbar.
Keep the outer panel rounder than its controls, use one quiet border, and reserve the accent for interaction.
Readers open one document at a time. PDF, file and Markdown tools render into
the fixed tool panel through `ReaderTools`; only the content scrolls. Web names
its service in the left picker only. The lesson progress rail stays above
the two columns, and search stays in the outline. A narrow pane wraps tools instead of clipping
operations. Compact layouts keep the same content mounted while showing one pane.

Change geometry in `tokens/shape.json` and regenerate; do not add page-specific
copies of the same radius or header dimensions.
