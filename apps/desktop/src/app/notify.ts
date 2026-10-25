import { notifications } from "./notifications";
import { t } from "../i18n";
import { errorText } from "./shared";

const show = (
  message: string,
  action?: { label: string; run: () => void },
  autoClose = 2800,
) => {
  notifications.hide("workspace-notice");
  notifications.show({
    id: "workspace-notice",
    message,
    action,
    autoClose,
    withCloseButton: false,
    classNames: { root: "workspaceNotice", description: "workspaceNoticeText" },
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
        autoClose: 1800,
        withCloseButton: false,
        classNames: {
          root: "workspaceNotice",
          description: "workspaceNoticeText",
        },
      });
      changedItems = 0;
    }, 600);
  },
  info: (message: string) => show(t(message)),
  /** A finished change with a button that reverses it. */
  undoable: (message: string, undo: () => void) =>
    show(t(message), { label: t("Undo"), run: undo }, 6000),
  error: (error: unknown, retry?: () => void) => {
    const message = t(errorText(error) ?? "unknown error");
    show(message, retry && { label: t("Retry saving"), run: retry });
  },
};
