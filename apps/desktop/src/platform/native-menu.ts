import { Menu, MenuItem, PredefinedMenuItem } from "@tauri-apps/api/menu";

/** One line of a native pop-up menu: a disabled fact, a divider or a choice. */
export type NativeMenuEntry =
  | { kind: "label"; text: string }
  | { kind: "separator" }
  | { kind: "action"; text: string; run: () => void };

/**
 * Opens a system menu at the pointer. The school views are native views
 * above the page, so a menu drawn in HTML would hide under them; the
 * system's own menu floats over every view (Tauri's menu API, so no
 * menu code of our own).
 */
export async function popupNativeMenu(entries: NativeMenuEntry[]) {
  const items = await Promise.all(
    entries.map((entry) =>
      entry.kind === "separator"
        ? PredefinedMenuItem.new({ item: "Separator" })
        : MenuItem.new({
            text: entry.text,
            enabled: entry.kind === "action",
            action: entry.kind === "action" ? entry.run : undefined,
          }),
    ),
  );
  const menu = await Menu.new({ items });
  await menu.popup();
}
