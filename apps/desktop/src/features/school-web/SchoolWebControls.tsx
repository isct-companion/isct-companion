import { useEffect, useState, type ReactNode } from "react";
import { DownloadNotice, WebNotice } from "../../components/DownloadNotice";
import { UiButton } from "../../components/ui";
import { t, type MessageKey } from "../../i18n";
import { desktopBridge } from "../../platform/native";
import type { SchoolSite } from "../../generated/persistence";
import {
  type SchoolDownloadEvent,
  type SchoolLoadFailure,
  type SchoolOtpEnrollment,
  type SchoolOtpIntent,
  type SchoolWebNotice,
  type SchoolWebPage,
  type SchoolWebProblem,
} from "../../platform/desktop-bridge";

/** Core's one-time notes about the page on screen. */
const noticeText: Record<SchoolWebNotice["kind"], MessageKey> = {
  reopened: "web.reopenedThisPageWithoutSending",
};

/** What the line under the address says when Core's sign-in of the view
 * stopped. */
const problemText: Record<SchoolWebProblem, MessageKey> = {
  unreachable: "web.signIn.unreachable",
  rejected: "web.signIn.rejected",
  storage: "web.signIn.storage",
  "still-sign-in": "web.signIn.stillSignIn",
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
 * sign-in stopped, the button that signs a stuck sign-in page in, a failed load with Retry and Open in browser (all from
 * Core's page, so every surface shows the same), Core's one-time note
 * that a reload was sent without the form, and a file being saved to
 * Downloads, then saved with its Reveal button. They sit outside the element
 * the native view covers, which a toast at the bottom of the window does not.
 */
export function SchoolWebNotices({
  site,
  page,
  onError,
  openAccounts,
}: {
  site: SchoolSite;
  page: SchoolWebPage | undefined;
  onError: (error: unknown) => void;
  openAccounts?: () => void;
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
  const [download, setDownload] = useState<SchoolDownloadEvent>();
  useEffect(() => {
    let clear: number | undefined;
    const dispose = desktopBridge?.onSchoolDownload?.((event) => {
      if (event.site !== site) return;
      window.clearTimeout(clear);
      setDownload(event);
      if (event.state !== "started")
        clear = window.setTimeout(() => setDownload(undefined), 30_000);
    });
    return () => {
      dispose?.();
      window.clearTimeout(clear);
      setDownload(undefined);
    };
  }, [site]);
  const failure = page?.notice;
  const access = page?.access;
  return (
    <>
      {page?.otp && (
        <OtpNotice
          key={page.otp.id}
          otp={page.otp}
          onError={onError}
          openAccounts={openAccounts}
        />
      )}
      {access?.kind === "signing-in" && (
        <WebNotice aria-busy="true">
          <span>{t("web.signIn.signingIn")}</span>
          <UiButton
            type="button"
            onClick={() =>
              void desktopBridge?.cancelSchoolWebSignIn?.(site).catch(onError)
            }
          >
            {t("web.signIn.cancel")}
          </UiButton>
        </WebNotice>
      )}
      {access?.kind === "problem" && (
        <WebNotice>
          <span>{t(problemText[access.problem])}</span>
          {access.problem === "rejected" && openAccounts && (
            <OpenAccounts openAccounts={openAccounts} />
          )}
        </WebNotice>
      )}
      {page?.signIn && (
        <WebNotice>
          <span>{t("web.signIn.asks")}</span>
          <UiButton
            type="button"
            onClick={() =>
              void desktopBridge?.reloadSchoolWeb?.(site).catch(onError)
            }
          >
            {t("web.signIn.useSaved")}
          </UiButton>
        </WebNotice>
      )}
      {page && page.popups > 0 && (
        <WebNotice>
          <span>{t("web.popupWindow")}</span>
          <UiButton
            type="button"
            onClick={() =>
              void desktopBridge?.closeSchoolWebPopup?.(site).catch(onError)
            }
          >
            {t("web.closeWindow")}
          </UiButton>
        </WebNotice>
      )}
      {failure && (
        <WebNotice warning>
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
        </WebNotice>
      )}
      {notice && (
        <WebNotice>
          <span>{t(noticeText[notice])}</span>
        </WebNotice>
      )}
      {download && <DownloadNotice download={download} onError={onError} />}
    </>
  );
}

const otpMessages: Record<SchoolOtpEnrollment["stage"], MessageKey> = {
  "sign-in": "otpEnrollment.signIn",
  checking: "otpEnrollment.checking",
  "setting-up": "otpEnrollment.settingUp",
  saving: "otpEnrollment.saving",
  complete: "otpEnrollment.complete",
  "account-mismatch": "otpEnrollment.accountMismatch",
  incomplete: "otpEnrollment.incomplete",
  unknown: "otpEnrollment.unknown",
  "save-failed": "otpEnrollment.saveFailed",
};

function OtpNotice({
  otp,
  onError,
  openAccounts,
}: {
  otp: SchoolOtpEnrollment;
  onError: (error: unknown) => void;
  openAccounts?: () => void;
}) {
  const send = (intent: SchoolOtpIntent) =>
    desktopBridge?.schoolOtpIntent?.(otp.id, intent).catch(onError);
  const active = ["sign-in", "checking", "setting-up"].includes(otp.stage);
  return (
    <WebNotice className="otpEnrollmentNotice">
      <span>{t(otpMessages[otp.stage], { account: otp.owner ?? "" })}</span>
      {openAccounts && otp.stage === "complete" && (
        <OpenAccounts
          label="otpEnrollment.settings"
          openAccounts={openAccounts}
        />
      )}
      {otp.stage === "save-failed" && (
        <UiButton type="button" onClick={() => void send("retry-save")}>
          {t("otpEnrollment.retrySave")}
        </UiButton>
      )}
      {otp.stage !== "saving" && (
        <UiButton type="button" onClick={() => void send("stop")}>
          {t(active ? "common.cancel" : "common.done")}
        </UiButton>
      )}
    </WebNotice>
  );
}

/** Settings › Accounts, where a refused password is updated. Only a surface
 * inside the app's routes has somewhere to go. */
function OpenAccounts({
  label = "web.signIn.openSettings",
  openAccounts,
}: {
  label?: MessageKey;
  openAccounts: () => void;
}) {
  return (
    <UiButton type="button" onClick={openAccounts}>
      {t(label)}
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
