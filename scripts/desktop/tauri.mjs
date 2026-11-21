import { execFileSync, spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { nativeOAuthEnv } from "../lib/oauth-config.mjs";
import {
  macDevelopmentSigningConfig,
  macSigningConfig,
} from "./macos-signing.mjs";
import { macWidgets } from "./macos-widgets.mjs";
import { macDevSigningIdentity } from "./macos-dev-signing.mjs";
import { developmentVersion } from "../lib/build-version.mjs";
import { refreshHelper } from "./bundle-vpn.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
const command = process.argv.slice(2);
const cargoSeparator = command.indexOf("--");
const flags =
  cargoSeparator === -1 ? command : command.slice(0, cargoSeparator);
const dev = command[0] === "dev";
const debug = flags.includes("--debug") || flags.includes("-d");
const packageVersion = JSON.parse(
  readFileSync(path.join(root, "package.json"), "utf8"),
).version;
const version =
  dev || debug
    ? (process.env.ISCT_DEV_VERSION ?? developmentVersion(packageVersion))
    : undefined;
const buildMacBundle =
  process.platform === "darwin" &&
  command[0] === "build" &&
  !flags.includes("--no-bundle") &&
  !flags.includes("--no-sign");
const releaseSigningIdentity = process.env.APPLE_SIGNING_IDENTITY?.trim();
const signMacRelease = buildMacBundle && Boolean(releaseSigningIdentity);
const macDevelopmentConfig =
  buildMacBundle && debug && !signMacRelease
    ? macDevelopmentSigningConfig()
    : undefined;
// A team-signed debug bundle tests the release app. All other development
// runs use separate data, instance locks and credentials.
const isolated = dev || (debug && !signMacRelease && !macDevelopmentConfig);
const signMacDevBundle = buildMacBundle && isolated;
const runMacDev = process.platform === "darwin" && dev;
// Both development entry points use the same code identity for Keychain.
const devSigningIdentity =
  runMacDev || signMacDevBundle ? macDevSigningIdentity() : undefined;
if (macDevelopmentConfig || signMacDevBundle) {
  console.log(
    macDevelopmentConfig
      ? "Debug bundle: signed with your Apple Development profile; iCloud enabled."
      : "Debug bundle: signed with your Apple Development certificate; iCloud is off.",
  );
}
const host = isolated ? "app.isctcompanion.dev" : "app.isctcompanion";
const isolatedApp = isolated
  ? { identifier: host, productName: "IsctCompanion Dev" }
  : undefined;
if (isolated)
  console.log(`Development app: ${host}, separate from the release app.`);
const macSigning = signMacRelease
  ? (() => {
      const signed = macSigningConfig({ required: true });
      signed.bundle.macOS.signingIdentity = releaseSigningIdentity;
      return signed;
    })()
  : (macDevelopmentConfig ??
    (signMacDevBundle
      ? {
          bundle: {
            macOS: {
              signingIdentity: devSigningIdentity,
              hardenedRuntime: false,
            },
          },
        }
      : undefined));
const macConfig = buildMacBundle
  ? macWidgets(root, flags, macSigning, {
      version: packageVersion,
      host,
      debug,
    })
  : macSigning;
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
const vpnHelper = { darwin: "isct-vpn-helper", win32: "isct-vpn-helper.exe" }[
  process.platform
];
const vpnPayload =
  vpnHelper !== undefined &&
  existsSync(
    path.join(root, "apps/desktop/src-tauri/resources/vpn", vpnHelper),
  );
// `--config` is merged last and replaces the resource list, so extend the one
// this platform bundles: tauri.windows.conf.json's map on Windows (it adds
// LiteRT's DLL beside the executable), tauri.conf.json's list elsewhere.
const platformResources = () => {
  const config = (name) => path.join(root, "apps/desktop/src-tauri", name);
  const windows = config("tauri.windows.conf.json");
  const file =
    process.platform === "win32" && existsSync(windows)
      ? windows
      : config("tauri.conf.json");
  return JSON.parse(readFileSync(file, "utf8")).bundle.resources;
};
const withVpn = (resources) =>
  Array.isArray(resources)
    ? [...resources, "resources/vpn/"]
    : { ...resources, "resources/vpn/": "resources/vpn/" };
// On macOS the helper is an SMAppService daemon, whose plist must sit in
// Contents/Library/LaunchDaemons; it runs the helper from resources/vpn.
const vpnDaemon =
  process.platform === "darwin"
    ? {
        macOS: {
          ...macConfig?.bundle?.macOS,
          files: {
            ...macConfig?.bundle?.macOS?.files,
            "Library/LaunchDaemons/app.isctcompanion.vpn.plist":
              "../../../crates/vpn-helper/assets/app.isctcompanion.vpn.plist",
          },
        },
      }
    : undefined;
const vpnBundle = vpnPayload
  ? { resources: withVpn(platformResources()), ...vpnDaemon }
  : undefined;
if (vpnPayload && command[0] === "build") refreshHelper();
if (vpnPayload) console.log("Campus VPN payload: included.");
// launchd records the helper's code requirement when the daemon is
// registered. An ad hoc helper is pinned to that one binary, so after a local
// rebuild launchd refuses the new helper (EX_CONFIG, "needs LWCR update") and
// the old one keeps answering. A local bundle signed with the person's Apple
// Development identity signs the helper with it too, as a release signs it
// with the Developer ID (bundle-vpn.mjs); Tauri signs nothing under Resources.
// The registration is also bound to the copy of the app that made it, which
// this signature does not change: the app registers afresh from its own copy
// on every install (campus_vpn/macos.rs `run_install`).
const helperIdentity =
  vpnPayload && process.platform === "darwin"
    ? macDevelopmentConfig?.bundle.macOS.signingIdentity
    : undefined;
if (helperIdentity) {
  execFileSync(
    "codesign",
    [
      "--force",
      "--sign",
      helperIdentity,
      path.join(root, "apps/desktop/src-tauri/resources/vpn", vpnHelper),
    ],
    { stdio: "inherit" },
  );
  console.log("Campus VPN helper: signed with your Apple Development identity.");
}
const tauriCommand = [...process.argv.slice(2)];
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
if (process.platform === "darwin" && !releaseSigningIdentity) {
  // Tauri checks this variable before its bundle config, even if it is empty.
  delete childEnv.APPLE_SIGNING_IDENTITY;
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
      ISCT_APPLE_ENTITLED: signMacRelease || macDevelopmentConfig ? "1" : "",
      ISCT_MACOS_DEV_SIGNING_IDENTITY: devSigningIdentity ?? "",
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
