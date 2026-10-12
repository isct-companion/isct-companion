import { existsSync, readdirSync, writeFileSync, mkdirSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const bundleId = "app.isctcompanion";
const teamId = "FB7AL944GK";

function decodeProfile(profilePath) {
  const decoded = spawnSync("openssl", [
    "smime",
    "-verify",
    "-inform",
    "der",
    "-in",
    profilePath,
    "-noverify",
  ], {
    encoding: "buffer",
  });
  if (decoded.status !== 0) return undefined;
  const extract = (keyPath, format = "raw") => {
    const result = spawnSync(
      "plutil",
      ["-extract", keyPath, format, "-o", "-", "-"],
      { input: decoded.stdout, encoding: "utf8" },
    );
    if (result.status !== 0) return undefined;
    const value = result.stdout.trim();
    if (format === "json") {
      try {
        return JSON.parse(value);
      } catch {
        return undefined;
      }
    }
    return value;
  };
  return {
    Platform: extract("Platform", "json"),
    ExpirationDate: extract("ExpirationDate"),
    Entitlements: {
      "com.apple.application-identifier": extract(
        "Entitlements.com\\.apple\\.application-identifier",
      ),
      "com.apple.developer.icloud-container-identifiers": extract(
        "Entitlements.com\\.apple\\.developer\\.icloud-container-identifiers",
        "json",
      ),
      "keychain-access-groups": extract(
        "Entitlements.keychain-access-groups",
        "json",
      ),
    },
  };
}

function authorizes(values, expected) {
  return Array.isArray(values)
    ? values.some((value) => value === expected || value === `${teamId}.*`)
    : false;
}

function validateProfile(profilePath) {
  const profile = decodeProfile(profilePath);
  const entitlements = profile?.Entitlements ?? {};
  const platforms = profile?.Platform ?? [];
  const appIdentifier = entitlements["com.apple.application-identifier"];
  const expires = Date.parse(profile?.ExpirationDate ?? "");
  if (!platforms.includes("OSX")) return undefined;
  if (!(expires > Date.now())) return undefined;
  if (
    appIdentifier !== `${teamId}.${bundleId}` &&
    appIdentifier !== `${teamId}.*`
  ) {
    return undefined;
  }
  if (
    !authorizes(entitlements["keychain-access-groups"], `${teamId}.${bundleId}`)
  ) {
    return undefined;
  }
  const containers = entitlements["com.apple.developer.icloud-container-identifiers"];
  if (!Array.isArray(containers) || !containers.includes(`iCloud.${bundleId}`)) {
    return undefined;
  }
  return { path: profilePath, profile };
}

function profileFromSecret() {
  const encoded = process.env.APPLE_MACOS_PROVISIONING_PROFILE;
  if (!encoded) return undefined;
  const directory = path.join(root, "backend", "target", "isct-signing");
  const destination = path.join(directory, "embedded.provisionprofile");
  mkdirSync(directory, { recursive: true });
  writeFileSync(destination, Buffer.from(encoded.replace(/\s+/g, ""), "base64"));
  return destination;
}

function candidateProfiles() {
  const explicit = process.env.ISCT_MACOS_PROVISIONING_PROFILE;
  if (explicit) return [path.resolve(explicit)];
  const secret = profileFromSecret();
  if (secret) return [secret];
  const directory = path.join(
    homedir(),
    "Library/Developer/Xcode/UserData/Provisioning Profiles",
  );
  if (!existsSync(directory)) return [];
  return readdirSync(directory)
    .filter((name) => name.endsWith(".provisionprofile"))
    .map((name) => path.join(directory, name));
}

export function macSigningConfig({ required = false } = {}) {
  if (process.platform !== "darwin") return undefined;
  const matches = candidateProfiles()
    .map(validateProfile)
    .filter(Boolean)
    .sort(
      (left, right) =>
        Date.parse(right.profile.ExpirationDate) -
        Date.parse(left.profile.ExpirationDate),
    );
  const selected = matches[0];
  if (!selected) {
    if (!required) return undefined;
    throw new Error(
      `No valid macOS provisioning profile authorizes ${bundleId}, iCloud and synchronized Keychain.`,
    );
  }
  return {
    bundle: {
      macOS: {
        entitlements: "Entitlements.plist",
        files: { "embedded.provisionprofile": selected.path },
      },
    },
  };
}
