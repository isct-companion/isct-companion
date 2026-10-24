import { Toast } from "radix-ui";
import { useEffect, type ReactNode } from "react";
import { create } from "zustand";
import { t } from "../i18n";
import { useTranslation } from "react-i18next";

type Notice = {
  id: string;
  message: ReactNode;
  autoClose?: number | false;
  withCloseButton?: boolean;
  color?: string;
  classNames?: { root?: string; description?: string };
  revision: number;
};
const useNotices = create<{ items: Notice[] }>(() => ({ items: [] }));
let revision = 0;
export const notifications = {
  show(notice: Omit<Notice, "revision">) {
    useNotices.setState(({ items }) => ({
      items: items.some((item) => item.id === notice.id)
        ? items.map((item) =>
            item.id === notice.id ? { ...notice, revision: ++revision } : item,
          )
        : [...items, { ...notice, revision: ++revision }],
    }));
  },
  hide(id: string) {
    useNotices.setState(({ items }) => ({
      items: items.filter((item) => item.id !== id),
    }));
  },
};
/** One visible toast, with queued notices and pause/swipe/keyboard handling owned by Radix. */
export function Notifications() {
  useTranslation();
  const notice = useNotices((state) => state.items[0]);
  useEffect(() => () => useNotices.setState({ items: [] }), []);
  return (
    <Toast.Provider swipeDirection="right" label={t("Notifications")}>
      {notice && (
        <Toast.Root
          key={`${notice.id}:${notice.revision}`}
          className={`uiToast ${notice.classNames?.root ?? ""}`}
          data-color={notice.color}
          duration={
            notice.autoClose === false ? Infinity : (notice.autoClose ?? 4000)
          }
          onOpenChange={(open) => {
            if (!open) notifications.hide(notice.id);
          }}
        >
          <Toast.Description asChild>
            <div className={notice.classNames?.description}>
              {notice.message}
            </div>
          </Toast.Description>
          {notice.withCloseButton && (
            <Toast.Close className="uiToastClose" aria-label={t("Close")}>
              ×
            </Toast.Close>
          )}
        </Toast.Root>
      )}
      <Toast.Viewport
        className="appNotifications"
        label={`${t("Notifications")} (F8)`}
        aria-label={t("Notifications")}
      />
    </Toast.Provider>
  );
}
