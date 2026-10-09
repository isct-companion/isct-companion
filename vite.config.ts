import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
export default defineConfig({
  plugins: [react()],
  base: "./",
  build: { outDir: "dist" },
  server: {
    port: 4173,
    watch: {
      ignored: ["**/backend/target/**"],
    },
  },
});
