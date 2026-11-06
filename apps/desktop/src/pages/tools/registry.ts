import type { Platform, Tool } from "../../app/shared";
import { nativeOs } from "../../platform/desktop-bridge";

/** The tools this build has, in page order. What each one needs is compiled
 * into the native app per platform: ScienceTokyo Wi-Fi sign-in into macOS, the
 * campus VPN into macOS and Windows, the printer into every desktop build. */
export function availableTools(platform: Platform): Tool[] {
  if (platform !== "native-desktop") return [];
  const os = nativeOs();
  return [
    ...(os === "macos" ? (["wifi"] as const) : []),
    ...(os === "macos" || os === "windows" ? (["vpn"] as const) : []),
    "printer",
  ];
}
