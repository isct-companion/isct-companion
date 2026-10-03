export type Page =
  "today" | "schedule" | "courses" | "web" | "settings";
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
  | "advanced";
export type Platform = "native-desktop" | "native-mobile" | "browser-preview";

export const jstDate = (date: Date) =>
  new Intl.DateTimeFormat("en-CA", { timeZone: "Asia/Tokyo" }).format(date);
export const stamp = () => new Date().toISOString();
