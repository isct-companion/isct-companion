import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import {
  googleDriveClientId,
  oauthClientTarget,
} from "./scripts/oauth-config.mjs";

export default defineConfig({
  plugins: [react()],
  base: "./",
  build: { outDir: "dist" },
  server: { port: 4173 },
  define: {
    __GOOGLE_DRIVE_CLIENT_ID__: JSON.stringify(
      googleDriveClientId(oauthClientTarget()),
    ),
  },
});
