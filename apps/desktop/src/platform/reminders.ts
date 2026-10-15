/** Native Core owns reminder planning, timing and durable delivery deduplication. */
import type { DesktopBridge } from "./desktop-bridge";
export const CLASS_LEAD_CHOICES = [5, 10, 15, 30] as const;
export const DEFAULT_CLASS_LEAD = 10;

/** One-time transfer of the previous webview's device-local delivery log. */
export async function bootstrapReminders(
  bridge: Pick<DesktopBridge, "bootstrapReminders">,
  storage:
    | Pick<Storage, "getItem" | "removeItem">
    | undefined = typeof localStorage === "undefined"
    ? undefined
    : localStorage,
) {
  const key = "isct-reminders-fired";
  let fired: unknown = {};
  try {
    fired = JSON.parse(storage?.getItem(key) ?? "{}") ?? {};
  } catch {
    /* An unreadable legacy log was empty in the previous runtime too. */
  }
  await bridge.bootstrapReminders(fired);
  try {
    storage?.removeItem(key);
  } catch {
    /* Core records completed migration atomically. */
  }
}
