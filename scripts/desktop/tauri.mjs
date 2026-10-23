import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { nativeOAuthEnv } from "../lib/oauth-config.mjs";
import {
  macDevelopmentSigningConfig,
  macSigningConfig,
} from "./macos-signing.mjs";
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
// UserNotifications then cannot associate the debug .app with its bundle ID,
// so local debug bundles are always signed. With an Apple Development identity
// and a team profile for this Mac they get the release entitlements (iCloud
// works); otherwise an ad-hoc signature without them. `tauri dev` runs a bare
// executable, which cannot carry a provisioning profile, so it never has iCloud.
const shouldSignLocalMac =
  process.platform === "darwin" &&
  command.includes("build") &&
  command.includes("--debug") &&
  !shouldSignMac;
const macDevelopmentConfig = shouldSignLocalMac
  ? macDevelopmentSigningConfig()
  : undefined;
if (shouldSignLocalMac) {
  console.log(
    macDevelopmentConfig
      ? "Debug bundle: signed with your Apple Development profile; iCloud enabled."
      : "Debug bundle: ad-hoc signed; no Apple Development profile for this Mac, so iCloud is off.",
  );
}
const macConfig = shouldSignMac
  ? macSigningConfig({ required: true })
  : (macDevelopmentConfig ??
    (shouldSignLocalMac
      ? {
          bundle: {
            macOS: {
              signingIdentity: "-",
              entitlements: null,
              hardenedRuntime: false,
            },
          },
        }
      : undefined));
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
      // build.rs compiles iCloud into a macOS debug build only when it will be
      // signed with the entitlements; always set, so a stale value cannot leak.
      ISCT_APPLE_ENTITLED: shouldSignMac || macDevelopmentConfig ? "1" : "",
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
