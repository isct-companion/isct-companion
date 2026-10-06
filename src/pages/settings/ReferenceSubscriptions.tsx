import { Fragment, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t } from "../../i18n";
import type { Subscription, Workspace } from "../../core/model";
import { UiButton, UiInput } from "../../components/mantine";
import { Row, Toggle } from "../../components/forms";
import { workspaceStore } from "../../store/workspace";
import {
  PUBLIC_DATA_INDEX,
  subscriptionYear,
  type ReferenceProbeResult,
  type ReferenceRefreshResult,
} from "../../platform/subscription-polling";

const definitions = [
  { kind: "calendar" as const, title: "Academic calendar" },
  { kind: "departments" as const, title: "Department courses" },
  { kind: "important-dates" as const, title: "Important dates" },
  { kind: "exams" as const, title: "Exams" },
];
type Update = (f: (d: Workspace) => Workspace) => void;

const activeRow = (data: Workspace, kind: Subscription["kind"]) =>
  data.subscriptions.find((s) => s.kind === kind && !s.deletedAt);

const indexUrl = (row: Subscription | undefined) =>
  row?.url.startsWith("https://") ? row.url : PUBLIC_DATA_INDEX;

const yearFor = (data: Workspace, kind: Subscription["kind"]) =>
  subscriptionYear(data, kind);

function saveSubscription(
  update: Update,
  kind: Subscription["kind"],
  values: Partial<Subscription>,
) {
  update((draft) => {
    let row = activeRow(draft, kind);
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

export function ReferenceSubscriptions({
  data,
  update,
  available,
  notify,
}: {
  data: Workspace;
  update: Update;
  available: boolean;
  notify: (message: string) => void;
}) {
  const [busy, setBusy] = useState<string>();
  return (
    <>
      <h2>{t("Subscriptions")}</h2>
      <p className="settingsIntro">
        {t("Source")}:{" "}
        <a
          href="https://github.com/isct-companion/Isct-Data"
          target="_blank"
          rel="noreferrer"
        >
          isct-companion/Isct-Data
        </a>
      </p>
      {definitions.map((definition) => {
        const row = activeRow(data, definition.kind);
        const year = yearFor(data, definition.kind);
        const url = indexUrl(row);
        const status = !year
          ? t("Set admission year in General")
          : (row?.lastError ??
            (row?.fetchedAt
              ? `${year} · ${t("Updated")} ${new Date(row.fetchedAt).toLocaleDateString()}`
              : `${year} · ${t("Not refreshed yet")}`));
        return (
          <Row key={definition.kind} title={definition.title} hint={status}>
            {/* Toggle first: the row label activates its first control. */}
            <span className="actions reverse">
              <Toggle
                disabled={!available || !!busy}
                checked={row?.enabled ?? true}
                onChange={(enabled) =>
                  saveSubscription(update, definition.kind, { enabled })
                }
              />
              <UiButton
                className="button small"
                disabled={
                  !available || !year || row?.enabled === false || !!busy
                }
                onClick={async () => {
                  setBusy(definition.kind);
                  try {
                    const probe = await invoke<ReferenceProbeResult>(
                      "probe_reference_source",
                      { indexUrl: url },
                    );
                    if (row?.indexHash === probe.indexHash) {
                      saveSubscription(update, definition.kind, {
                        url,
                        lastError: undefined,
                      });
                      notify(`${t(definition.title)}: 0 ${t("changes")}`);
                      return;
                    }
                    const result = await invoke<ReferenceRefreshResult>("refresh_reference_source", {
                      kind: definition.kind,
                      indexUrl: url,
                      year,
                      expectedIndexHash: probe.indexHash,
                      knownHash: row?.remoteHash,
                    });
                    await workspaceStore.reload();
                    saveSubscription(update, definition.kind, {
                      url,
                      indexHash: probe.indexHash,
                      dataUrl: result.dataUrl,
                      remoteHash: result.remoteHash,
                      sourceTs: result.sourceTs,
                      fetchedAt: new Date().toISOString(),
                      lastError: undefined,
                    });
                    notify(
                      `${t(definition.title)}: ${result.changed} ${t("changes")}`,
                    );
                  } catch (error) {
                    const message =
                      error instanceof Error ? error.message : String(error);
                    saveSubscription(update, definition.kind, {
                      url,
                      lastError: message,
                    });
                    notify(message);
                  } finally {
                    setBusy(undefined);
                  }
                }}
              >
                {busy === definition.kind ? t("Refreshing…") : t("Refresh")}
              </UiButton>
            </span>
          </Row>
        );
      })}
      {!available && (
        <p className="settingsIntro">{t("Refresh in the desktop app.")}</p>
      )}
    </>
  );
}

/** Developer config: where each subscription reads its index from. */
export function SubscriptionSources({
  data,
  update,
}: {
  data: Workspace;
  update: Update;
}) {
  const [urls, setUrls] = useState<Record<string, string>>({});
  return (
    <>
      {definitions.map((definition) => {
        const row = activeRow(data, definition.kind);
        const url = urls[definition.kind] ?? indexUrl(row);
        return (
          <Fragment key={definition.kind}>
            <label className="configField">
              <span>{t(definition.title)}</span>
              <UiInput
                value={url}
                type="url"
                onChange={(event) =>
                  setUrls((prior) => ({
                    ...prior,
                    [definition.kind]: event.target.value,
                  }))
                }
                onBlur={() => {
                  if (urls[definition.kind] !== undefined && url !== row?.url)
                    saveSubscription(update, definition.kind, {
                      url,
                      indexHash: undefined,
                      dataUrl: undefined,
                      remoteHash: undefined,
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
