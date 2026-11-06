export type Page =
  "home" | "timetable" | "lectures" | "web" | "tools" | "settings" | "about";
export type Tool = "wifi" | "vpn" | "printer";
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
  | "developer"
  | "agent"
  | "extensions"
  | "diagnostics";
export type { Platform } from "../platform/capabilities";

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
