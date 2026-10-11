import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { nativeOAuthEnv, writeIosInfoPlist } from "./oauth-config.mjs";

const root = fileURLToPath(new URL("../", import.meta.url));
writeIosInfoPlist();
const child = spawn(
  process.execPath,
  [
    path.join(root, "node_modules/@tauri-apps/cli/tauri.js"),
    ...process.argv.slice(2),
  ],
  {
    cwd: root,
    stdio: "inherit",
    env: {
      ...process.env,
      ...nativeOAuthEnv(),
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
  if (signal) process.kill(process.pid, signal);
  else process.exitCode = code ?? 1;
});
