import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite";

/**
 * Builds the read-only Markdown page the Android and iOS apps bundle
 * (src/features/markdown-viewer) into resources/markdown-viewer:
 * markdown-viewer.html, .js and .css, flat, because both apps copy the folder's
 * files to the root of their assets. `yarn markdown-viewer` rebuilds it; the
 * output is committed so the app builds need no Node.
 *
 * The script is a classic one (iife), which a page loaded from a file URL can
 * run on iOS, and KaTeX's fonts are inlined as WOFF2 only, which every
 * supported WebView reads; the WOFF and TTF copies are dropped.
 */
// The staleness test builds into a scratch folder to compare with the commit.
const outDir =
  process.env.ISCT_MARKDOWN_VIEWER_OUT ??
  fileURLToPath(new URL("../../resources/markdown-viewer", import.meta.url));

// Images load only from the page itself (data:) and the exam site, whose
// shared notes link pictures it or its Firebase storage serves (Core's
// `exam_share::client::Endpoint::Site`): an image from anywhere else would
// tell its host who opened the document.
const images = [
  "data:",
  "https://sciencetokyoexam.com",
  "https://*.sciencetokyoexam.com",
  "https://firebasestorage.googleapis.com",
].join(" ");

const page = `<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <meta name="referrer" content="no-referrer" />
    <meta
      http-equiv="Content-Security-Policy"
      content="default-src 'none'; script-src 'self' file:; style-src 'self' file: 'unsafe-inline'; font-src data:; img-src ${images}"
    />
    <link rel="stylesheet" href="markdown-viewer.css" />
    <script src="markdown-viewer.js"></script>
  </head>
  <body>
    <article id="document"></article>
  </body>
</html>
`;

const woff2Only: Plugin = {
  name: "katex-woff2-only",
  enforce: "pre",
  transform(code, id) {
    if (!/katex(\.min)?\.css$/.test(id.split("?")[0])) return;
    return code.replace(
      /,\s*url\([^)]+\.(?:woff|ttf)\)\s*format\("(?:woff|truetype)"\)/g,
      "",
    );
  },
};

const html: Plugin = {
  name: "markdown-viewer-page",
  generateBundle() {
    this.emitFile({
      type: "asset",
      fileName: "markdown-viewer.html",
      source: page,
    });
  },
};

export default defineConfig({
  root: fileURLToPath(new URL(".", import.meta.url)),
  publicDir: false,
  // Production builds of every package (micromark ships a development one
  // with assertions), whatever NODE_ENV the build runs under.
  mode: "production",
  resolve: { conditions: ["module", "browser", "production"] },
  define: { "process.env.NODE_ENV": JSON.stringify("production") },
  plugins: [woff2Only, html],
  build: {
    outDir,
    emptyOutDir: true,
    assetsInlineLimit: Number.MAX_SAFE_INTEGER,
    cssCodeSplit: false,
    lib: {
      entry: "src/features/markdown-viewer/viewer.ts",
      name: "IsctMarkdownViewer",
      formats: ["iife"],
      fileName: () => "markdown-viewer.js",
      cssFileName: "markdown-viewer",
    },
  },
});
