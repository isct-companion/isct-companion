import { existsSync, readdirSync, writeFileSync, mkdirSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  appleDevelopmentIdentities,
  macDevSigningIdentity,
} from "./macos-dev-signing.mjs";

const root = fileURLToPath(new URL("../../", import.meta.url));
const bundleId = "app.isctcompanion";
const teamId = "FB7AL944GK";

function decodeProfile(profilePath) {
  const decoded = spawnSync("security", ["cms", "-D", "-i", profilePath], {
    encoding: "buffer",
  });
  if (decoded.status !== 0) return undefined;
  // plistlib preserves arrays, booleans and UTC dates across macOS versions.
  const parsed = spawnSync(
    "python3",
    [
      "-c",
      `
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
`,
    ],
    { input: decoded.stdout, encoding: "utf8" },
  );
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
    if (diagnostics)
      console.error(`Mac provisioning profile rejected: ${reason}`);
    return undefined;
  };
  if (!profile) return invalid("CMS decoding failed");
  const entitlements = profile?.Entitlements ?? {};
  const platforms = profile?.Platform ?? [];
  const appIdentifier = entitlements["com.apple.application-identifier"];
  const expires = Date.parse(profile?.ExpirationDate ?? "");
  if (!platforms.includes("OSX")) return invalid("macOS platform missing");
  if (development) {
    if (profile.ProvisionsAllDevices)
      return invalid("expected a development profile");
    const mac = provisioningUdid();
    if (!mac || !profile.ProvisionedDevices.includes(mac)) {
      return invalid("this Mac is not in the profile");
    }
  } else if (!profile.ProvisionsAllDevices) {
    return invalid("Developer ID distribution missing");
  }
  if (!(expires > Date.now()))
    return invalid("expiration date invalid or expired");
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
  const containers =
    entitlements["com.apple.developer.icloud-container-identifiers"];
  if (
    !Array.isArray(containers) ||
    !containers.includes(`iCloud.${bundleId}`)
  ) {
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

function profileFromSecret() {
  const encoded = process.env.APPLE_MACOS_PROVISIONING_PROFILE;
  if (!encoded) return undefined;
  const directory = path.join(root, "target", "isct-signing");
  const destination = path.join(directory, "embedded.provisionprofile");
  mkdirSync(directory, { recursive: true });
  writeFileSync(
    destination,
    Buffer.from(encoded.replace(/\s+/g, ""), "base64"),
  );
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

/** An explicitly requested project-team profile enables release behavior in debug. */
function macDevelopmentSigningConfig(profilePath) {
  const profile = path.resolve(profilePath);
  const selected = validateProfile(profile, {
    development: true,
    diagnostics: true,
  });
  const identity =
    selected &&
    appleDevelopmentIdentities().find(({ hash }) =>
      selected.profile.Certificates.includes(hash),
    );
  if (!identity) {
    throw new Error(
      "ISCT_MACOS_PROVISIONING_PROFILE must authorize this Mac and match an installed Apple Development identity.",
    );
  }
  return {
    signingIdentity: identity.hash,
    entitlements: "Entitlements.plist",
    files: { "embedded.provisionprofile": profile },
  };
}

export function macSigningConfig() {
  const matches = candidateProfiles()
    .map((profile) => validateProfile(profile, { diagnostics: true }))
    .filter(Boolean)
    .sort(
      (left, right) =>
        Date.parse(right.profile.ExpirationDate) -
        Date.parse(left.profile.ExpirationDate),
    );
  const selected = matches[0];
  if (!selected) {
    throw new Error(
      `No valid macOS provisioning profile authorizes ${bundleId}, iCloud and synchronized Keychain.`,
    );
  }
  return {
    entitlements: "Entitlements.plist",
    files: { "embedded.provisionprofile": selected.path },
  };
}

/** Select one identity for the app, its widget and any bundled VPN helper. */
export function macosBuildSigning({ dev, debug, packagesApp, noSign }) {
  if (dev) return { developmentIdentity: macDevSigningIdentity() };
  if (noSign) return {};

  const releaseIdentity = process.env.APPLE_SIGNING_IDENTITY?.trim();
  if (releaseIdentity) {
    return {
      releaseIdentity,
      entitled: true,
      macOS: { ...macSigningConfig(), signingIdentity: releaseIdentity },
    };
  }

  const profile = process.env.ISCT_MACOS_PROVISIONING_PROFILE;
  if (debug && profile) {
    const macOS = macDevelopmentSigningConfig(profile);
    console.log(
      "Debug bundle: using the explicitly selected project profile and release app identity.",
    );
    return {
      developmentIdentity: macOS.signingIdentity,
      entitled: true,
      macOS,
    };
  }
  if (debug && packagesApp) {
    const developmentIdentity = macDevSigningIdentity();
    console.log(
      "Debug bundle: using Apple Development signing and the development app identity.",
    );
    return {
      developmentIdentity,
      macOS: { signingIdentity: developmentIdentity, hardenedRuntime: false },
    };
  }
  return {};
}
