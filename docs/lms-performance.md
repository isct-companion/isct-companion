# LMS performance check — 2026-10-05

## Scope and limits

This check measures local section-summary parsing and production bundle output. It does not measure native macOS RSS, idle CPU, energy, PDF memory, authenticated LMS rendering, or native renderer lifetime. The sandbox rejected `ps` with “operation not permitted”. Consequently, no claim that IsctApp uses less memory than the official LMS is justified. The continuous-reader redesign, editor, and downloader are not yet included in these measurements.

## Reproduce

```sh
node --experimental-strip-types scripts/performance/lms-content.mjs
yarn test
yarn build
```

The parser benchmark runs in Node 22.23.0 with jsdom. Synthetic bodies contain bilingual text, lists, and attachment links. Each path receives one warm-up; the result is the median of seven batches. The previous path sanitizes and reconstructs all HTML before extracting a collapsed summary. The optimized path extracts bounded inert text directly; expanded rich HTML still uses the sanitizer. This is an indicative microbenchmark, not a browser load or retained-memory benchmark.

| Synthetic fixture | Previous batch median | Optimized batch median | Time reduction |
| --- | ---: | ---: | ---: |
| 14 sections × 2,860 characters | 66.68 ms | 28.40 ms | 57.4% |
| 52 sections × 7,150 characters | 440.55 ms | 188.52 ms | 57.2% |

## Changes

- Read persisted course reading state once per component mount instead of parsing session storage on every render.
- Defer rich-HTML sanitization until content is displayed; collapsed summaries and search extract text while excluding scripts, styles, frames, forms and embedded objects, with a 100,000-character input bound.
- Find the nearest reading section with one scan instead of allocating and sorting a second array per scroll event.
- Load the LMS workspace chunk only on opening a course. Main production JS fell from 850.15 kB to 836.18 kB; the deferred chunk is 15.57 kB. These are transfer sizes, not runtime memory. The app still has a large main bundle.

## Existing verified resource bounds

The workspace retains at most two course DOMs, five document-tab metadata entries per course, and eight recent reading-state records. An inactive file preview is unmounted, leaving zero preview iframes; reactivation recreates one. These bounds are checked by the workspace tests and do not establish a byte-level memory budget. The resident official LMS WebView is hidden and reused, not destroyed; hiding alone must not be reported as freeing its process memory.

All 164 frontend tests across 27 files passed; the production build passed.

## Native comparison still required

Use the installed desktop release and official LMS in the same WebKit environment, account, course, viewport and attachment set. Record application and associated renderer RSS/idle CPU separately, with a baseline before opening content. Measure idle Today, one course, one PDF, repeated course switching, hidden workspace, and notes editing when implemented. Report warm and cold results; verify whether retained memory settles after closing documents. Whole Codex browser process measurements would mix unrelated tabs and are unsuitable for attributing memory to this app.

Further heavy viewers and editors should load on demand and have only one active instance. User notes persist independently of cache eviction. Prefer recreating a document from local disk over retaining decoded PDF pages or inactive editor instances.

## Continuous reader implementation update

The course now reads as one continuous surface, with common text activities and assignment requirements inline. A viewer appears only on opening a document or local note. Accordingly, the summary microbenchmark above describes the previous folded-outline optimization; it is not a measurement of the current continuous reader's complete rendering time.

The workspace keeps up to two recent course DOMs for quick return. After 45 seconds it evicts background course DOMs; leaving Today for that interval evicts all course DOMs. Position and tab metadata remain bounded and can reconstruct the reader. Inactive documents are unmounted immediately, including when returning to the reader in a narrow viewport. The rich-content component is memoized so position highlights and note-save status do not rerender unchanged teacher HTML. Opening/closing the second column restores the section-relative reading anchor.

Production output for the implemented reader is approximately 841 kB main JS, 19 kB deferred workspace, 2 kB each for note/file UI, 1,358 kB deferred WYSIWYG Markdown editor/renderer (426 kB gzip, including KaTeX). The inline-editor update replaces ByteMD with modular Milkdown Crepe; no language packs, image uploader, AI integration or fixed top toolbar are enabled. This is larger than the former 610 kB editor plus 259 kB math chunk, and is loaded only when opening a Markdown note/attachment. Editor CSS and fonts are also separate from initial application CSS. These are build/transfer sizes, not heap measurements; JavaScript modules remain cached after loading even though editor and viewer instances are destroyed.

Native downloads are serialized with an async lock before allocating a blocking worker. File bytes stream to a temporary file, with a 128 MB bound, and are renamed only on complete success. Cookies remain in Rust and are used only for an HTTPS request to the LMS attachment host; redirects are rejected. The native asset protocol serves cached files directly, avoiding a base64 copy in JS. Only active UTF-8 text previews and notes cross IPC, bounded to 1 MB. Note writes use a separate lock so downloads do not block autosave. Listing notes returns metadata only, bounded to 1,000 notes per course.

Browser preview notes persist in browser storage under a separate preview course namespace. Desktop storage defaults to the OS Documents directory / Campus Life, with application storage fallback when Documents is unavailable. Mobile uses app storage and does not offer an unsupported folder picker. Desktop Settings / Advanced allows selecting a root. Materials and notes occupy separate child folders; changing the root does not move or delete previous files.

Browser QA verified source/editor split, formula rendering, autosave status, and reading continuity. Unit checks cover ordered note writes, reopening after pending save, save-failure recovery, idle DOM eviction, bounded document previews, and folder/URL validation. Authenticated native attachment downloads, native PDF rendering and OS folder selection still need runtime acceptance in the installed desktop app. Native RSS/CPU comparison with LMS remains unmeasured.


## Inline editing and proximity scrolling (2026-10-05)

- One WYSIWYG document surface; formulas, Markdown shortcuts, selection toolbar and slash/block menu. Only the active note is mounted. Read-only Markdown attachments do not install editing menus. No image upload integration.
- Imported Markdown is not written merely because it was loaded. Actual document changes are captured synchronously before the existing 800 ms disk-save debounce, preserving the last keystroke when a tab closes. Controlled echoes do not rebuild editors. Async startup/destroy uses separate mount nodes.
- Native `scroll-snap-type: y proximity` attracts nearby section starts, with a 22% reading inset and normal snap stops. Long sections remain freely scrollable. Saved-position restoration and explicit assignment navigation disable snap until the next manual wheel/touch/key scroll. Search and reduced-motion preference disable it. No animation loop or custom wheel interception.
- Editor integration tests use real ProseMirror transactions to verify formula/list/code roundtrip, no save on initial load, synchronous change capture, controlled echo stability and read-only external replacement. jsdom layout observers are stubbed; native memory remains unmeasured.

## Native file and announcement update (2026-10-05)

The native App verified attachment download and disk caching; the old embedded PDF iframe was blank and therefore replaced. PDF.js loads only when opening a PDF (deferred display module about 489 kB / 149 kB gzip, worker about 1.32 MB uncompressed). Only one page canvas is mounted, bounded to two million pixels, roughly 8 MB of raw RGBA pixels; this is a canvas bound, not total viewer RSS. Tab switch/unmount cancels rendering and destroys its worker and document buffers. No all-page rasterization or inactive PDF retention. Native RSS/CPU comparison against official LMS remains unmeasured and must not be represented as meeting a lower-memory target.

Announcement reads are capped at eight discussions per forum and sixteen per import, within the existing bounded HTML content budget. Short workstation entry/viewer/section and hover transitions have no background animation loop and respect reduced motion.

## Continuous PDF and opt-in storage follow-up

PDFs now scroll as a continuous document. Page placeholders retain reading geometry; IntersectionObserver mounts canvases only near the visible viewport (200 px margin) and cancels/free canvases on leaving. Each canvas remains bounded to two million pixels with at most 1.5× display scaling. PDF.js page resources clean up after rendering; the document/worker is destroyed on tab leave. This replaces the earlier single-page buttons. Total document buffers and fonts are additional to the canvas budget; no lower-than-LMS RSS claim has been established.

Automatically save course files is off by default. Temporary previews live in a per-process folder and normal exit removes it. Explicit saving uses native Save As. Turning the option on stores opened files in the configured course hierarchy; notes retain their existing explicit creation/autosave behavior. Account status is non-secret metadata, allowing resident session reuse without reading Keychain solely for a status check.

### Reader budget update (2026-10-05)

PDF.js remains lazy. Continuous pages allocate canvases only in/near the viewport (200 px overscan), capped at 2,000,000 pixels per active page and device scale 1.5; text layers mount with active pages and are cancelled/removed with them. Zoom does not lift the canvas pixel cap. PDF find requests text only when explicitly used and stops at a matching page. Text/code preview is lazy CodeMirror with line virtualization and a 1 MB UTF-8 input limit; the added text-reader chunk is about 219 kB raw / 81 kB gzip. Markdown retains its separate lazy editor chunk. These are component allocation and bundle limits, not a claim that total App/WebKit RSS is below the original LMS.

### Final section-note and offline reuse checks

Only one primary note entry per section is exposed; legacy duplicates remain recoverable rather than being destructively merged. Autosave/delete operations are serialized, and inactive editors remain unmounted. Opening and closing transitions run for at most 240 ms and have no polling or continuous animation.

With automatic storage disabled, the native reader first checks an existing validated persistent course file, then its per-process preview cache. The final native run reopened the existing OOP PDF and searched to page 6 without downloading it again. PDF text streams use explicit readers for native WebKit compatibility. Final automated frontend checks passed 188/188; total App versus official LMS RSS/CPU remains unmeasured.
