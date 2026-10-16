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

export { tokyoDate as jstDate } from "../lib/civil-date";
export const stamp = () => new Date().toISOString();
/** Tauri errors may be native typed failures or JavaScript Errors. */
export const errorText = (error: unknown) =>
  error instanceof Error
    ? error.message
    : typeof error === "string"
      ? error
      : error &&
          typeof error === "object" &&
          "message" in error &&
          typeof error.message === "string"
        ? error.message
        : undefined;
