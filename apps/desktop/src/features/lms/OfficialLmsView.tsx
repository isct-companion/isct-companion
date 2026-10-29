import { EmptyState } from "../../components/ui";
import { Globe } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
  claimSchoolView,
  desktopBridge,
  hideNativeSchoolWeb,
  releaseSchoolView,
  schoolWebExit,
} from "../../app/native";
import { schoolWebBounds } from "../../platform/school-web-bounds";
import { useConfirmationPending } from "../../components/confirm";
import { ExternalLink } from "../../components/ExternalLink";
import { t } from "../../i18n";
import { LmsIcon } from "./LmsIcon";
import { ReaderTools } from "./ReaderPane";

/** Same resident LMS WebView and login flow, bounded to this viewer only. */
export function OfficialLmsView({
  url,
  active: visible,
  available,
}: {
  url: string;
  active: boolean;
  available: boolean;
}) {
  // The native view draws above every HTML overlay, so it hides while a
  // confirmation asks and returns to the same page afterwards.
  const confirming = useConfirmationPending();
  const active = visible && !confirming;
  const host = useRef<HTMLDivElement>(null);
  const owner = useRef(Symbol("today-lms"));
  const opened = useRef(false);
  const [status, setStatus] = useState(() => t("lms.openingLMS"));
  const [location, setLocation] = useState(url);
  useEffect(() => {
    setLocation(url);
    if (!active) return;
    return desktopBridge?.onSchoolWebUrl?.((event) => {
      if (event.site === "science-lms") setLocation(event.url);
    });
  }, [url, active]);
  useEffect(() => {
    if (!active || !available || !desktopBridge) return;
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
          current.searchParams.get("id") !== target.searchParams.get("id") ||
          (target.searchParams.get("action") === "editsubmission" &&
            current.searchParams.get("action") !== "editsubmission")
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
  }, [url, active, available]);
  const shown = new URL(location);
  const navigate = (move?: (site: "science-lms") => Promise<boolean>) =>
    void move?.("science-lms").catch(() => undefined);
  return (
    <>
      {/* The page's address and the browser's own controls, in the reader's
          tool row as a file's tools are. */}
      <ReaderTools>
        <div className="lmsSourceBar lmsFileToolbar lmsWebToolbar">
          <div className="lmsWebAddress" title={location}>
            <span>{shown.hostname}</span>
            {shown.pathname !== "/" && <small>{shown.pathname}</small>}
          </div>
          <div className="lmsFileActions">
            {available && desktopBridge?.schoolWebBack && (
              <button
                type="button"
                title={t("common.previousPage")}
                aria-label={t("common.previousPage")}
                onClick={() => navigate(desktopBridge?.schoolWebBack)}
              >
                <LmsIcon name="back" size={15} />
              </button>
            )}
            {available && desktopBridge?.schoolWebForward && (
              <button
                type="button"
                className="lmsWebForward"
                title={t("common.nextPage")}
                aria-label={t("common.nextPage")}
                onClick={() => navigate(desktopBridge?.schoolWebForward)}
              >
                <LmsIcon name="back" size={15} />
              </button>
            )}
            {available && desktopBridge?.reloadSchoolWeb && (
              <button
                type="button"
                title={t("common.reload")}
                aria-label={t("common.reload")}
                onClick={() => navigate(desktopBridge?.reloadSchoolWeb)}
              >
                <LmsIcon name="refresh" size={15} />
              </button>
            )}
            <ExternalLink
              href={location}
              title={t("common.openInBrowser")}
              aria-label={t("common.openInBrowser")}
            >
              <LmsIcon name="external" size={15} />
            </ExternalLink>
          </div>
        </div>
      </ReaderTools>
      <div className="lmsOfficialHost" ref={host}>
        {!available ? (
          <EmptyState
            icon={Globe}
            action={
              <a href={url} target="_blank" rel="noopener noreferrer">
                {t("lms.openLMSInBrowser")} ↗
              </a>
            }
          >
            {t("lms.theDesktopAppOpensThe")}
          </EmptyState>
        ) : (
          status && <p role="status">{status}</p>
        )}
      </div>
    </>
  );
}
