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
| `tokens/shape.json` | Radius by role, desktop workspace metrics, font stack and Mantine's tonal accent scale |
| `platforms/mobile.json` | Phone values for the same roles (radius); never new names |
| `build.mjs` | Style Dictionary config and the few custom formats |

Tokens use the [W3C Design Tokens (DTCG) format](https://www.designtokens.org/tr/drafts/format/):
`$value`, `$type`, `$description`, groups by nesting.

## Outputs

| Output | Consumer |
|---|---|
| `apps/desktop/src/generated/design/tokens.css` | `styles.css` imports it; names are the public CSS variables (`--bg`, `--accent`, `--radius-chip`, …) that per-site CSS relies on |
| `apps/desktop/src/generated/design/tokens.dark.css` | Dark theme: chosen (`data-appearance="dark"`) or following the system |
| `apps/desktop/src/generated/design/tokens.json` | Mantine theme (`app/theme.ts`): tonal scale, font |
| `crates/isct-core/src/schedule/course_palette.rs` | `display_color` in core, included by `colors.rs` |
| `design/dist/ios/IsctTokens*.swift` | SwiftUI shell (light constants + dark colours) |
| `design/dist/android/IsctTokens*.kt` | Compose shell (light constants + dark colours) |

Generated files are committed so Cargo, Xcode and Gradle builds need no Node.
`yarn test` fails when one is stale (`apps/desktop/tests/unit/tooling.test.mjs`).

## Rules

- **Names are roles, not values**: `radius.chip`, not `radius.6`. A platform
  overrides a role's value in `platforms/`, never adds a name.
- **Course colour has two uses only**: a tint of the course colour on the
  surface for something that occupies time (`course.tint.fill`, timetable
  block) or emphasis on a row (`course.tint.soft`, the current or next class),
  and a small square for "belongs to this course" in lists. Text on a fill
  takes the course colour mixed into ink (`course.tint.ink`), so a block reads
  as one colour on desktop and phone.
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
