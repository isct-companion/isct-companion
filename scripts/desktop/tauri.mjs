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
const version = development
  ? (process.env.ISCT_DEV_VERSION ?? developmentVersion(packageVersion))
  : undefined;
const shouldSignMac =
  process.platform === "darwin" && Boolean(process.env.APPLE_SIGNING_IDENTITY);
// A linker-only signature identifies the executable by its Rust target name.
// UserNotifications then cannot associate the debug .app with its bundle ID.
// Local bundles need an ad-hoc bundle signature, without distribution-only
// CloudKit/Keychain entitlements (those require a provisioning profile).
const shouldSignLocalMac =
  process.platform === "darwin" &&
  command.includes("build") &&
  command.includes("--debug") &&
  !shouldSignMac;
const macConfig = shouldSignMac
  ? macSigningConfig({ required: true })
  : shouldSignLocalMac
    ? {
        bundle: {
          macOS: {
            signingIdentity: "-",
            entitlements: null,
            hardenedRuntime: false,
          },
        },
      }
    : undefined;
// Azure Artifact Signing signs the app binary and the NSIS installer on
// Windows; without a trusted signature Smart App Control blocks the installer
// and Controlled folder access blocks writes to Documents. Credentials come
// from AZURE_* variables read by artifact-signing-cli itself.
const windowsSigningEndpoint =
  process.platform === "win32"
    ? process.env.AZURE_ARTIFACT_SIGNING_ENDPOINT
    : undefined;
const windowsConfig = windowsSigningEndpoint
  ? {
      windows: {
        signCommand: {
          cmd: "artifact-signing-cli",
          args: ["-e", windowsSigningEndpoint, "-d", "IsctCompanion", "%1"],
        },
      },
    }
  : undefined;
const tauriCommand = [...process.argv.slice(2)];
if (macConfig || windowsConfig || version) {
  // Keep Tauri options before the separator for additional Cargo arguments.
  const separator = tauriCommand.indexOf("--");
  tauriCommand.splice(
    separator < 0 ? tauriCommand.length : separator,
    0,
    "--config",
    JSON.stringify({
      ...macConfig,
      ...(windowsConfig ? { bundle: windowsConfig } : {}),
      ...(version ? { version } : {}),
    }),
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
