import { useEffect, useState, type ReactNode } from "react";
import { UiButton } from "../../components/ui";
import { t, type MessageKey } from "../../i18n";
import { desktopBridge } from "../../app/native";
import type {
  SchoolLoadFailure,
  SchoolSite,
  SchoolWebPage,
} from "../../platform/desktop-bridge";

/** What the notice under the address says when a page did not load. */
const failureText: Record<SchoolLoadFailure, MessageKey> = {
  offline: "web.thisPageCouldNotBe",
  "host-not-found": "web.thisPageCouldNotBeOpened",
  unreachable: "web.thisPageCouldNotBeOpenedIts",
  insecure: "web.thisPageCouldNotBeOpenedThe",
  cleartext: "web.thisPageUsesAnUnencrypted",
  other: "web.thisPageCouldNotBeOpened2",
};

/**
 * The lines between a surface's toolbar and the school page: the popup in
 * front with its close button, a failed load with Retry and Open in browser
 * (both from Core's page, so every surface shows the same), and Core's
 * one-time note that a reload was sent without the form. They sit outside
 * the element the native view covers.
 */
export function SchoolWebNotices({
  site,
  page,
  onError,
}: {
  site: SchoolSite;
  page: SchoolWebPage | undefined;
  onError: (error: unknown) => void;
}) {
  const [reopened, setReopened] = useState(false);
  useEffect(() => {
    let clear: number | undefined;
    const dispose = desktopBridge?.onSchoolWebNotice?.((notice) => {
      if (notice.site !== site) return;
      window.clearTimeout(clear);
      setReopened(true);
      clear = window.setTimeout(() => setReopened(false), 6000);
    });
    return () => {
      dispose?.();
      window.clearTimeout(clear);
      setReopened(false);
    };
  }, [site]);
  const failure = page?.notice;
  return (
    <>
      {page && page.popups > 0 && (
        <div className="webNotice" role="status">
          <span>{t("web.popupWindow")}</span>
          <UiButton
            type="button"
            onClick={() =>
              void desktopBridge?.closeSchoolWebPopup?.(site).catch(onError)
            }
          >
            {t("web.closeWindow")}
          </UiButton>
        </div>
      )}
      {failure && (
        <div className="webNotice warning" role="alert">
          <span>{t(failureText[failure.reason])}</span>
          {failure.url && failure.reason !== "cleartext" && (
            <UiButton
              type="button"
              onClick={() =>
                void desktopBridge
                  ?.navigateSchoolWeb?.(site, failure.url!)
                  .catch(onError)
              }
            >
              {t("common.retry")}
            </UiButton>
          )}
          {failure.url && (
            <UiButton
              type="button"
              onClick={() =>
                void desktopBridge?.openExternal(failure.url!).catch(onError)
              }
            >
              {t("common.openInBrowser")}
            </UiButton>
          )}
        </div>
      )}
      {reopened && (
        <div className="webNotice" role="status">
          <span>{t("web.reopenedThisPageWithoutSending")}</span>
        </div>
      )}
    </>
  );
}

/**
 * Back, Forward and Reload for the page in front. Core decides what each
 * does (Back closes a popup at the start of its history); a button is
 * disabled only when Core says it would do nothing.
 */
export function SchoolWebNavigation({
  site,
  page,
  icons,
  className,
  onError,
}: {
  site: SchoolSite;
  page: SchoolWebPage | undefined;
  icons: { back: ReactNode; forward: ReactNode; reload: ReactNode };
  /** The surface's own button look. */
  className: { back: string; forward: string; reload: string };
  onError: (error: unknown) => void;
}) {
  const run = (step?: (site: SchoolSite) => Promise<boolean>) =>
    void step?.(site).catch(onError);
  return (
    <>
      {desktopBridge?.schoolWebBack && (
        <UiButton
          type="button"
          className={className.back}
          title={t("common.previousPage")}
          aria-label={t("common.previousPage")}
          disabled={page?.back === "no"}
          onClick={() => run(desktopBridge?.schoolWebBack)}
        >
          {icons.back}
        </UiButton>
      )}
      {desktopBridge?.schoolWebForward && (
        <UiButton
          type="button"
          className={className.forward}
          title={t("common.nextPage")}
          aria-label={t("common.nextPage")}
          disabled={page?.forward === "no"}
          onClick={() => run(desktopBridge?.schoolWebForward)}
        >
          {icons.forward}
        </UiButton>
      )}
      {desktopBridge?.reloadSchoolWeb && (
        <UiButton
          type="button"
          className={className.reload}
          title={t("common.reload")}
          aria-label={t("common.reload")}
          onClick={() => run(desktopBridge?.reloadSchoolWeb)}
        >
          {icons.reload}
        </UiButton>
      )}
    </>
  );
}
