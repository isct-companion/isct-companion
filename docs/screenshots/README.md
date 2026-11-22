# Screenshots

Website and README screenshots come from the **demo app**: a debug build with
the Cargo feature `demo` that shows one fictional student on Thursday
2026-07-23 at 10:20 JST, in 2026 2Q. Nothing in it comes from a school account,
so a capture never needs redacting.

The files here are the README hero images, `desktop-today.{en,ja}.png`, made
from the light captures and downscaled to 1600 px wide. `phone-today.{en,ja}.png`
are still the 2026-10-25 iPhone captures. The website (`isct-companion/Homepage`)
keeps the full-size captures in `screenshots/{ja,zh,en}/`.

## What the demo app is

- **Its own app.** `scripts/desktop/tauri.mjs --demo` builds the identifier
  `app.isctcompanion.demo` ("IsctCompanion Demo"), with its own data folder
  (`~/Library/Application Support/app.isctcompanion.demo/`, course files in
  `Courses/` inside it), keychain service, WebKit storage and signing
  identifier. `build.rs` refuses `demo` under any other identifier, and
  `app-core` refuses it in a release build.
- **One clock.** Every reading of the time in Core and the shells goes through
  `app_core::clock::now` (`clippy.toml` rejects `Utc::now`). In the demo it
  starts at the scene's moment and runs on; the desktop page gets the same
  offset as `__ISCT_COMPANION_CLOCK_OFFSET_MS__` (`wallNow()`).
- **The scene.** `crates/app-core/src/demo/scene.json` is written once into
  an empty store through the same ingest functions and intents a real sync
  uses (`app_core::demo::seed`): the course catalog, LMS enrolment, course
  outlines, deadlines and submission states, then term, admission year,
  department, course colours and weekly events, then the items Today lists as
  new. Exams, the academic calendar and department recommendations download
  from the public Isct-Data as usual.
- **No school network.** Both school accounts read as connected from an
  in-memory vault (`DemoVault`); sign-ins end offline before sending anything,
  the planner never schedules a school read, the school web refuses to open,
  cloud sync supports no provider, and the update check never runs. Past exams
  answer from the scene (`exam_share/demo.rs`); every document opens
  `sample-exam.pdf`, printed from `sample-exam.html`:
  `"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --no-pdf-header-footer --print-to-pdf=sample-exam.pdf sample-exam.html`.

Change the scene by editing `scene.json` (the payloads are Core's own:
`LmsEnrolment`, `LmsCourseContent`, `LmsDeadlines`, `WorkspaceIntent`,
`ExamSubjectPage`), then delete the demo data folder so the next launch seeds
it again. A seeded store is left as it is.

## Running it

```sh
# Iterate on code: hot reload, a bare executable (screen tools cannot address it).
CARGO_TARGET_DIR=/Users/xby/isct-companion/target yarn desktop:demo
# Capture: an unsigned debug bundle that macOS and screen tools treat as an app.
CARGO_TARGET_DIR=/Users/xby/isct-companion/target yarn desktop:build --debug --no-sign --demo --bundles app
open "$CARGO_TARGET_DIR/debug/bundle/macos/IsctCompanion Demo.app"
# Start the scene over.
rm -rf ~/Library/Application\ Support/app.isctcompanion.demo
```

Only one demo instance can hold the data folder; quit the dev run before
opening the bundle.

## Capturing

```sh
out=/tmp/isct-shots
scripts/screenshots/capture.sh "$out" light ja desktop-today
scripts/screenshots/contact-sheet.sh "$out" desktop-today desktop-lecture desktop-exams desktop-past-exams desktop-planner
```

`capture.sh` finds the window with `window-id.swift` (CoreGraphics window
list) and runs `screencapture -x -o -l<id>`: the 1280x820 window at 2x, so
2560x1640 with transparent rounded corners and no shadow. Since 1.4.0 each
column's toolbar shares the traffic lights' row, so the title bar is kept, not
cropped. The terminal (or the agent host) needs Screen Recording permission.

Setting up each shot, by hand or with a screen-control agent:

- **Clicks.** The page is a WKWebView: clicks sent to a background window do
  not reach it, so a driving agent needs full-screen control with the demo
  window in front. Move the pointer off the window before each capture.
- **Settings → Appearance** holds the interface language, the school content
  language, Light/Dark and "Show one language in bilingual LMS" (on for
  captures). For Chinese, set the school content language to English: Automatic
  follows Japanese there.
- **Screens** (2026-11 set, desktop 1.4.0):
  - `desktop-today`: Today, left column scrolled to the top.
  - `desktop-lecture`: Lectures → Lectures tab → Artificial Intelligence →
    lecture 11's PDF "探索アルゴリズム / Search algorithms".
  - `desktop-exams`: Lectures → Exams tab → Artificial Intelligence.
  - `desktop-past-exams`: Past exams → Artificial Intelligence → 2025 Final →
    Questions; collapse 中国語 if it opened.
  - `desktop-planner`: Timetable → Tuesday 1-2. A click on the slot toggles
    it: the candidates list shows the conditions Quarter, Department and Time
    with four courses when it is selected.

Light and dark go in separate folders; the website shows the dark set to
readers whose system is dark.

Before committing captures anywhere, look at every image: no real names,
student IDs, emails, grades or course material.
