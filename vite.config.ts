import { readFileSync } from "node:fs";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import {
  googleDriveClientId,
  oauthClientTarget,
} from "./scripts/oauth-config.mjs";
import { thirdPartyNotices } from "./scripts/third-party-notices.mjs";

const { version } = JSON.parse(
  readFileSync(new URL("./package.json", import.meta.url), "utf8"),
) as { version: string };

export default defineConfig({
  plugins: [react(), thirdPartyNotices()],
  base: "./",
  build: {
    outDir: "dist",
  },
  server: {
    port: 4173,
    watch: {
      ignored: ["**/backend/target/**"],
    },
  },
  define: {
    __APP_VERSION__: JSON.stringify(version),
    __GOOGLE_DRIVE_CLIENT_ID__: JSON.stringify(
      googleDriveClientId(oauthClientTarget(process.env.ISCT_BUILD_TARGET)),
    ),
  },
});
