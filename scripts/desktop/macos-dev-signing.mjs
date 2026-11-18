import { spawnSync } from "node:child_process";

/**
 * The Apple Development identity `yarn desktop:dev` signs the macOS app with:
 * the one whose SHA-1 ISCT_MACOS_DEV_SIGNING_IDENTITY names. Any developer's
 * own works, a Personal Team included. A fixed identity keeps the app's
 * designated requirement, and with it the keychain items the development app
 * created, across rebuilds.
 */
export function macDevSigningIdentity() {
  const chosen = process.env.ISCT_MACOS_DEV_SIGNING_IDENTITY?.trim().toUpperCase();
  const found = spawnSync("security", ["find-identity", "-v", "-p", "codesigning"], {
    encoding: "utf8",
  });
  const identities = [
    ...(found.stdout ?? "").matchAll(
      /^\s*\d+\)\s+([0-9A-F]{40})\s+"(Apple Development: [^"]+)"/gm,
    ),
  ].map(([, hash, name]) => ({ hash, name }));
  if (chosen && identities.some(({ hash }) => hash === chosen)) return chosen;
  const choices = identities.length
    ? identities.map(({ hash, name }) => `  ${hash}  ${name}`).join("\n")
    : "  none: add your Apple ID in Xcode > Settings > Accounts and create an Apple Development certificate";
  console.error(
    `${chosen ? `ISCT_MACOS_DEV_SIGNING_IDENTITY ${chosen} is not a valid Apple Development identity on this Mac.` : "Set ISCT_MACOS_DEV_SIGNING_IDENTITY to your Apple Development identity."}\n` +
      `Valid identities:\n${choices}`,
  );
  process.exit(1);
}
