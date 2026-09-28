export type Page =
  "today" | "schedule" | "tasks" | "courses" | "web" | "settings";
export type Section =
  | "general"
  | "accounts"
  | "sync"
  | "notifications"
  | "calendar"
  | "data"
  | "advanced";
export type Platform = "native-desktop" | "native-mobile" | "browser-preview";

export const jstDate = (date: Date) =>
  new Intl.DateTimeFormat("en-CA", { timeZone: "Asia/Tokyo" }).format(date);
export const stamp = () => new Date().toISOString();
