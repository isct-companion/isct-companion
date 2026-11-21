import type {
  SchoolProfileSession,
  SyncSnapshot,
} from "../generated/persistence";
import { notify } from "../components/notifications/notify";
import { UiButton } from "../components/ui";
import { t } from "../i18n";
import { desktopBridge } from "../platform/native";
import { popupNativeMenu } from "../platform/native-menu";
import { progressLine } from "../platform/sync";
import { retry } from "../platform/sync-intents";
import { schoolSessions } from "../platform/school-session";
import {
  statusMenu,
  statusOverview,
  type StatusMenuAction,
} from "../lib/sync-text";

/** The school status at the very foot of the rail: one dot whose colour
 * says whether the school sites are connected, updating, paused or need the
 * person. It opens a native menu (`statusMenu`) with each account, what is
 * running, each failure with its Retry, and Refresh. */
export function SchoolStatusDot({
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
  const overview = statusOverview(
    snapshot,
    line,
    [scienceSession, tokyoTechSession],
    locale,
  );
  const run = (action: StatusMenuAction) => {
    switch (action.kind) {
      case "retryAccount":
        if (desktopBridge)
          void schoolSessions
            .retry(desktopBridge, action.profile)
            .catch(notify.error);
        return;
      case "retrySync":
        retry(action.plan);
        return;
      case "refresh":
        refresh?.();
    }
  };
  const open = () =>
    void popupNativeMenu(
      statusMenu(
        snapshot,
        line,
        { science: scienceSession, "tokyo-tech": tokyoTechSession },
        locale,
        refresh !== undefined,
      ).map((entry) =>
        entry.kind === "action"
          ? { kind: "action", text: entry.text, run: () => run(entry.action) }
          : entry,
      ),
    ).catch(notify.error);
  return (
    <UiButton
      type="button"
      className="schoolStatusDot"
      data-kind={overview.kind}
      aria-haspopup="menu"
      aria-label={`${t("app.schoolConnectionStatus")}: ${overview.text}`}
      title={overview.text}
      onClick={open}
    >
      <i aria-hidden="true" />
    </UiButton>
  );
}
