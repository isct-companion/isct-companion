import { Text } from "@mantine/core";
import { modals } from "@mantine/modals";
import { t } from "../i18n";

/**
 * Asks before a destructive action with an in-app Mantine dialog; resolves
 * true only when the user presses the confirm button. Never use
 * `window.confirm`: tauri-plugin-dialog replaces it with an async version, so
 * `!confirm(...)` is always false and the action runs without asking.
 * Needs `<ModalsProvider>` (mounted in `App`).
 */
export const confirmAction = (message: string, confirmLabel: string) =>
  new Promise<boolean>((resolve) => {
    modals.openConfirmModal({
      withCloseButton: false,
      centered: true,
      children: <Text size="sm">{message}</Text>,
      labels: { confirm: confirmLabel, cancel: t("Cancel") },
      confirmProps: { color: "red" },
      onConfirm: () => resolve(true),
      // Cancel, Escape, and clicking outside all land here; a settled promise ignores it.
      onClose: () => resolve(false),
    });
  });
