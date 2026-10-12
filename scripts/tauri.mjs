import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
import {
  nativeOAuthEnv,
  oauthClientTarget,
  writeIosInfoPlist,
} from "./oauth-config.mjs";
import { patchIosProject } from "./patch-ios-project.mjs";
import { macSigningConfig } from "./macos-signing.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
const command = process.argv.slice(2);
const commandPlatform = command.includes("ios")
  ? "ios"
  : command.includes("android")
    ? "android"
    : undefined;
const buildTarget = oauthClientTarget(commandPlatform);
writeIosInfoPlist();
if (buildTarget === "ios") patchIosProject({ required: false });
const shouldSignMac =
  buildTarget === "desktop" &&
  process.platform === "darwin" &&
  Boolean(process.env.APPLE_SIGNING_IDENTITY);
const macConfig = shouldSignMac
  ? macSigningConfig({ required: true })
  : undefined;
const tauriCommand = [...process.argv.slice(2)];
if (macConfig) tauriCommand.push("--config", JSON.stringify(macConfig));
const child = spawn(
  process.execPath,
  [
    path.join(root, "node_modules/@tauri-apps/cli/tauri.js"),
    ...tauriCommand,
  ],
  {
    cwd: root,
    stdio: "inherit",
    env: {
      ...process.env,
      ...(buildTarget === "desktop" ? nativeOAuthEnv() : {}),
      ISCT_BUILD_TARGET: buildTarget,
      TAURI_APP_PATH: path.join(root, "backend"),
      TAURI_FRONTEND_PATH: root,
    },
  },
);
child.on("error", (error) => {
  console.error(error.message);
  process.exitCode = 1;
});
child.on("exit", (code, signal) => {
  if (code === 0 && buildTarget === "ios" && command.includes("init")) {
    patchIosProject();
  }
  if (signal) process.kill(process.pid, signal);
  else process.exitCode = code ?? 1;
});
