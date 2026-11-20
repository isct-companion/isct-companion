import { SchoolStatusLine } from "./SchoolStatusLine";
import { notify } from "../components/notifications/notify";
import { UiButton } from "../components/ui";
import { t } from "../i18n";
import { desktopBridge } from "../platform/native";
import { useState } from "react";
import { schoolSessions } from "../platform/school-session";
import type {
  SchoolAuthProfile,
  SchoolProfileSession,
} from "../platform/school-session-contract";
import { sessionDetails, sessionText } from "../lib/sync-text";
import type { SyncSnapshot } from "../platform/sync";

/** One account in the footer: a dot by state, its label, the reason on
 * hover, and Retry while the account waits on the person. */
function SchoolStatusAccount({
  profile,
  name,
  session,
  locale,
}: {
  profile: SchoolAuthProfile;
  name: string;
  session: SchoolProfileSession | undefined;
  locale: string;
}) {
  const [retrying, setRetrying] = useState(false);
  const retry = () => {
    if (!desktopBridge) return;
    setRetrying(true);
    void schoolSessions
      .retry(desktopBridge, profile)
      .catch(notify.error)
      .finally(() => setRetrying(false));
  };
  const label = session
    ? [
        sessionText(session, locale),
        session.maintenance ? t("common.underMaintenance") : "",
      ]
        .filter(Boolean)
        .join(" · ")
    : t("app.checkingSession");
  return (
    <span
      className="schoolStatusAccount"
      data-state={session?.state ?? "checking"}
      data-maintenance={session?.maintenance ? "" : undefined}
      title={
        session
          ? [label, ...sessionDetails(profile, session, locale)].join("\n")
          : undefined
      }
    >
      <i aria-hidden="true" />
      <span>
        <b>{name}</b>
        <small>{label}</small>
      </span>
      {session?.presentation.recovery.includes("retry") && (
        <UiButton
          type="button"
          className="textButton"
          disabled={retrying}
          title={
            session.state === "refused"
              ? t("schoolSession.retry.otherDevice")
              : undefined
          }
          onClick={retry}
        >
          {t("common.retry")}
        </UiButton>
      )}
    </span>
  );
}

export function SchoolStatusBar({
  scienceSession,
  tokyoTechSession,
  snapshot,
  locale,
  refresh,
}: {
  scienceSession: SchoolProfileSession | undefined;
  tokyoTechSession: SchoolProfileSession | undefined;
  snapshot: SyncSnapshot;
  locale: string;
  refresh?: () => void;
}) {
  return (
    <aside
      className="schoolStatusBar"
      aria-label={t("app.schoolConnectionStatus")}
    >
      <div className="schoolStatusAccounts">
        <SchoolStatusAccount
          profile="science"
          name="Science Tokyo"
          session={scienceSession}
          locale={locale}
        />
        <SchoolStatusAccount
          profile="tokyo-tech"
          name="Tokyo Tech"
          session={tokyoTechSession}
          locale={locale}
        />
      </div>
      <SchoolStatusLine
        snapshot={snapshot}
        locale={locale}
        refresh={refresh}
      />
    </aside>
  );
}
