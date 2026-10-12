import { lstat, readdir, readFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { readOAuthClients } from "./oauth-config.mjs";

const [target, artifact] = process.argv.slice(2);
if (!new Set(["desktop", "ios", "android"]).has(target) || !artifact) {
  throw new Error(
    "Usage: node scripts/verify-platform-bundle.mjs <desktop|ios|android> <artifact>",
  );
}

const clients = readOAuthClients()["google-drive"] ?? {};
const expected = clients[target]?.clientId;
const forbidden = Object.entries(clients)
  .filter(([name]) => name !== target)
  .map(([, value]) => value?.clientId)
  .filter(Boolean);
if (target !== "desktop") forbidden.push("GOCSPX-");

let expectedFound = false;
const leaks = [];

async function scan(file) {
  const metadata = await lstat(file);
  if (metadata.isSymbolicLink()) return;
  if (metadata.isDirectory()) {
    for (const entry of await readdir(file)) await scan(path.join(file, entry));
    return;
  }
  if (!metadata.isFile() || metadata.size > 256 * 1024 * 1024) return;
  const contents = await readFile(file);
  if (expected && contents.includes(Buffer.from(expected)))
    expectedFound = true;
  if (forbidden.some((marker) => contents.includes(Buffer.from(marker)))) {
    leaks.push(path.relative(artifact, file) || path.basename(file));
  }
}

await scan(path.resolve(artifact));
if (leaks.length) {
  throw new Error(
    `The ${target} artifact contains another platform's OAuth configuration: ${leaks.join(", ")}`,
  );
}
if (expected && !expectedFound) {
  throw new Error(
    `The ${target} OAuth client ID is missing from the artifact.`,
  );
}
console.log(`${target} bundle isolation verified.`);
