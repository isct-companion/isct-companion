import { useSyncExternalStore } from "react";
import { detectPlatform, type Platform } from "../platform/capabilities";

/**
 * Browser previews can use the compact shell at phone and portrait-tablet
 * sizes. The native desktop app keeps its workstation shell when Windows
 * snaps the window into a narrow or portrait viewport; iOS and Android have
 * their own native shells.
 */
export const STACKED_QUERY =
  "(max-width: 600px), (orientation: portrait) and (max-width: 1100px)";

export type FormFactor = "mobile" | "workstation";

export function formFactorFor(
  platform: Platform,
  stackedViewport: boolean,
): FormFactor {
  return platform === "browser-preview" && stackedViewport
    ? "mobile"
    : "workstation";
}

const currentPlatform = (): Platform =>
  typeof window === "undefined" ? "browser-preview" : detectPlatform();

const media = () => {
  if (
    currentPlatform() !== "browser-preview" ||
    typeof window === "undefined" ||
    typeof window.matchMedia !== "function"
  )
    return undefined;
  return window.matchMedia(STACKED_QUERY);
};

export const isStacked = () => {
  const platform = currentPlatform();
  const stackedViewport =
    platform === "browser-preview" ? (media()?.matches ?? false) : false;
  return formFactorFor(platform, stackedViewport) === "mobile";
};

const subscribe = (changed: () => void) => {
  const query = media();
  query?.addEventListener?.("change", changed);
  return () => query?.removeEventListener?.("change", changed);
};

export function useFormFactor(): FormFactor {
  return useSyncExternalStore(subscribe, () =>
    isStacked() ? "mobile" : "workstation",
  );
}
