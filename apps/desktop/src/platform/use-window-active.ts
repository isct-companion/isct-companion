import { useSyncExternalStore } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getDesktopBridge } from "./desktop-bridge";
import { quietly } from "./desktop-bridge/helpers";

let nativeFocus: boolean | undefined;
const listeners = new Set<() => void>();
let release: (() => void) | undefined;

const active = () =>
  document.visibilityState !== "hidden" && (nativeFocus ?? document.hasFocus());

function watch() {
  const update = () => listeners.forEach((listener) => listener());
  let closed = false;
  let removeNative: (() => void) | undefined;
  window.addEventListener("focus", update);
  window.addEventListener("blur", update);
  document.addEventListener("visibilitychange", update);
  if (getDesktopBridge()) {
    const windowHandle = getCurrentWindow();
    const focus = (focused: boolean) => {
      if (closed || typeof focused !== "boolean") return;
      nativeFocus = focused;
      update();
    };
    void windowHandle
      .onFocusChanged(({ payload }) => focus(payload))
      .then((unlisten) => {
        if (closed) return quietly(unlisten);
        removeNative = unlisten;
        return windowHandle.isFocused().then(focus);
      })
      .catch(() => undefined);
  }
  return () => {
    closed = true;
    nativeFocus = undefined;
    if (removeNative) quietly(removeNative);
    window.removeEventListener("focus", update);
    window.removeEventListener("blur", update);
    document.removeEventListener("visibilitychange", update);
  };
}

function subscribe(update: () => void) {
  listeners.add(update);
  release ??= watch();
  return () => {
    listeners.delete(update);
    if (listeners.size) return;
    release?.();
    release = undefined;
  };
}

export const useWindowActive = () => useSyncExternalStore(subscribe, active);
