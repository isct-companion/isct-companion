import { spawn, spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { nativeOAuthEnv } from "../lib/oauth-config.mjs";
import { macosBuildSigning } from "./macos-signing.mjs";
import { macWidgets } from "./macos-widgets.mjs";
import { developmentVersion } from "../lib/build-version.mjs";
import { campusVpnBundle } from "./bundle-vpn.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
// `--demo` is ours, not Tauri's: the website screenshot scene (Cargo feature
// `demo`) as its own app with its own data and keychain, from `dev` or as a
// debug bundle (`build --debug --no-sign`) that screen tools can address.
const demo = process.argv.includes("--demo");
const command = process.argv.slice(2).filter((arg) => arg !== "--demo");
const cargoSeparator = command.indexOf("--");
const flags =
  cargoSeparator === -1 ? command : command.slice(0, cargoSeparator);
const dev = command[0] === "dev";
const debug = flags.includes("--debug") || flags.includes("-d");
if (demo && !dev && !(command[0] === "build" && debug && flags.includes("--no-sign"))) {
  console.error("--demo runs only with `dev` or `build --debug --no-sign`.");
  process.exit(1);
}
const packageVersion = JSON.parse(
  readFileSync(path.join(root, "package.json"), "utf8"),
).version;
const version =
  dev || debug
    ? (process.env.ISCT_DEV_VERSION ?? developmentVersion(packageVersion))
    : undefined;
const bundleCommand = command[0] === "build" || command[0] === "bundle";
const noSign = flags.includes("--no-sign");
const packagesApp = bundleCommand && !flags.includes("--no-bundle");
const macOS = process.platform === "darwin";
const runMacDev = macOS && dev;
const signing =
  macOS && (dev || bundleCommand)
    ? macosBuildSigning({ dev, debug, packagesApp, noSign })
    : {};
const isolated = dev || (debug && !signing.entitled);
const host = demo
  ? "app.isctcompanion.demo"
  : isolated
    ? "app.isctcompanion.dev"
    : "app.isctcompanion";
const isolatedApp = isolated
  ? {
      identifier: host,
      productName: demo ? "IsctCompanion Demo" : "IsctCompanion Dev",
    }
  : undefined;
if (isolated)
  console.log(`Development app: ${host}, separate from the release app.`);
const macConfig =
  macOS && packagesApp && !noSign
    ? macWidgets(root, flags, signing.macOS, {
        version: packageVersion,
        host,
        debug,
      })
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
// The demo app has no VPN tool, so its bundle neither carries nor rebuilds
// the helper the release app installs.
const vpnBundle = demo
  ? undefined
  : campusVpnBundle({
      macOS: macConfig?.bundle.macOS,
      packagesApp,
      signing,
    });
const tauriCommand = [...command];
if (demo) tauriCommand.splice(1, 0, "--features", "demo");
if (macConfig || windowsConfig || version || vpnBundle || isolatedApp) {
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
      ...isolatedApp,
    }),
  );
}
// `tauri dev` runs the bare executable Cargo built. On macOS it is signed
// with the developer's own Apple Development identity before each launch
// (macos-dev-runner.sh), so its keychain items stay readable across rebuilds.
if (runMacDev) {
  const runner = path.join(root, "scripts/desktop/macos-dev-runner.sh");
  const separator = tauriCommand.indexOf("--");
  // Arguments after `--` go to Cargo; an array keeps spaces in the path.
  const runnerConfig = ["aarch64-apple-darwin", "x86_64-apple-darwin"].flatMap(
    (target) => [
      "--config",
      `target.${target}.runner=${JSON.stringify([runner])}`,
    ],
  );
  if (separator < 0) tauriCommand.push("--", ...runnerConfig);
  else tauriCommand.splice(separator + 1, 0, ...runnerConfig);
}
if (version) console.log(`Development build: ${version}`);
const childEnv = { ...process.env, ...nativeOAuthEnv() };
if (macOS) {
  // Tauri checks this variable before its bundle config, even if it is empty.
  delete childEnv.APPLE_SIGNING_IDENTITY;
  if (signing.releaseIdentity)
    childEnv.APPLE_SIGNING_IDENTITY = signing.releaseIdentity;
}
const child = spawn(
  process.execPath,
  [path.join(root, "node_modules/@tauri-apps/cli/tauri.js"), ...tauriCommand],
  {
    cwd: path.join(root, "apps/desktop"),
    stdio: "inherit",
    env: {
      ...childEnv,
      // Always set, so a stale exported value cannot reach a release frontend.
      ISCT_APP_VERSION: version ?? packageVersion,
      // build.rs compiles iCloud into a macOS debug build only when it will be
      // signed with the entitlements; always set, so a stale value cannot leak.
      ISCT_APPLE_ENTITLED: signing.entitled ? "1" : "",
      ISCT_MACOS_DEV_SIGNING_IDENTITY: signing.developmentIdentity ?? "",
      ISCT_MACOS_DEV_IDENTIFIER: host,
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
  // The demo bundle answers no URL scheme: src-tauri/Info.plist registers the
  // release app's `isctcompanion`, and a release widget's link must never
  // open the screenshot app. The bundle is unsigned, so its plist can change.
  if (demo && bundleCommand && macOS && code === 0) {
    const target = process.env.CARGO_TARGET_DIR ?? path.join(root, "target");
    const plist = path.join(
      target,
      "debug/bundle/macos/IsctCompanion Demo.app/Contents/Info.plist",
    );
    const removed = spawnSync(
      "/usr/libexec/PlistBuddy",
      ["-c", "Delete :CFBundleURLTypes", plist],
      { stdio: "inherit" },
    );
    if (removed.status !== 0) process.exitCode = 1;
  }
});
