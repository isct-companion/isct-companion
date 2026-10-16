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
| `tokens/shape.json` | Radius by role, desktop font stack, Mantine's tonal accent scale |
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
`yarn test` fails when one is stale (`design/build.test.mjs`).

## Rules

- **Names are roles, not values**: `radius.chip`, not `radius.6`. A platform
  overrides a role's value in `platforms/`, never adds a name.
- **Course colour has two uses only**: a tint of the course colour on the
  surface for something that occupies time (`course.tint.fill`, timetable
  block) or emphasis on a row (`course.tint.soft`, the current or next class),
  and a small square for "belongs to this course" in lists.
- **System chrome is not tokenised**: tab bars, navigation bars, sheets,
  switches and system fonts on phones come from the platform. Tokens cover
  what we draw and the neutrals behind it.
- Changing a palette value changes every course's colour on every surface;
  core's tests pin the hash, not the colours.
