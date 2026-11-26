import { Toast } from "radix-ui";
import { useEffect, useRef, useState } from "react";
import { create } from "zustand";
import { useTranslation } from "react-i18next";
import { t } from "../../i18n";
import { trackNativeModals, useSurfaceBlocked } from "../ui/modal-state";

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

/** Keeps a single remaining-time countdown across hover/focus and modal/window
 * blocks, so Undo (and feedback) only consume time while visible and usable. */
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
    if (paused) return;
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
      duration={Infinity}
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
  const blocked = useSurfaceBlocked();
  useEffect(trackNativeModals, []);
  useEffect(() => () => useNotices.setState({ items: [] }), []);
  return (
    <Toast.Provider swipeDirection="right" label={t("common.notifications")}>
      {notice && (
        <NoticeToast key={notice.revision} notice={notice} blocked={blocked} />
      )}
      <Toast.Viewport
        className="appNotifications"
        label={`${t("common.notifications")} (F8)`}
        aria-label={t("common.notifications")}
      />
    </Toast.Provider>
  );
}
