// Every place the desktop release version is written. The release workflow
// validates them (scripts/desktop/desktop-release.mjs) and the bump script
// rewrites them (scripts/release/bump.mjs); both read this one list.
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

export const RELEASE_VERSION = /^\d+\.\d+\.\d+(?:-[a-zA-Z0-9.-]+)?$/;

const json = (pick) => (text) => pick(JSON.parse(text));
const TOP_LEVEL_VERSION = /^( {2}"version": ")([^"]+)(")/m;

/** `read` is the authority; `write` is a targeted edit that keeps formatting. */
const PLACES = [
  {
    label: "workspace package.json",
    file: "package.json",
    read: json((value) => value.version),
    write: TOP_LEVEL_VERSION,
  },
  {
    label: "desktop package.json",
    file: "apps/desktop/package.json",
    read: json((value) => value.version),
    write: TOP_LEVEL_VERSION,
  },
  {
    label: "Tauri version",
    file: "apps/desktop/src-tauri/tauri.conf.json",
    read: json((value) => value.version),
    write: TOP_LEVEL_VERSION,
  },
  {
    label: "macOS bundleVersion",
    file: "apps/desktop/src-tauri/tauri.conf.json",
    read: json((value) => value.bundle?.macOS?.bundleVersion),
    write: /("bundleVersion": ")([^"]+)(")/,
  },
  {
    label: "Cargo.toml",
    file: "apps/desktop/src-tauri/Cargo.toml",
    read: (text) => /^version = "([^"]+)"/m.exec(text)?.[1],
    write: /^(version = ")([^"]+)(")/m,
  },
  {
    label: "Cargo.lock",
    file: "Cargo.lock",
    read: (text) =>
      /name = "isct-companion"\r?\nversion = "([^"]+)"/.exec(text)?.[1],
    write: /(name = "isct-companion"\r?\nversion = ")([^"]+)(")/,
  },
];

/** The version each place holds, in a fixed order. */
export function readVersions(root) {
  return PLACES.map(({ label, file, read }) => ({
    label,
    file,
    version: read(readFileSync(path.join(root, file), "utf8")),
  }));
}

/** The one version every place agrees on; throws naming the first that differs. */
export function releaseVersion(root) {
  const [first, ...rest] = readVersions(root);
  const odd = rest.find((place) => place.version !== first.version);
  if (odd)
    throw new Error(
      `${odd.label}/${first.label} version mismatch: ${odd.version} in ${odd.file}, ${first.version} in ${first.file}`,
    );
  return first.version;
}

/** Writes `version` into every place, then reads them back to prove it. */
export function writeVersion(root, version) {
  for (const { file, write } of PLACES) {
    const full = path.join(root, file);
    const text = readFileSync(full, "utf8");
    if (!write.test(text)) throw new Error(`No version to replace in ${file}`);
    writeFileSync(
      full,
      text.replace(write, (_, head, _old, tail) => `${head}${version}${tail}`),
    );
  }
  const written = releaseVersion(root);
  if (written !== version)
    throw new Error(`Wrote ${version} but the files read ${written}`);
  return [...new Set(PLACES.map((place) => place.file))];
}
