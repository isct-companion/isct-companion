import { useLayoutEffect } from "react";
import { create } from "zustand";
import { getDesktopBridge } from "../../platform/desktop-bridge";

const useModalCount = create(() => ({ count: 0 }));
export const useAppModalOpen = () => useModalCount(({ count }) => count > 0);

export function useAppModalLayer(open: boolean) {
  useLayoutEffect(() => {
    if (!open) return;
    useModalCount.setState(({ count }) => ({ count: count + 1 }));
    return () => useModalCount.setState(({ count }) => ({ count: count - 1 }));
  }, [open]);
}

const useNativeModalCount = create(() => ({ count: 0 }));
export const useNativeModalOpen = () =>
  useNativeModalCount(({ count }) => count > 0);

export function trackNativeModals() {
  const stop = getDesktopBridge()?.onSchoolNativeModal?.((opened) => {
    useNativeModalCount.setState(({ count }) => ({
      count: Math.max(0, count + (opened ? 1 : -1)),
    }));
  });
  return () => {
    stop?.();
    useNativeModalCount.setState({ count: 0 });
  };
}
