import { useSyncExternalStore } from "react";

/**
 * Phones, and tablets held upright, get the single-column mobile shell; wide
 * landscape windows keep the two-column workstation. Keep in sync with the
 * `@media` blocks in src/layouts/compact/mobile.css.
 */
export const STACKED_QUERY =
  "(max-width: 600px), (orientation: portrait) and (max-width: 1100px)";

export type FormFactor = "mobile" | "workstation";

const media = () =>
  typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia(STACKED_QUERY)
    : undefined;

export const isStacked = () => media()?.matches ?? false;

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
