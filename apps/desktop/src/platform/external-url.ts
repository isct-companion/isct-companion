import type { Platform } from "./capabilities";
import { getDesktopBridge } from "./desktop-bridge";

/**
 * Opens a URL in the system browser. Inside the desktop app a plain
 * `target="_blank"` link does nothing, so this goes through the native opener.
 */
export function openExternalUrl(url: string, platform?: Platform) {
  const bridge = platform === "browser-preview" ? undefined : getDesktopBridge();
  if (bridge) return bridge.openExternal(url);
  window.open(url, "_blank", "noopener,noreferrer");
  return Promise.resolve();
}
