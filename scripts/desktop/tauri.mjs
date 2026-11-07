import { spawn, spawnSync } from "node:child_process";
import { existsSync, readFileSync, mkdirSync, copyFileSync } from "node:fs";
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
// Build the dedicated menu installer before Tauri collects bundle resources.
// It must also ship in dev builds: the Add button exercises the real installer.
const campusMenuPayload = process.platform === "win32" &&
  (command.includes("build") || command.includes("dev"));
if (campusMenuPayload) {
  const targetIndex = command.indexOf("--target");
  const target = targetIndex >= 0 ? command[targetIndex + 1] : undefined;
  const build = spawnSync("cargo", ["build", "--locked", "--release", "-p", "isct-campus-menu-setup",
    ...(target ? ["--target", target] : [])], { cwd: root, stdio: "inherit" });
  if (build.error || build.status !== 0) throw build.error ?? new Error("Campus menu installer build failed");
  const targetDir = path.resolve(root, process.env.CARGO_TARGET_DIR ?? "target");
  const helper = path.join(targetDir, ...(target ? [target] : []), "release/isct-campus-menu-setup.exe");
  const directory = path.join(root, "apps/desktop/src-tauri/resources/campus-print");
  mkdirSync(directory, { recursive: true });
  const bundledHelper = path.join(directory, "isct-campus-menu-setup.exe");
  copyFileSync(helper, bundledHelper);
  if (process.env.AZURE_ARTIFACT_SIGNING_ENDPOINT) {
    const signed = spawnSync("artifact-signing-cli", ["-e", process.env.AZURE_ARTIFACT_SIGNING_ENDPOINT,
      "-d", "IsctCompanion", bundledHelper], { stdio: "inherit" });
    if (signed.error || signed.status !== 0) throw signed.error ?? new Error("Campus menu installer signing failed");
  }
}
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
// The campus VPN payload (`bundle-vpn.mjs`) ships only when it was built:
// the helper, route hook and openconnect land in resources/vpn.
const vpnHelper = { darwin: "isct-vpn-helper", win32: "isct-vpn-helper.exe" }[process.platform];
const vpnPayload =
  vpnHelper !== undefined &&
  existsSync(path.join(root, "apps/desktop/src-tauri/resources/vpn", vpnHelper));
const vpnBundle = vpnPayload || campusMenuPayload
  ? {
      resources: [
        ...JSON.parse(
          readFileSync(path.join(root, "apps/desktop/src-tauri/tauri.conf.json"), "utf8"),
        ).bundle.resources,
        ...(vpnPayload ? ["resources/vpn/"] : []),
        ...(campusMenuPayload ? ["resources/campus-print/"] : []),
      ],
    }
  : undefined;
if (vpnPayload) console.log("Campus VPN payload: included.");
const tauriCommand = [...process.argv.slice(2)];
if (macConfig || windowsConfig || version || vpnBundle) {
  // Keep Tauri options before the separator for additional Cargo arguments.
  const separator = tauriCommand.indexOf("--");
  tauriCommand.splice(
    separator < 0 ? tauriCommand.length : separator,
    0,
    "--config",
    JSON.stringify({
      ...macConfig,
      ...(windowsConfig || vpnBundle
        ? { bundle: { ...macConfig?.bundle, ...windowsConfig, ...vpnBundle } }
        : {}),
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
