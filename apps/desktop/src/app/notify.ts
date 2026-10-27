import { notifications } from "./notifications";
import { t } from "../i18n";
import { errorText } from "./shared";

/** Every transient notice is one line and at most one action, for 4 s
 * (slotledger approach/feedback-banner-toast). */
const NOTICE_MS = 4000;
const show = (message: string, action?: { label: string; run: () => void }) => {
  notifications.hide("workspace-notice");
  notifications.show({
    id: "workspace-notice",
    message,
    action,
    autoClose: NOTICE_MS,
    withCloseButton: false,
  });
};
let changedItems = 0;
let changedTimer: ReturnType<typeof setTimeout> | undefined;
export const notify = {
  updated: (count: number) => {
    if (count <= 0) return;
    changedItems += count;
    clearTimeout(changedTimer);
    changedTimer = setTimeout(() => {
      notifications.show({
        id: "school-update",
        message: t("Updated {{count}} items", { count: changedItems }),
        autoClose: NOTICE_MS,
        withCloseButton: false,
      });
      changedItems = 0;
    }, 600);
  },
  info: (message: string) => show(t(message)),
  /** A finished change with a button that reverses it. */
  undoable: (message: string, undo: () => void) =>
    show(t(message), { label: t("Undo"), run: undo }),
  error: (error: unknown, retry?: () => void) => {
    const message = t(errorText(error) ?? "unknown error");
    show(message, retry && { label: t("Retry saving"), run: retry });
  },
};
