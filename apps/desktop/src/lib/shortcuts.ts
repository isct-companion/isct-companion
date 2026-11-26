import type { NativeOs } from "../platform/desktop-bridge";
import type { Page } from "./route-contract";

/** The rail's pages in shortcut order: the primary modifier with 1–6 opens
 * the page at that position. */
export const railPageOrder: readonly Page[] = [
  "home",
  "timetable",
  "lectures",
  "web",
  "tools",
  "pastExams",
];

/** Windows uses Ctrl for app shortcuts; macOS (and the browser preview,
 * which only ever runs on a developer's Mac) uses ⌘. */
export const primaryModifierName = (os: NativeOs | undefined) =>
  os === "windows" ? "Ctrl" : "⌘";
/** The modifier as it prefixes a key: "⌘1", "Ctrl+1". */
export const primaryModifierLabel = (os: NativeOs | undefined) =>
  os === "windows" ? "Ctrl+" : "⌘";

/** The rail label's shortcut hint for `page`, when it has one. */
export const shortcutHint = (page: Page, os: NativeOs | undefined) => {
  if (page === "settings") return `${primaryModifierLabel(os)},`;
  const index = railPageOrder.indexOf(page);
  return index < 0 ? undefined : `${primaryModifierLabel(os)}${index + 1}`;
};

type ShortcutKey = Pick<
  KeyboardEvent,
  "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey"
>;

/** The page a key press opens, or `undefined` when it is not a page
 * shortcut or names a page this build does not show. */
export const shortcutPage = (
  event: ShortcutKey,
  os: NativeOs | undefined,
  available: readonly Page[],
): Page | undefined => {
  const primary = os === "windows" ? event.ctrlKey : event.metaKey;
  if (!primary || event.altKey || event.shiftKey) return undefined;
  if (event.key === ",") return "settings";
  const index = /^[1-9]$/.test(event.key) ? Number(event.key) - 1 : -1;
  const page = railPageOrder[index];
  return page && available.includes(page) ? page : undefined;
};
