export type Page = "today" | "schedule" | "web" | "settings";
export type Section =
  | "accounts"
  | "study"
  | "notifications"
  | "calendar"
  | "appearance"
  | "files"
  | "backup"
  | "hidden"
  | "sources"
  | "automation"
  | "developer"
  | "agent"
  | "extensions"
  | "diagnostics";
export type Platform = "native-desktop" | "native-mobile" | "browser-preview";

export { tokyoDate as jstDate } from "../core/schedule";
export const stamp = () => new Date().toISOString();
