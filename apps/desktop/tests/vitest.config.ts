import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

const app = fileURLToPath(new URL("..", import.meta.url));
const tests = fileURLToPath(new URL(".", import.meta.url));
const repo = fileURLToPath(new URL("../../..", import.meta.url));

/**
 * One configuration for the whole desktop test tree.
 *
 * Suites live only under `apps/desktop/tests/`; production code is imported
 * through the `@app` alias so a suite can sit in a topic folder instead of
 * beside the module it exercises, and repository-level tooling through `@repo`.
 *
 * A suite that needs a DOM opts in with `// @vitest-environment jsdom` on its
 * first line. Everything else runs in plain node.
 */
export default defineConfig({
  root: app,
  plugins: [react()],
  resolve: {
    alias: { "@app": `${app}/src`, "@tests": tests, "@repo": repo },
  },
  define: {
    // vite.config.ts injects these at build time; tests need the same globals.
    __APP_VERSION__: JSON.stringify("test"),
    __GOOGLE_DRIVE_CLIENT_ID__: JSON.stringify(""),
  },
  test: {
    include: ["tests/**/*.test.{ts,tsx,mjs}"],
    setupFiles: [`${tests}/harness/setup.ts`],
    // Fixtures and the live school suite read files relative to the repository.
    env: { ISCT_REPO_ROOT: repo },
    // A mounting suite pays the shell's module transform once, which can take
    // tens of seconds on a loaded machine; the individual assertions are fast.
    testTimeout: 30_000,
    hookTimeout: 60_000,
  },
});
