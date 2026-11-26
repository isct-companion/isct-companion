import { Toast } from "radix-ui";
import { useEffect, useRef, useState } from "react";
import { create } from "zustand";
import { useTranslation } from "react-i18next";
import { t } from "../../i18n";
import { useWindowActive } from "../../platform/use-window-active";
import {
  trackNativeModals,
  useAppModalOpen,
  useNativeModalOpen,
} from "../ui/modal-state";

type Notice = {
  id: string;
  message: string;
  action?: { label: string; run: () => void };
  undo?: boolean;
  revision: number;
};

const DURATION = 4000;
const useNotices = create<{ items: Notice[] }>(() => ({ items: [] }));
let revision = 0;

export const notifications = {
  show(notice: Omit<Notice, "revision">) {
    const entry = { ...notice, revision: ++revision };
    useNotices.setState(({ items }) => ({
      items: [...items.filter((item) => item.id !== entry.id), entry],
    }));
  },
};

function close(notice: Notice) {
  const { items } = useNotices.getState();
  if (!items.includes(notice)) return false;
  useNotices.setState({ items: items.filter((item) => item !== notice) });
  return true;
}

export const useUndoAvailable = () =>
  useNotices(({ items }) => items.some((item) => item.undo));

/** An undo stays attached to its operation and only counts down while usable. */
function NoticeToast({
  notice,
  blocked,
}: {
  notice: Notice;
  blocked: boolean;
}) {
  const remaining = useRef(DURATION);
  const [paused, setPaused] = useState(false);
  useEffect(() => {
    if (blocked) {
      setPaused(false);
      return;
    }
    if (!notice.undo || paused) return;
    const start = performance.now();
    const timer = window.setTimeout(() => close(notice), remaining.current);
    return () => {
      window.clearTimeout(timer);
      remaining.current = Math.max(
        0,
        remaining.current - (performance.now() - start),
      );
    };
  }, [notice, paused, blocked]);
  if (blocked) return null;
  return (
    <Toast.Root
      className="uiToast"
      duration={notice.undo ? Infinity : DURATION}
      onPause={() => setPaused(true)}
      onResume={() => setPaused(false)}
      onOpenChange={(open) => {
        if (!open) close(notice);
      }}
    >
      <Toast.Description>{notice.message}</Toast.Description>
      {notice.action && (
        <Toast.Action
          className="uiToastAction"
          altText={notice.action.label}
          onClick={() => {
            if (close(notice)) notice.action?.run();
          }}
        >
          {notice.action.label}
        </Toast.Action>
      )}
    </Toast.Root>
  );
}

export function Notifications() {
  useTranslation();
  const notice = useNotices(
    ({ items }) => items.find((item) => item.undo) ?? items[0],
  );
  const modal = useAppModalOpen();
  const nativeModal = useNativeModalOpen();
  useEffect(trackNativeModals, []);
  const active = useWindowActive();
  useEffect(() => () => useNotices.setState({ items: [] }), []);
  return (
    <Toast.Provider swipeDirection="right" label={t("common.notifications")}>
      {notice && (
        <NoticeToast
          key={notice.revision}
          notice={notice}
          blocked={modal || nativeModal || !active}
        />
      )}
      <Toast.Viewport
        className="appNotifications"
        label={`${t("common.notifications")} (F8)`}
        aria-label={t("common.notifications")}
      />
    </Toast.Provider>
  );
}
