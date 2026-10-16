import { createElement, Fragment } from "react";
import { notifications } from "@mantine/notifications";
import { UiButton } from "../components/mantine";
import { t } from "../i18n";
import { errorText } from "./shared";

const show = (message: React.ReactNode) => {
  notifications.hide("workspace-notice");
  notifications.show({
    id: "workspace-notice",
    message,
    autoClose: 2800,
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
  error: (error: unknown, retry?: () => void) => {
    const message = t(errorText(error) ?? "unknown error");
    show(
      retry
        ? createElement(
            Fragment,
            null,
            message,
            createElement(UiButton, { onClick: retry }, t("Retry saving")),
          )
        : message,
    );
  },
};
