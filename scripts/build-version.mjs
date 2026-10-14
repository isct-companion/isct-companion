import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));

export function developmentVersion(version) {
  let revision = "unknown";
  try {
    revision = `g${execFileSync("git", ["rev-parse", "--short=7", "HEAD"], {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim()}`;
    const changes = execFileSync("git", ["status", "--porcelain"], {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    });
    if (changes.trim()) revision += ".dirty";
  } catch {
    // Source archives have no Git metadata; the build time still identifies them.
  }
  const base = version.split("+")[0];
  const stamp = new Date().toISOString().replace(/\D/g, "").slice(0, 14);
  return `${base}${base.includes("-") ? "." : "-"}dev.${stamp}+${revision}`;
}
