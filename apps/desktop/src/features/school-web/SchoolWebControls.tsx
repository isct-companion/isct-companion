import { useEffect, useState, type ReactNode } from "react";
import { useInRouterContext } from "react-router";
import { UiButton } from "../../components/ui";
import { t, type MessageKey } from "../../i18n";
import { desktopBridge } from "../../app/native";
import { useAnimatedNavigate } from "../../app/navigation";
import { paths } from "../../app/routes";
import type {
  SchoolLoadFailure,
  SchoolSite,
  SchoolWebNotice,
  SchoolWebPage,
  SchoolWebProblem,
} from "../../platform/desktop-bridge";

/** Core's one-time notes about the page on screen. */
const noticeText: Record<SchoolWebNotice["kind"], MessageKey> = {
  reopened: "web.reopenedThisPageWithoutSending",
};

/** What the line under the address says when Core's sign-in of the view
 * stopped, or ended with something to say. */
const problemText: Record<SchoolWebProblem, MessageKey> = {
  unreachable: "web.signIn.unreachable",
  rejected: "web.signIn.rejected",
  storage: "web.signIn.storage",
  "still-sign-in": "web.signIn.stillSignIn",
  "submission-unconfirmed": "web.signIn.submissionUnconfirmed",
};

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
 * front with its close button, Core signing the view in (the native view is
 * out of sight meanwhile) with "Cancel, I'll sign in myself", why that
 * sign-in stopped, a failed load with Retry and Open in browser (all from
 * Core's page, so every surface shows the same), and Core's one-time note
 * that a reload was sent without the form. They sit outside the element the
 * native view covers.
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
  const [notice, setNotice] = useState<SchoolWebNotice["kind"]>();
  useEffect(() => {
    let clear: number | undefined;
    const dispose = desktopBridge?.onSchoolWebNotice?.((next) => {
      if (next.site !== site) return;
      window.clearTimeout(clear);
      setNotice(next.kind);
      clear = window.setTimeout(() => setNotice(undefined), 6000);
    });
    return () => {
      dispose?.();
      window.clearTimeout(clear);
      setNotice(undefined);
    };
  }, [site]);
  const routed = useInRouterContext();
  const failure = page?.notice;
  const access = page?.access;
  return (
    <>
      {access?.kind === "signing-in" && (
        <div className="webNotice" role="status" aria-busy="true">
          <span>{t("web.signIn.signingIn")}</span>
          <UiButton
            type="button"
            onClick={() =>
              void desktopBridge?.cancelSchoolWebSignIn?.(site).catch(onError)
            }
          >
            {t("web.signIn.cancel")}
          </UiButton>
        </div>
      )}
      {access?.kind === "problem" && (
        <div
          className={
            access.problem === "submission-unconfirmed"
              ? "webNotice warning"
              : "webNotice"
          }
          role="status"
        >
          <span>{t(problemText[access.problem])}</span>
          {access.problem === "rejected" && routed && <OpenAccounts />}
        </div>
      )}
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
      {notice && (
        <div className="webNotice" role="status">
          <span>{t(noticeText[notice])}</span>
        </div>
      )}
    </>
  );
}

/** Settings › Accounts, where a refused password is updated. Only a surface
 * inside the app's routes has somewhere to go. */
function OpenAccounts() {
  const navigate = useAnimatedNavigate();
  return (
    <UiButton
      type="button"
      onClick={() => void navigate(paths.settings("accounts"))}
    >
      {t("web.signIn.openSettings")}
    </UiButton>
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
