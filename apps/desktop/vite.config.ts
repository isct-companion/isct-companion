import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname } from "node:path";
import { defineConfig, normalizePath } from "vite";
import react from "@vitejs/plugin-react";
import { viteStaticCopy } from "vite-plugin-static-copy";
import {
  googleDriveClientId,
  oauthClientTarget,
} from "../../scripts/lib/oauth-config.mjs";
import { thirdPartyNotices } from "../../scripts/desktop/third-party-notices.mjs";
import { interactionTrace } from "../../scripts/desktop/interaction-trace.mjs";
import { developmentVersion } from "../../scripts/lib/build-version.mjs";

const { version } = JSON.parse(
  readFileSync(new URL("./package.json", import.meta.url), "utf8"),
) as { version: string };

// PDF.js fetches these at runtime: CMaps for non-embedded CJK fonts, the
// standard 14 fonts, and the JBIG2/JPEG 2000/ICC decoders. PdfDocument.tsx
// points getDocument() at the copies under pdfjs/.
const pdfjsRoot = normalizePath(
  dirname(createRequire(import.meta.url).resolve("pdfjs-dist/package.json")),
);
const pdfjsData = ["cmaps", "standard_fonts", "wasm", "iccs"].map((folder) => ({
  src: `${pdfjsRoot}/${folder}/*`,
  dest: `pdfjs/${folder}`,
  rename: { stripBase: true as const },
}));

export default defineConfig(({ command }) => ({
  plugins: [
    react(),
    viteStaticCopy({ targets: pdfjsData }),
    thirdPartyNotices(),
    interactionTrace(),
  ],
  base: "./",
  build: {
    outDir: "dist",
  },
  server: {
    port: 4173,
    watch: {
      ignored: ["**/target/**"],
    },
  },
  define: {
    __APP_VERSION__: JSON.stringify(
      process.env.ISCT_APP_VERSION ??
        (command === "serve" ? developmentVersion(version) : version),
    ),
    __GOOGLE_DRIVE_CLIENT_ID__: JSON.stringify(
      googleDriveClientId(oauthClientTarget(process.env.ISCT_BUILD_TARGET)),
    ),
  },
}));
