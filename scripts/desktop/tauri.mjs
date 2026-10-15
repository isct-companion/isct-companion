import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { nativeOAuthEnv } from "../lib/oauth-config.mjs";
import { macSigningConfig } from "./macos-signing.mjs";
import { developmentVersion } from "../lib/build-version.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
const command = process.argv.slice(2);
const development = command.includes("dev") || command.includes("--debug");
const packageVersion = JSON.parse(
  readFileSync(path.join(root, "package.json"), "utf8"),
).version;
const version = development ? developmentVersion(packageVersion) : undefined;
const buildTarget = "desktop";
const shouldSignMac =
  process.platform === "darwin" &&
  Boolean(process.env.APPLE_SIGNING_IDENTITY);
const macConfig = shouldSignMac
  ? macSigningConfig({ required: true })
  : undefined;
const tauriCommand = [...process.argv.slice(2)];
if (macConfig || version) {
  // Keep Tauri options before the separator for additional Cargo arguments.
  const separator = tauriCommand.indexOf("--");
  tauriCommand.splice(
    separator < 0 ? tauriCommand.length : separator,
    0,
    "--config",
    JSON.stringify({ ...macConfig, ...(version ? { version } : {}) }),
  );
}
if (version) console.log(`Development build: ${version}`);
const child = spawn(
  process.execPath,
  [path.join(root, "node_modules/@tauri-apps/cli/tauri.js"), ...tauriCommand],
  {
    cwd: path.join(root, "apps/desktop"),
    stdio: "inherit",
    env: {
      ...process.env,
      ...nativeOAuthEnv(),
      // Always set, so a stale exported value cannot reach a release frontend.
      ISCT_APP_VERSION: version ?? packageVersion,
      ISCT_BUILD_TARGET: buildTarget,
      TAURI_APP_PATH: path.join(root, "apps/desktop/src-tauri"),
      TAURI_FRONTEND_PATH: path.join(root, "apps/desktop"),
    },
  },
);
child.on("error", (error) => {
  console.error(error.message);
  process.exitCode = 1;
});
child.on("exit", (code, signal) => {
  if (signal) process.kill(process.pid, signal);
  else process.exitCode = code ?? 1;
});
