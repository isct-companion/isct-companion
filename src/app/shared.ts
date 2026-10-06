export type Page = "today" | "schedule" | "web" | "settings";
export type Section =
  | "general"
  | "accounts"
  | "sync"
  | "notifications"
  | "calendar"
  | "subscriptions"
  | "hidden"
  | "scheduled"
  | "agent"
  | "data"
  | "files"
  | "extensions"
  | "configs";
export type Platform = "native-desktop" | "native-mobile" | "browser-preview";

export { tokyoDate as jstDate } from "../core/schedule";
export const stamp = () => new Date().toISOString();
