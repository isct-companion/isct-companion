// Build-time access to config/oauth-clients.yaml. Client IDs are public and
// fixed at build time, not at runtime.
import { readFileSync } from "node:fs";
import { parse } from "yaml";

const root = new URL("../../", import.meta.url);

export function readOAuthClients() {
  return parse(
    readFileSync(new URL("config/oauth-clients.yaml", root), "utf8"),
  );
}

export function googleDriveClientId(target, clients = readOAuthClients()) {
  return clients["google-drive"]?.[target]?.trim() ?? "";
}

/**
 * Build env for the native backend: Google's Desktop client needs its
 * secret in token requests. An already-set environment value (CI) wins.
 */
export function nativeOAuthEnv(
  clients = readOAuthClients(),
  env = process.env,
) {
  const google = clients["google-drive"] ?? {};
  return {
    ISCT_GOOGLE_DESKTOP_CLIENT_ID:
      env.ISCT_GOOGLE_DESKTOP_CLIENT_ID ?? google.desktop?.trim() ?? "",
    ISCT_GOOGLE_DESKTOP_CLIENT_SECRET:
      env.ISCT_GOOGLE_DESKTOP_CLIENT_SECRET ??
      google["desktop-secret"]?.trim() ??
      "",
  };
}
