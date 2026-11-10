#!/usr/bin/env node
// Moves the desktop release to a new version in every place CI validates.
//
//   node scripts/release/bump.mjs <version>   rewrite all places to <version>
//   node scripts/release/bump.mjs --check     list the places; fail if they differ
//
// It refuses a version that is not newer than both the files and the latest
// v* tag, and places that already disagree. It does not commit, tag or push;
// it prints the commits since the last tag for the release notes.
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  RELEASE_VERSION,
  readVersions,
  releaseVersion,
  writeVersion,
} from "./versions.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
const git = (...args) =>
  execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();

/** Negative, zero or positive as `a` is older, equal or newer than `b`. */
function compare(a, b) {
  const [coreA, preA] = a.split("-", 2);
  const [coreB, preB] = b.split("-", 2);
  const numbersA = coreA.split(".").map(Number);
  const numbersB = coreB.split(".").map(Number);
  for (let index = 0; index < 3; index += 1)
    if (numbersA[index] !== numbersB[index])
      return numbersA[index] - numbersB[index];
  // A prerelease comes before its release.
  if (preA === preB) return 0;
  if (preA === undefined) return 1;
  if (preB === undefined) return -1;
  return preA < preB ? -1 : 1;
}

function latestTag() {
  return git("tag", "--list", "v*", "--sort=-v:refname").split("\n")[0] || null;
}

const [argument] = process.argv.slice(2);
if (argument === "--check") {
  for (const place of readVersions(root))
    console.log(`${place.version.padEnd(12)} ${place.label} (${place.file})`);
  console.log(`\nRelease version: ${releaseVersion(root)}`);
} else if (argument && RELEASE_VERSION.test(argument)) {
  const current = releaseVersion(root);
  const tag = latestTag();
  if (compare(argument, current) <= 0)
    throw new Error(`${argument} is not newer than the current ${current}`);
  // Newer than the latest tag also means its own tag is not taken.
  if (tag && compare(argument, tag.slice(1)) <= 0)
    throw new Error(`${argument} is not newer than the latest tag ${tag}`);
  const files = writeVersion(root, argument);
  console.log(`${current} → ${argument}\n`);
  for (const file of files) console.log(`  ${file}`);
  const notes = `docs/releases/${argument}.md`;
  console.log(
    existsSync(path.join(root, notes))
      ? `\nRelease notes: ${notes}`
      : `\nWrite ${notes} before tagging; the release workflow requires it.`,
  );
  if (tag) {
    console.log(`\nCommits since ${tag}:`);
    console.log(git("log", "--format=  %h %s", `${tag}..HEAD`) || "  (none)");
  }
} else {
  throw new Error(
    "Usage: bump.mjs <version> | --check   (version like 1.2.3 or 1.2.3-rc.1)",
  );
}
