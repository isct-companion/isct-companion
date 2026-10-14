export type Page = "home" | "lectures" | "web" | "settings";
export type Section =
  | "accounts"
  | "study"
  | "notifications"
  | "calendar"
  | "appearance"
  | "files"
  | "backup"
  | "sources"
  | "automation"
  | "wifi"
  | "developer"
  | "agent"
  | "extensions"
  | "diagnostics"
  | "about";
export type Platform = "native-desktop" | "native-mobile" | "browser-preview";

export { tokyoDate as jstDate } from "../core/civil-date";
export const stamp = () => new Date().toISOString();
/** Tauri commands reject with plain strings, not Errors. */
export const errorText = (error: unknown) =>
  error instanceof Error
    ? error.message
    : typeof error === "string"
      ? error
      : undefined;
