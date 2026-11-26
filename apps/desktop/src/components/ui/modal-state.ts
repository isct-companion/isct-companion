import { useLayoutEffect } from "react";
import { create } from "zustand";
import { getDesktopBridge } from "../../platform/desktop-bridge";
import { useWindowActive } from "../../platform/use-window-active";

const useModals = create(() => ({ app: 0, native: 0 }));

export const useAppModalOpen = () => useModals(({ app }) => app > 0);

/** True while any app modal, native school dialog, or unfocused window blocks
 * automatic dialogs and notice countdowns. */
export function useSurfaceBlocked() {
  const modal = useModals(({ app, native }) => app > 0 || native > 0);
  const active = useWindowActive();
  return modal || !active;
}

export function useAppModalLayer(open: boolean) {
  useLayoutEffect(() => {
    if (!open) return;
    useModals.setState((state) => ({ ...state, app: state.app + 1 }));
    return () =>
      useModals.setState((state) => ({ ...state, app: state.app - 1 }));
  }, [open]);
}

export function trackNativeModals() {
  const stop = getDesktopBridge()?.onSchoolNativeModal?.((opened) => {
    useModals.setState((state) => ({
      ...state,
      native: Math.max(0, state.native + (opened ? 1 : -1)),
    }));
  });
  return () => {
    stop?.();
    useModals.setState((state) => ({ ...state, native: 0 }));
  };
}
