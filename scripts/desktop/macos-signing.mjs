import { existsSync, readdirSync, writeFileSync, mkdirSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));
const bundleId = "app.isctcompanion";
const teamId = "FB7AL944GK";

function decodeProfile(profilePath) {
  const decoded = spawnSync("security", ["cms", "-D", "-i", profilePath], {
    encoding: "buffer",
  });
  if (decoded.status !== 0) return undefined;
  // plistlib preserves arrays, booleans and UTC dates across macOS versions.
  const parsed = spawnSync("python3", ["-c", `
import sys, plistlib, json, hashlib
profile = plistlib.loads(sys.stdin.buffer.read())
expires = profile.get("ExpirationDate")
print(json.dumps({
    "Platform": profile.get("Platform", []),
    "ProvisionsAllDevices": profile.get("ProvisionsAllDevices", False),
    "ProvisionedDevices": profile.get("ProvisionedDevices", []),
    "Certificates": [hashlib.sha1(c).hexdigest().upper() for c in profile.get("DeveloperCertificates", [])],
    "ExpirationDate": expires.isoformat() + "Z" if expires else "",
    "Entitlements": profile.get("Entitlements", {})
}))
`], { input: decoded.stdout, encoding: "utf8" });
  if (parsed.status !== 0) return undefined;
  return JSON.parse(parsed.stdout);
}

function authorizes(values, expected) {
  return Array.isArray(values)
    ? values.some((value) => value === expected || value === `${teamId}.*`)
    : false;
}

// A development profile names the Macs it runs on; Developer ID runs anywhere.
function validateProfile(
  profilePath,
  { diagnostics = false, development = false } = {},
) {
  const profile = decodeProfile(profilePath);
  const invalid = (reason) => {
    if (diagnostics) console.error(`Mac provisioning profile rejected: ${reason}`);
    return undefined;
  };
  if (!profile) return invalid("CMS decoding failed");
  const entitlements = profile?.Entitlements ?? {};
  const platforms = profile?.Platform ?? [];
  const appIdentifier = entitlements["com.apple.application-identifier"];
  const expires = Date.parse(profile?.ExpirationDate ?? "");
  if (!platforms.includes("OSX")) return invalid("macOS platform missing");
  if (development) {
    const mac = provisioningUdid();
    if (
      !profile.ProvisionsAllDevices &&
      !(mac && profile.ProvisionedDevices.includes(mac))
    ) {
      return invalid("this Mac is not in the profile");
    }
  } else if (!profile.ProvisionsAllDevices) {
    return invalid("Developer ID distribution missing");
  }
  if (!(expires > Date.now())) return invalid("expiration date invalid or expired");
  if (
    appIdentifier !== `${teamId}.${bundleId}` &&
    appIdentifier !== `${teamId}.*`
  ) {
    return invalid("app identifier does not match");
  }
  if (
    !authorizes(entitlements["keychain-access-groups"], `${teamId}.${bundleId}`)
  ) {
    return invalid("Keychain group missing");
  }
  const containers = entitlements["com.apple.developer.icloud-container-identifiers"];
  if (!Array.isArray(containers) || !containers.includes(`iCloud.${bundleId}`)) {
    return invalid("CloudKit container missing");
  }
  // Entitlements.plist asks for Production, the database the iPhone app uses.
  // A profile that names environments must include it.
  const environments =
    entitlements["com.apple.developer.icloud-container-environment"];
  if (environments && ![environments].flat().includes("Production")) {
    return invalid("CloudKit Production environment missing");
  }
  return { path: profilePath, profile };
}

function provisioningUdid() {
  const hardware = spawnSync("system_profiler", ["SPHardwareDataType"], {
    encoding: "utf8",
  });
  return hardware.stdout?.match(/Provisioning UDID:\s*(\S+)/)?.[1];
}

function signingIdentities() {
  const found = spawnSync(
    "security",
    ["find-identity", "-v", "-p", "codesigning"],
    { encoding: "utf8" },
  );
  return [...(found.stdout ?? "").matchAll(/^\s*\d+\)\s+([0-9A-F]{40})\s/gm)].map(
    (match) => match[1],
  );
}

function profileFromSecret() {
  const encoded = process.env.APPLE_MACOS_PROVISIONING_PROFILE;
  if (!encoded) return undefined;
  const directory = path.join(root, "target", "isct-signing");
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
  // Xcode 16 keeps profiles in UserData; older Xcode in MobileDevice.
  return [
    "Library/Developer/Xcode/UserData/Provisioning Profiles",
    "Library/MobileDevice/Provisioning Profiles",
  ]
    .map((directory) => path.join(homedir(), directory))
    .filter((directory) => existsSync(directory))
    .flatMap((directory) =>
      readdirSync(directory)
        .filter((name) => name.endsWith(".provisionprofile"))
        .map((name) => path.join(directory, name)),
    );
}

/**
 * A debug bundle signed like the release: the Apple Development identity and
 * team profile of whoever builds it, so iCloud and CloudKit work in debug.
 * Undefined when this Mac has no such identity and profile.
 */
export function macDevelopmentSigningConfig() {
  if (process.platform !== "darwin") return undefined;
  const identities = signingIdentities();
  const selected = candidateProfiles()
    .map((profile) => validateProfile(profile, { development: true }))
    .filter(Boolean)
    .map((match) => ({
      ...match,
      identity: match.profile.Certificates.find((hash) =>
        identities.includes(hash),
      ),
    }))
    .filter((match) => match.identity)
    .sort(
      (left, right) =>
        Date.parse(right.profile.ExpirationDate) -
        Date.parse(left.profile.ExpirationDate),
    )[0];
  if (!selected) return undefined;
  return {
    bundle: {
      macOS: {
        signingIdentity: selected.identity,
        entitlements: "Entitlements.plist",
        files: { "embedded.provisionprofile": selected.path },
      },
    },
  };
}

export function macSigningConfig({ required = false } = {}) {
  if (process.platform !== "darwin") return undefined;
  const matches = candidateProfiles()
    .map((profile) => validateProfile(profile, { diagnostics: required }))
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
