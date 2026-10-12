// Build-time access to config/oauth-clients.yaml. Client IDs are public; the
// platform choice and the iOS callback scheme are fixed here, not at runtime.
import { readFileSync, writeFileSync } from "node:fs";
import { parse } from "yaml";

const root = new URL("../", import.meta.url);

export function readOAuthClients() {
  return parse(
    readFileSync(new URL("config/oauth-clients.yaml", root), "utf8"),
  );
}

/** Maps Tauri's TAURI_ENV_PLATFORM to a Google client type. */
export function oauthClientTarget(
  platform = process.env.TAURI_ENV_PLATFORM,
  env = process.env,
) {
  if (
    platform === "ios" ||
    env.PLATFORM_NAME === "iphoneos" ||
    env.PLATFORM_NAME === "iphonesimulator" ||
    env.SDK_NAME?.startsWith("iphone")
  )
    return "ios";
  if (platform === "android") return "android";
  return "desktop";
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
    ISCT_GOOGLE_IOS_CLIENT_ID:
      env.ISCT_GOOGLE_IOS_CLIENT_ID ?? google.ios?.trim() ?? "",
    ISCT_GOOGLE_ANDROID_CLIENT_ID:
      env.ISCT_GOOGLE_ANDROID_CLIENT_ID ?? google.android?.trim() ?? "",
    ISCT_GOOGLE_DESKTOP_CLIENT_ID:
      env.ISCT_GOOGLE_DESKTOP_CLIENT_ID ?? google.desktop?.trim() ?? "",
    ISCT_GOOGLE_DESKTOP_CLIENT_SECRET:
      env.ISCT_GOOGLE_DESKTOP_CLIENT_SECRET ??
      google["desktop-secret"]?.trim() ??
      "",
  };
}

/** Google's iOS callback scheme is the client ID with its parts reversed. */
export function googleRedirectScheme(clientId) {
  return clientId.split(".").reverse().join(".");
}

export function iosInfoPlist(clientId) {
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- Generated from config/oauth-clients.yaml by scripts/oauth-config.mjs. -->
<plist version="1.0">
<dict>
  <key>GIDClientID</key>
  <string>${clientId}</string>
  <key>ITSAppUsesNonExemptEncryption</key>
  <false/>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key>
      <string>google-oauth</string>
      <key>CFBundleURLSchemes</key>
      <array>
        <string>${googleRedirectScheme(clientId)}</string>
      </array>
    </dict>
  </array>
</dict>
</plist>
`;
}

/** Tauri merges backend/Info.ios.plist into the generated iOS Info.plist. */
export function writeIosInfoPlist(clients = readOAuthClients()) {
  const clientId = googleDriveClientId("ios", clients);
  if (!clientId) return;
  writeFileSync(
    new URL("backend/Info.ios.plist", root),
    iosInfoPlist(clientId),
  );
}
