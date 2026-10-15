import { Text } from "@mantine/core";
import { modals } from "@mantine/modals";
import { t } from "../i18n";

/**
 * Asks before an action with an in-app Mantine dialog; resolves true only
 * when the user presses the confirm button (red unless `danger: false`). Never use
 * `window.confirm`: tauri-plugin-dialog replaces it with an async version, so
 * `!confirm(...)` is always false and the action runs without asking.
 * Needs `<ModalsProvider>` (mounted in `App`).
 */
export const confirmAction = (
  message: string,
  confirmLabel: string,
  { title, danger = true }: { title?: string; danger?: boolean } = {},
) =>
  new Promise<boolean>((resolve) => {
    modals.openConfirmModal({
      title,
      withCloseButton: false,
      centered: true,
      children: <Text size="sm">{message}</Text>,
      labels: { confirm: confirmLabel, cancel: t("Cancel") },
      confirmProps: danger ? { color: "red" } : undefined,
      onConfirm: () => resolve(true),
      // Cancel, Escape, and clicking outside all land here; a settled promise ignores it.
      onClose: () => resolve(false),
    });
  });
