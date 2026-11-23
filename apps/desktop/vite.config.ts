import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname } from "node:path";
import { defineConfig, normalizePath } from "vite";
import react from "@vitejs/plugin-react";
import { viteStaticCopy } from "vite-plugin-static-copy";
import { googleDriveClientId } from "../../scripts/lib/oauth-config.mjs";
import { thirdPartyNotices } from "../../scripts/desktop/third-party-notices.mjs";
import { interactionTrace } from "../../scripts/desktop/interaction-trace.mjs";
import { developmentVersion } from "../../scripts/lib/build-version.mjs";
import { hasCampusVpn } from "../../scripts/desktop/desktop-target.mjs";

const { version } = JSON.parse(
  readFileSync(new URL("./package.json", import.meta.url), "utf8"),
) as { version: string };

// PDF.js fetches these at runtime: CMaps for non-embedded CJK fonts, the
// standard 14 fonts, and the JBIG2/JPEG 2000/ICC decoders. PdfDocument.tsx
// points getDocument() at the copies under pdfjs/.
const pdfjsRoot = normalizePath(
  dirname(createRequire(import.meta.url).resolve("pdfjs-dist/package.json")),
);
const pdfjsData = [
  ...["cmaps", "standard_fonts", "iccs"].map((folder) => ({
    src: `${pdfjsRoot}/${folder}/*`,
    dest: `pdfjs/${folder}`,
  })),
  {
    // Only the image decoders and their notices. The reader does not run PDF scripts.
    src: `${pdfjsRoot}/wasm/{jbig2*,openjpeg*,qcms_bg.wasm,LICENSE*JBIG2,LICENSE*OPENJPEG,LICENSE*QCMS}`,
    dest: "pdfjs/wasm",
  },
].map((target) => ({ ...target, rename: { stripBase: true as const } }));

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
    "import.meta.env.ISCT_CAMPUS_VPN": JSON.stringify(
      !process.env.TAURI_ENV_TARGET_TRIPLE ||
        hasCampusVpn(process.env.TAURI_ENV_TARGET_TRIPLE),
    ),
    __APP_VERSION__: JSON.stringify(
      process.env.ISCT_APP_VERSION ??
        (command === "serve" ? developmentVersion(version) : version),
    ),
    __GOOGLE_DRIVE_CLIENT_ID__: JSON.stringify(googleDriveClientId("desktop")),
  },
}));
