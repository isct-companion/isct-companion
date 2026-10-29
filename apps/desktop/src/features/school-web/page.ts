import { useEffect, useState } from "react";
import { desktopBridge, schoolWebExit } from "../../app/native";
import type { SchoolSite, SchoolWebPage } from "../../platform/desktop-bridge";
import { schoolWebBounds } from "../../platform/school-web-bounds";

/**
 * The page Core says the person sees of `site`, for every surface that shows
 * its view. The listener is registered before the snapshot is read, so no
 * change falls between the two; of the snapshot and the events, the highest
 * revision wins. Effects end while the surface is hidden (`<Activity>`), and
 * coming back reads the snapshot again.
 */
export function useSchoolWebPage(site: SchoolSite | undefined) {
  const [page, setPage] = useState<SchoolWebPage>();
  useEffect(() => {
    const bridge = desktopBridge;
    if (!site || !bridge?.onSchoolWebPage || !bridge.schoolWebPage) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const keep = (next: SchoolWebPage) => {
      if (disposed || next.site !== site) return;
      setPage((current) =>
        current?.site === site && current.revision >= next.revision
          ? current
          : next,
      );
    };
    void bridge
      .onSchoolWebPage(keep)
      .then((dispose) => {
        if (disposed) return dispose();
        unlisten = dispose;
        return bridge.schoolWebPage!(site).then(keep);
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [site]);
  return page?.site === site ? page : undefined;
}

/**
 * Keeps the native school view over `element` wherever it actually is. A
 * resize is not the only way it moves (a pane slides in, an outline reflows,
 * a notice appears above it), so its rectangle is read every frame and sent
 * only when it changed, once `ready` says the view is open. Returns the stop.
 */
export function followBounds(
  element: () => HTMLElement | null,
  ready: () => boolean,
) {
  let sent = "";
  let frame = 0;
  const follow = () => {
    const rect = element()?.getBoundingClientRect();
    const next = rect ? schoolWebBounds(rect, window) : undefined;
    const key = next ? JSON.stringify(next) : "";
    if (ready() && next && key !== sent && !schoolWebExit.pending) {
      sent = key;
      void desktopBridge?.setSchoolWebBounds?.(next).catch(() => (sent = ""));
    }
    frame = requestAnimationFrame(follow);
  };
  frame = requestAnimationFrame(follow);
  return () => cancelAnimationFrame(frame);
}

/** The rectangle `element` gives the native view now, for opening it. */
export const boundsOf = (element: HTMLElement | null) =>
  element
    ? schoolWebBounds(element.getBoundingClientRect(), window)
    : undefined;
