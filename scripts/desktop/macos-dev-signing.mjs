import { execFileSync } from "node:child_process";
import { X509Certificate } from "node:crypto";

/**
 * Both forms of the macOS development app use this Apple Development
 * identity, so they can read the same Keychain items across rebuilds.
 */
export function macDevSigningIdentity() {
  const chosen =
    process.env.ISCT_MACOS_DEV_SIGNING_IDENTITY?.trim().toUpperCase();
  const found = execFileSync(
    "/usr/bin/security",
    ["find-identity", "-v", "-p", "codesigning"],
    { encoding: "utf8" },
  );
  const identities = [
    ...found.matchAll(
      /^\s*\d+\)\s+([0-9A-F]{40})\s+"(Apple Development: [^"]+)"/gm,
    ),
  ].map(([, hash, name]) => ({ hash, name }));
  if (chosen && identities.some(({ hash }) => hash === chosen)) return chosen;
  if (!chosen && identities.length === 1) return identities[0].hash;
  const choices = identities.length
    ? identities.map(({ hash, name }) => `  ${hash}  ${name}`).join("\n")
    : "  none: add your Apple ID in Xcode > Settings > Accounts and create an Apple Development certificate";
  console.error(
    `${chosen ? `ISCT_MACOS_DEV_SIGNING_IDENTITY ${chosen} is not a valid Apple Development identity on this Mac.` : "Set ISCT_MACOS_DEV_SIGNING_IDENTITY to your Apple Development identity."}\n` +
      `Valid identities:\n${choices}`,
  );
  process.exit(1);
}

/** The signing team's ID for the development app and widget's shared group. */
export function macDevSigningTeam(identity) {
  const certificates = execFileSync(
    "/usr/bin/security",
    ["find-certificate", "-a", "-c", "Apple Development:", "-p"],
    { encoding: "utf8" },
  );
  const certificate = [
    ...certificates.matchAll(
      /-----BEGIN CERTIFICATE-----[\s\S]*?-----END CERTIFICATE-----/g,
    ),
  ]
    .map(([pem]) => new X509Certificate(pem))
    .find((cert) => cert.fingerprint.replaceAll(":", "") === identity);
  const team = certificate?.toLegacyObject().subject.OU;
  if (typeof team !== "string" || !/^[A-Z0-9]{10}$/.test(team)) {
    throw new Error(
      "Could not read the team ID from the selected Apple Development certificate.",
    );
  }
  return team;
}
