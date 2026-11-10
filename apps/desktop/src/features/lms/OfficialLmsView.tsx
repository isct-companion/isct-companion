import { EmptyState } from "../../components/ui";
import { Globe } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
  claimSchoolView,
  desktopBridge,
  hideNativeSchoolWeb,
  releaseSchoolView,
} from "../../app/native";
import { useConfirmationPending } from "../../components/confirm";
import { ExternalLink } from "../../components/ExternalLink";
import { t } from "../../i18n";
import {
  boundsOf,
  followBounds,
  SchoolWebNavigation,
  SchoolWebNotices,
  useSchoolWebPage,
} from "../school-web";
import { LmsIcon } from "./LmsIcon";
import { ReaderTools } from "./ReaderPane";

/** Each mounted reader presents the LMS view as a surface of its own. */
let readers = 0;

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
  const [surface] = useState(() => `reader-${++readers}`);
  /** The URL this reader has gone to; showing it again is a return. */
  const opened = useRef<string | undefined>(undefined);
  const [status, setStatus] = useState(() => t("lms.openingLMS"));
  const page = useSchoolWebPage(
    available && active ? "science-lms" : undefined,
  );
  const location = page?.url ?? url;
  useEffect(() => {
    if (!active || !available || !desktopBridge) return;
    let disposed = false;
    // Set once the native view is open; bounds follow it from then on.
    let ready = false;
    void (async () => {
      // Await any previous owner hiding its native view before opening ours.
      await hideNativeSchoolWeb();
      if (disposed) return;
      claimSchoolView(owner.current);
      // Coming back to the activity finds it as it was left, popups and a
      // form in progress included, unless another surface showed the view
      // meanwhile; Core decides.
      await desktopBridge!.openSchoolSite(
        "science-lms",
        "",
        "",
        boundsOf(host.current),
        surface,
        opened.current === url ? { kind: "return", url } : { kind: "go", url },
      );
      opened.current = url;
      ready = true;
      if (!disposed) setStatus("");
      else await releaseSchoolView(owner.current);
    })().catch((error) => {
      if (!disposed) setStatus(String(error));
    });
    const stop = followBounds(
      () => host.current,
      () => ready,
    );
    return () => {
      disposed = true;
      stop();
      void releaseSchoolView(owner.current).catch(() => undefined);
    };
  }, [url, active, available, surface]);
  const shown = new URL(location);
  const failed = (error: unknown) => setStatus(String(error));
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
            {available && (
              <SchoolWebNavigation
                site="science-lms"
                page={page}
                icons={{
                  back: <LmsIcon name="back" size={15} />,
                  forward: <LmsIcon name="back" size={15} />,
                  reload: <LmsIcon name="refresh" size={15} />,
                }}
                className={{ back: "", forward: "lmsWebForward", reload: "" }}
                onError={failed}
              />
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
      {available && active && (
        <SchoolWebNotices site="science-lms" page={page} onError={failed} />
      )}
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
