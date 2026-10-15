import { readFileSync } from "node:fs";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import {
  googleDriveClientId,
  oauthClientTarget,
} from "../../scripts/lib/oauth-config.mjs";
import { thirdPartyNotices } from "../../scripts/desktop/third-party-notices.mjs";
import { developmentVersion } from "../../scripts/lib/build-version.mjs";

const { version } = JSON.parse(
  readFileSync(new URL("./package.json", import.meta.url), "utf8"),
) as { version: string };

export default defineConfig(({ command }) => ({
  plugins: [react(), thirdPartyNotices()],
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
