import type {
  SchoolAuthProfile,
  SchoolProfileSession,
  SyncSnapshot,
} from "../generated/persistence";
import { RefreshCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { SchoolStatusLine } from "./SchoolStatusLine";
import { notify } from "../components/notifications/notify";
import { Popover, UiButton } from "../components/ui";
import { t } from "../i18n";
import { desktopBridge } from "../platform/native";
import { progressLine } from "../platform/sync";
import { useClock } from "../platform/use-clock";
import { schoolSessions } from "../platform/school-session";
import { sessionDetails, sessionText, statusOverview } from "../lib/sync-text";

/** One account in the status panel: a dot by state, its label, the reason
 * on hover, and Retry while the account waits on the person. */
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

/** The workstation's status, floating at the bottom right: one line that
 * sums up the school sites and a Refresh. Pointing at the line (or clicking
 * it, for the keyboard) opens the panel with each account, each source's
 * age, what is running and every failure with its Retry. */
export function SchoolStatusPanel({
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
  const line = progressLine(snapshot);
  const now = useClock(line.kind === "idle");
  const overview = statusOverview(
    snapshot,
    line,
    [scienceSession, tokyoTechSession],
    now,
    locale,
  );
  const [open, setOpen] = useState(false);
  // A click opens the panel for the keyboard too, so focus moves into it;
  // pointing only shows it.
  const [focusPanel, setFocusPanel] = useState(false);
  const panel = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const closing = useRef<number | undefined>(undefined);
  const show = () => {
    window.clearTimeout(closing.current);
    setOpen(true);
  };
  // A short grace lets the pointer cross from the line into the panel.
  const hide = () => {
    window.clearTimeout(closing.current);
    closing.current = window.setTimeout(() => setOpen(false), 200);
  };
  useEffect(() => () => window.clearTimeout(closing.current), []);
  useEffect(() => {
    if (open && focusPanel) panel.current?.focus();
    if (!open) setFocusPanel(false);
  }, [open, focusPanel]);
  return (
    <aside
      className="schoolStatusPanel"
      aria-label={t("app.schoolConnectionStatus")}
    >
      <Popover
        opened={open}
        onChange={(next) => {
          window.clearTimeout(closing.current);
          setOpen(next);
        }}
        onDismiss={() => button.current?.focus()}
        position="top-end"
        offset={8}
        width={380}
      >
        <Popover.Anchor>
          <button
            ref={button}
            type="button"
            className="schoolStatusOverview"
            data-kind={overview.kind}
            aria-haspopup="dialog"
            aria-expanded={open}
            onPointerEnter={show}
            onPointerLeave={hide}
            onClick={() => {
              show();
              setFocusPanel(true);
            }}
          >
            {overview.kind === "busy" ? (
              <i className="schoolStatusSpinner" aria-hidden="true" />
            ) : (
              <i className="schoolStatusDot" aria-hidden="true" />
            )}
            <span>{overview.text}</span>
          </button>
        </Popover.Anchor>
        <Popover.Dropdown
          className="schoolStatusDetails"
          onPointerEnter={show}
          onPointerLeave={hide}
        >
          <div ref={panel} tabIndex={-1} className="schoolStatusDetailsBody">
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
            <SchoolStatusLine snapshot={snapshot} locale={locale} />
          </div>
        </Popover.Dropdown>
      </Popover>
      {refresh && (
        <UiButton
          type="button"
          className="schoolStatusRefresh"
          aria-label={t("common.refreshNow")}
          title={t("common.refreshNow")}
          disabled={snapshot.resetting}
          onClick={refresh}
        >
          <RefreshCw
            size={14}
            strokeWidth={1.75}
            aria-hidden="true"
            focusable="false"
          />
        </UiButton>
      )}
    </aside>
  );
}
