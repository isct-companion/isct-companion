import { useEffect, useRef, useState } from "react";
import {
  claimSchoolView,
  desktopBridge,
  hideNativeSchoolWeb,
  releaseSchoolView,
  schoolWebExit,
} from "../../app/native";
import { schoolWebBounds } from "../../platform/school-web-bounds";
import { useSchoolFilePreviewOpen } from "../school-preview/SchoolFilePreview";

/** Same resident LMS WebView and login flow, bounded to this viewer only. */
export function OfficialLmsView({
  url,
  active,
  available,
}: {
  url: string;
  active: boolean;
  available: boolean;
}) {
  const host = useRef<HTMLDivElement>(null);
  const owner = useRef(Symbol("today-lms"));
  const opened = useRef(false);
  const [status, setStatus] = useState("Opening LMS…");
  // An attachment preview covers the app; the native view stays hidden under it.
  const previewOpen = useSchoolFilePreviewOpen();
  useEffect(() => {
    if (!active || !available || !desktopBridge || previewOpen) return;
    let disposed = false;
    // Set once the native view is open; bounds follow it from then on.
    let ready = false;
    let sent = "";
    let frame = 0;
    const bounds = () =>
      host.current
        ? schoolWebBounds(host.current.getBoundingClientRect(), window)
        : undefined;
    void (async () => {
      // Await any previous owner hiding its native view before opening ours.
      await hideNativeSchoolWeb();
      if (disposed) return;
      claimSchoolView(owner.current);
      const state = await desktopBridge!.openSchoolSite(
        "science-lms",
        "",
        "",
        bounds(),
        opened.current ? undefined : url,
      );
      if (!disposed && opened.current) {
        const current = new URL(state.url);
        const target = new URL(url);
        // Re-show the resident submission page, including an in-progress
        // official form, when it still belongs to this activity.
        if (
          current.hostname !== target.hostname ||
          current.searchParams.get("id") !== target.searchParams.get("id")
        )
          await desktopBridge?.navigateSchoolWeb?.("science-lms", url);
      }
      opened.current = true;
      ready = true;
      if (!disposed) setStatus("");
      else await releaseSchoolView(owner.current);
    })().catch((error) => {
      if (!disposed) setStatus(String(error));
    });
    // The native view sits over this element wherever it actually is. A
    // resize is not the only way it moves: the reader slides in, the outline
    // reflows, the rail collapses. So its rectangle is read every frame and
    // sent only when it changed.
    const follow = () => {
      const next = bounds();
      const key = next ? JSON.stringify(next) : "";
      if (ready && next && key !== sent && !schoolWebExit.pending) {
        sent = key;
        void desktopBridge?.setSchoolWebBounds?.(next).catch(() => (sent = ""));
      }
      frame = requestAnimationFrame(follow);
    };
    frame = requestAnimationFrame(follow);
    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      void releaseSchoolView(owner.current).catch(() => undefined);
    };
  }, [url, active, available, previewOpen]);
  return (
    <div className="lmsOfficialHost" ref={host}>
      {!available ? (
        <div className="lmsViewerEmpty">
          <b>Official LMS</b>
          <p>
            The desktop app opens the signed-in LMS here. Browser preview cannot
            embed LMS.
          </p>
          <a href={url} target="_blank" rel="noopener noreferrer">
            Open LMS in browser ↗
          </a>
        </div>
      ) : (
        status && <p role="status">{status}</p>
      )}
    </div>
  );
}
