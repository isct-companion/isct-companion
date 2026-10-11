import { notify as appNotify } from "../../app/notify";
import { Fragment, useState } from "react";
import { t } from "../../i18n";
import type { Subscription, Workspace } from "../../core/model";
import { UiButton, UiInput } from "../../components/mantine";
import {
  Row,
  SettingsGroup,
  SettingsHeader,
  Toggle,
} from "../../components/forms";
import {
  PUBLIC_DATA_INDEX,
  REFERENCE_KINDS,
  subscriptionNetwork,
  subscriptionYear,
} from "../../platform/subscription-polling";
import type { Platform } from "../../app/shared";
import {
  formatDay,
  subscriptionRow,
  subscriptionState,
} from "./subscription-status";

type Update = (f: (d: Workspace) => Workspace) => void;

const DATASETS: Record<
  Subscription["kind"],
  { title: string; follows: "academic" | "admission" }
> = {
  calendar: { title: "Academic calendar", follows: "academic" },
  "important-dates": { title: "Important dates", follows: "academic" },
  exams: { title: "Exam timetable", follows: "academic" },
  departments: { title: "Study guide", follows: "admission" },
};

function saveSubscription(
  update: Update,
  kind: Subscription["kind"],
  values: Partial<Subscription>,
) {
  update((draft) => {
    let row = subscriptionRow(draft, kind);
    const now = new Date().toISOString();
    if (!row) {
      row = {
        id: `subscription:public:${kind}`,
        kind,
        url: PUBLIC_DATA_INDEX,
        enabled: true,
        rev: 0,
        updatedAt: now,
        updatedBy: draft.deviceId,
      };
      draft.subscriptions.push(row);
    }
    Object.assign(row, values, {
      rev: row.rev + 1,
      updatedAt: now,
      updatedBy: draft.deviceId,
    });
    return draft;
  });
}

/** Official reference data from the public Isct-Data repository. */
export function DataUpdatesSettings({
  data,
  update,
  updateSubscriptions,
  platform,
  refreshReferences,
  notify,
}: {
  data: Workspace;
  /** Device preferences. */
  update: Update;
  /** Subscription rows (desktop only). */
  updateSubscriptions: Update;
  platform: Platform;
  refreshReferences?: () => Promise<string>;
  notify: (message: string) => void;
}) {
  const [busy, setBusy] = useState(false);
  const locale = data.settings.locale;
  const editable = platform === "native-desktop";
  const network = subscriptionNetwork(platform);
  const lastChecked = data.settings.subscriptionRefresh.lastCheckedAt;
  const checkNow = async () => {
    if (!refreshReferences) return;
    setBusy(true);
    try {
      notify(t(await refreshReferences()));
    } catch (error) {
      appNotify.error(error);
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <SettingsHeader
        title="Data updates"
        description="Calendars, exam timetables and study guides come from a public, reviewed data set. The app checks for changes once a day."
        actions={
          refreshReferences && (
            <UiButton
              className="button small"
              disabled={busy}
              onClick={checkNow}
            >
              {busy ? t("Checking…") : t("Check now")}
            </UiButton>
          )
        }
      />
      <SettingsGroup
        footer={
          <>
            {lastChecked
              ? t("Last checked {{date}}", {
                  date: formatDay(lastChecked, locale, {
                    hour: "2-digit",
                    minute: "2-digit",
                  }),
                })
              : t("Not checked yet")}
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
          const row = subscriptionRow(data, kind);
          const year = subscriptionYear(data, kind);
          const state = subscriptionState(data, kind);
          const hint =
            state.state === "off"
              ? t("Off")
              : state.state === "no-year"
                ? t("Set the year you started in Study profile")
                : state.state === "error"
                  ? state.message
                  : state.state === "current"
                    ? `${definition.follows === "admission" ? t("{{year}} entry", { year }) : t("AY {{year}}", { year })} · ${t("Updated {{date}}", { date: formatDay(state.fetchedAt, locale) })}`
                    : `${definition.follows === "admission" ? t("{{year}} entry", { year }) : t("AY {{year}}", { year })} · ${t("Waiting for the next check")}`;
          return (
            <Row key={kind} id={kind} title={definition.title} hint={hint}>
              <Toggle
                disabled={!editable}
                checked={row?.enabled ?? true}
                onChange={(enabled) =>
                  saveSubscription(updateSubscriptions, kind, { enabled })
                }
              />
            </Row>
          );
        })}
      </SettingsGroup>
      <SettingsGroup>
        <Row
          id="cellular"
          title="Update on cellular data"
          hint={
            network === "cellular" &&
            !data.settings.subscriptionRefresh.allowCellular
              ? t("Paused until Wi-Fi is available")
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
            ? t("Turn individual updates on or off in the desktop app.")
            : t("Data updates run in the installed app.")}
        </p>
      )}
    </>
  );
}

/** Developer option: where each dataset reads its index from. */
export function DataSourceUrls({
  data,
  update,
}: {
  data: Workspace;
  update: Update;
}) {
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
                  saveSubscription(update, kind, {
                    url,
                    indexHash: undefined,
                    dataUrl: undefined,
                    remoteHash: undefined,
                    year: undefined,
                    lastError: undefined,
                  });
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
