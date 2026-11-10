import type { Platform } from "../../app/shared";
import { nativeTools, type DesktopTool } from "../../platform/desktop-bridge";

/** The tools this build has, in page order: the native binary reports the
 * ones it was compiled with (`src-tauri/build.rs` names each tool's systems),
 * so the page never decides by operating system. */
export function availableTools(platform: Platform): readonly DesktopTool[] {
  return platform === "native-desktop" ? nativeTools() : [];
}
