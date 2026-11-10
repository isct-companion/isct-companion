import { workspaceActions } from "../../store/workspace";
import { notify as appNotify } from "../../app/notify";
import { Fragment, useState, type ReactNode } from "react";
import { refresh, useSync } from "../../app/useSync";
import { runsFor } from "../../platform/sync";
import { t, type MessageKey } from "../../i18n";
import type { Subscription, Workspace } from "../../lib/model";
import { UiButton, UiInput } from "../../components/ui";
import {
  Row,
  SettingsGroup,
  SettingsHeader,
  Toggle,
} from "../../components/forms";
import {
  PUBLIC_DATA_INDEX,
  REFERENCE_KINDS,
} from "../../generated/persistence";
import type { Platform } from "../../app/shared";
import {
  formatDay,
  subscriptionRow,
  useSubscriptionViews,
} from "./subscription-status";

type Update = (f: (d: Workspace) => Workspace) => void;

const DATASETS: Record<
  Subscription["kind"],
  { title: MessageKey; follows: "academic" | "admission" }
> = {
  calendar: {
    title: "settings.academicCalendar.academicCalendar",
    follows: "academic",
  },
  departments: { title: "webRouter.studyGuide", follows: "admission" },
  "important-dates": { title: "today.importantDates", follows: "academic" },
  exams: { title: "settings.dataUpdates.examTimetable", follows: "academic" },
  "class-schedules": {
    title: "settings.dataUpdates.alternatingClassSchedules",
    follows: "academic",
  },
};

/** Official reference data from the public Isct-Data repository. */
export function DataUpdatesSettings({
  data,
  update,
  platform,
  refreshReferences,
  tasks,
}: {
  data: Workspace;
  /** The scheduled tasks, which keep this data and school records fresh. */
  tasks?: ReactNode;
  /** Device preferences. */
  update: Update;
  platform: Platform;
  /** Whether this shell can ask Core to check the reference data now. */
  refreshReferences?: boolean;
}) {
  const sync = useSync();
  const subscriptions = useSubscriptionViews();
  const locale = data.settings.locale;
  const editable = platform === "native-desktop";
  // Core's planner decides whether the check runs now or waits for Wi-Fi.
  const busy = runsFor(sync, ["reference"]).some((run) => !run.held);
  const waitingForWifi =
    sync.sources.find((source) => source.source === "references")?.held
      ?.kind === "cellular";
  const lastChecked = data.settings.subscriptionRefresh.lastCheckedAt;
  const checkNow = () => refresh({ kind: "references" });
  return (
    <>
      <SettingsHeader
        title="settings.main.dataUpdates"
        actions={
          refreshReferences && (
            <UiButton
              className="button small"
              disabled={busy}
              onClick={checkNow}
            >
              {busy
                ? t("settings.academicCalendar.checking")
                : t("settings.academicCalendar.checkNow")}
            </UiButton>
          )
        }
      />
      {tasks}
      <SettingsGroup
        title="settings.dataUpdates.officialData"
        footer={
          <>
            {lastChecked
              ? t("settings.dataUpdates.lastCheckedDate", {
                  date: formatDay(lastChecked, locale, {
                    hour: "2-digit",
                    minute: "2-digit",
                  }),
                })
              : t("settings.dataUpdates.notCheckedYet")}
            {" · "}
            <a
              href="https://github.com/isct-companion/Isct-Data"
              target="_blank"
              rel="noreferrer"
            >
              isct-companion/Isct-Data
            </a>
          </>
        }
      >
        {REFERENCE_KINDS.map((kind) => {
          const definition = DATASETS[kind];
          const view = subscriptions.value?.[kind];
          const year = view?.year;
          const state = view?.state ?? { state: "pending" as const };
          const hint =
            state.state === "off"
              ? t("settings.main.off")
              : state.state === "no-year"
                ? t("settings.dataUpdates.setTheYearYouStarted")
                : state.state === "error"
                  ? state.message
                  : state.state === "current"
                    ? `${definition.follows === "admission" ? t("settings.dataUpdates.yearEntry", { year }) : t("timetable.academicYear", { year })} · ${t("syncStatus.updatedDate", { date: formatDay(state.fetchedAt, locale) })}`
                    : `${definition.follows === "admission" ? t("settings.dataUpdates.yearEntry", { year }) : t("timetable.academicYear", { year })} · ${t("settings.dataUpdates.waitingForTheNextCheck")}`;
          return (
            <Row key={kind} id={kind} title={definition.title} hint={hint}>
              <Toggle
                disabled={!editable || !view}
                checked={view?.enabled ?? true}
                onChange={(enabled) =>
                  void workspaceActions
                    .mutate({
                      kind: "setSubscription",
                      subscriptionKind: kind,
                      enabled,
                      resetRemote: false,
                    })
                    .catch(appNotify.error)
                }
              />
            </Row>
          );
        })}
      </SettingsGroup>
      <SettingsGroup>
        <Row
          id="cellular"
          title="settings.main.updateOnCellularData"
          hint={
            waitingForWifi
              ? t("settings.dataUpdates.pausedUntilWiFiIs")
              : undefined
          }
        >
          <Toggle
            checked={data.settings.subscriptionRefresh.allowCellular}
            onChange={(enabled) =>
              update((draft) => {
                draft.settings.subscriptionRefresh.allowCellular = enabled;
                return draft;
              })
            }
          />
        </Row>
      </SettingsGroup>
      {!editable && (
        <p className="settingsNote">
          {refreshReferences
            ? t("settings.dataUpdates.turnIndividualUpdatesOnOr")
            : t("settings.dataUpdates.dataUpdatesRunInThe")}
        </p>
      )}
    </>
  );
}

/** Developer option: where each dataset reads its index from. */
export function DataSourceUrls({ data }: { data: Workspace }) {
  const [urls, setUrls] = useState<Record<string, string>>({});
  return (
    <>
      {REFERENCE_KINDS.map((kind) => {
        const row = subscriptionRow(data, kind);
        const current = row?.url.startsWith("https://")
          ? row.url
          : PUBLIC_DATA_INDEX;
        const url = urls[kind] ?? current;
        return (
          <Fragment key={kind}>
            <label className="configField">
              <span>{t(DATASETS[kind].title)}</span>
              <UiInput
                value={url}
                type="url"
                spellCheck={false}
                onChange={(event) =>
                  setUrls((prior) => ({ ...prior, [kind]: event.target.value }))
                }
                onBlur={() => {
                  if (urls[kind] === undefined || url === row?.url) return;
                  if (!url.startsWith("https://"))
                    return setUrls((prior) => ({ ...prior, [kind]: current }));
                  void workspaceActions
                    .mutate({
                      kind: "setSubscription",
                      subscriptionKind: kind,
                      url,
                      resetRemote: true,
                    })
                    .catch(appNotify.error);
                }}
              />
              {row?.dataUrl && (
                <a href={row.dataUrl} target="_blank" rel="noreferrer">
                  {row.dataUrl}
                </a>
              )}
            </label>
          </Fragment>
        );
      })}
    </>
  );
}
