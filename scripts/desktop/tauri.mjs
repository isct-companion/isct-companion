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
  process.platform === "darwin" &&
  command[0] === "build" &&
  Boolean(process.env.APPLE_SIGNING_IDENTITY);
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
// Development builds (`dev`, and a debug bundle without the team profile) get
// their own identifier and productName, so their data folder, single-instance
// lock and credential service are not the release app's. build.rs reads the
// identifier back to compile out what would touch the release app's state. A
// debug bundle signed with the team profile tests iCloud as the release app.
const isolated =
  command[0] === "dev" ||
  (command.includes("--debug") && !shouldSignMac && !macDevelopmentConfig);
const host = isolated ? "app.isctcompanion.dev" : "app.isctcompanion";
const isolatedApp = isolated
  ? { identifier: host, productName: "IsctCompanion Dev" }
  : undefined;
if (isolated) console.log(`Development app: ${host}, separate from the release app.`);
const macSigning = shouldSignMac
  ? (() => {
      const signed = macSigningConfig({ required: true });
      signed.bundle.macOS.signingIdentity = process.env.APPLE_SIGNING_IDENTITY;
      return signed;
    })()
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
const macConfig =
  process.platform === "darwin" &&
  command[0] === "build" &&
  !command.includes("--no-bundle") &&
  !command.includes("--no-sign")
    ? macWidgets(root, command, macSigning, packageVersion, host)
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
if (vpnPayload) console.log("Campus VPN payload: included.");
// launchd records the helper's code requirement when the daemon is
// registered. An ad hoc helper is pinned to that one binary, so after a local
// rebuild launchd refuses the new helper (EX_CONFIG, "needs LWCR update") and
// the old one keeps answering. A local bundle signed with the person's Apple
// Development identity signs the helper with it too, as a release signs it
// with the Developer ID (bundle-vpn.mjs); Tauri signs nothing under Resources.
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
const devSigningIdentity =
  process.platform === "darwin" && command[0] === "dev"
    ? macDevSigningIdentity()
    : undefined;
if (devSigningIdentity) {
  const runner = path.join(root, "scripts/desktop/macos-dev-runner.sh");
  const separator = tauriCommand.indexOf("--");
  // Arguments after `--` go to Cargo; an array keeps spaces in the path.
  const runnerConfig = ["aarch64-apple-darwin", "x86_64-apple-darwin"].flatMap(
    (target) => ["--config", `target.${target}.runner=${JSON.stringify([runner])}`],
  );
  if (separator < 0) tauriCommand.push("--", ...runnerConfig);
  else tauriCommand.splice(separator + 1, 0, ...runnerConfig);
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
