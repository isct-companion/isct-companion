import { copyFileSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const read = (name) => readFileSync(path.join(root, name), "utf8");
const version = JSON.parse(read("package.json")).version;
const targets = {
  "aarch64-apple-darwin": ["dmg", ".dmg"],
  "x86_64-apple-darwin": ["dmg", ".dmg"],
  "x86_64-pc-windows-msvc": ["nsis", ".exe"],
};

function validate() {
  if (!/^\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?$/.test(version)) throw new Error("Invalid release version");
  if (JSON.parse(read("backend/tauri.conf.json")).version !== version) throw new Error("Tauri/package version mismatch");
  const cargoVersion = /^version = "([^"]+)"/m.exec(read("backend/Cargo.toml"))?.[1];
  if (cargoVersion !== version) throw new Error("Cargo/package version mismatch");
  if (process.env.GITHUB_REF?.startsWith("refs/tags/") && process.env.GITHUB_REF !== `refs/tags/v${version}`) throw new Error("Release tag must match app version");
  for (const file of ["docs/legal/EULA.md", "docs/legal/EULA.ja.md"])
    if (/\[licensor|\[YYYY|\[contact|\[Confirm|［|<!--\s*DRAFT/i.test(read(file))) throw new Error(`Unfinished EULA: ${file}`);
}

validate();
if (process.argv[2] === "collect") {
  const target = process.argv[3];
  if (!targets[target]) throw new Error("Unsupported release target");
  const [format, extension] = targets[target];
  const bundle = path.join(root, "target", target, "release/bundle", format);
  const installers = readdirSync(bundle, { withFileTypes: true }).filter((entry) => entry.isFile() && entry.name.endsWith(extension));
  if (installers.length !== 1) throw new Error(`Expected exactly one ${format} installer, found ${installers.length}`);
  const output = path.join(root, "artifacts/desktop", target);
  mkdirSync(output, { recursive: true });
  copyFileSync(path.join(bundle, installers[0].name), path.join(output, `IsctCompanion_${version}_${target}${extension}`));
  for (const [source, destination] of [
    ["docs/distribution/README.md", "README.md"],
    ["docs/legal/EULA.md", "LICENSE"],
    ["docs/legal/EULA.ja.md", "LICENSE.ja.md"],
    ["dist/THIRD-PARTY-NOTICES.txt", "THIRD-PARTY-NOTICES.txt"],
  ]) copyFileSync(path.join(root, source), path.join(output, destination));
  writeFileSync(path.join(output, "VERSION"), `${version}\n`);
} else if (process.argv[2] !== "validate") throw new Error("Usage: desktop-release.mjs validate | collect <target>");
