import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { RELEASE_VERSION, releaseVersion } from "../release/versions.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
const read = (name) => readFileSync(path.join(root, name), "utf8");
const version = releaseVersion(root);
const targets = {
  "aarch64-apple-darwin": ["dmg", ".dmg"],
  "x86_64-apple-darwin": ["dmg", ".dmg"],
  "x86_64-pc-windows-msvc": ["nsis", ".exe"],
};

function validate() {
  if (!RELEASE_VERSION.test(version))
    throw new Error("Invalid release version");
  if (
    process.env.GITHUB_REF?.startsWith("refs/tags/") &&
    process.env.GITHUB_REF !== `refs/tags/v${version}`
  )
    throw new Error("Release tag must match app version");
  if (
    process.env.GITHUB_REF?.startsWith("refs/tags/") &&
    !existsSync(path.join(root, "docs/releases", `${version}.md`))
  )
    throw new Error("Add versioned release notes before pushing a release tag");
  for (const file of ["docs/legal/EULA.md", "docs/legal/EULA.ja.md"])
    if (
      /\[licensor|\[YYYY|\[contact|\[Confirm|［|<!--\s*DRAFT/i.test(read(file))
    )
      throw new Error(`Unfinished EULA: ${file}`);
}

validate();
if (process.argv[2] === "collect") {
  const target = process.argv[3];
  if (!targets[target]) throw new Error("Unsupported release target");
  const [format, extension] = targets[target];
  const bundle = path.join(root, "target", target, "release/bundle", format);
  const installers = readdirSync(bundle, { withFileTypes: true }).filter(
    (entry) => entry.isFile() && entry.name.endsWith(extension),
  );
  if (installers.length !== 1)
    throw new Error(
      `Expected exactly one ${format} installer, found ${installers.length}`,
    );
  const output = path.join(root, "artifacts/desktop", target);
  mkdirSync(output, { recursive: true });
  copyFileSync(
    path.join(bundle, installers[0].name),
    path.join(output, `IsctCompanion_${version}_${target}${extension}`),
  );
  // Tauri emits detached signatures only when an update signing key is configured.
  const updaterExtension = target.includes("apple") ? ".app.tar.gz" : ".exe";
  const updaterDirectory = target.includes("apple")
    ? path.join(root, "target", target, "release/bundle/macos")
    : bundle;
  const updates = (
    existsSync(updaterDirectory) ? readdirSync(updaterDirectory) : []
  ).filter(
    (name) =>
      name.endsWith(updaterExtension) &&
      existsSync(path.join(updaterDirectory, `${name}.sig`)),
  );
  if (updates.length > 1)
    throw new Error("Expected at most one signed update bundle");
  if (process.env.ISCT_UPDATER_PUBLIC_KEY && updates.length !== 1)
    throw new Error("Update-enabled build is missing its signed bundle");
  if (updates[0]) {
    const destination = `IsctCompanion_${version}_${target}${updaterExtension}`;
    copyFileSync(
      path.join(updaterDirectory, updates[0]),
      path.join(output, destination),
    );
    copyFileSync(
      path.join(updaterDirectory, `${updates[0]}.sig`),
      path.join(output, `${destination}.sig`),
    );
  }
  for (const [source, destination] of [
    ["docs/distribution/README.md", "README.md"],
    ["docs/legal/EULA.md", "LICENSE"],
    ["docs/legal/EULA.ja.md", "LICENSE.ja.md"],
    ["apps/desktop/dist/THIRD-PARTY-NOTICES.txt", "THIRD-PARTY-NOTICES.txt"],
  ])
    copyFileSync(path.join(root, source), path.join(output, destination));
  writeFileSync(path.join(output, "VERSION"), `${version}\n`);
} else if (process.argv[2] !== "validate")
  throw new Error("Usage: desktop-release.mjs validate | collect <target>");
