import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Subscription, Workspace } from "../../core/model";
import { UiButton, UiInput } from "../../components/mantine";
import { Row, Toggle } from "../../components/forms";
import { workspaceStore } from "../../store/workspace";

export const PUBLIC_DATA_INDEX =
  "https://raw.githubusercontent.com/Gliese129/Isct-Data/main/index.json";
const definitions = [
  {
    kind: "calendar" as const,
    title: "Academic calendar",
    detail:
      "Holidays, class cancellations, replacement teaching days and quarter dates.",
    path: "academic-calendar",
  },
  {
    kind: "departments" as const,
    title: "Study guide recommendations",
    detail:
      "Department courses from the official admission guide PDF, selected by admission year.",
    path: "departments",
  },
];
export function ReferenceSubscriptions({
  data,
  update,
  available,
  notify,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
  available: boolean;
  notify: (message: string) => void;
}) {
  const [busy, setBusy] = useState<string>();
  const [urls, setUrls] = useState<Record<string, string>>({});
  const academicYear = Number(data.selectedTerm.slice(0, 4));
  const save = (kind: Subscription["kind"], values: Partial<Subscription>) =>
    update((draft) => {
      let row = draft.subscriptions.find(
        (s) => s.kind === kind && !s.deletedAt,
      );
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
  return (
    <>
      <h2>Subscriptions</h2>
      <p className="muted">
        Public reference data:{" "}
        <a
          href="https://github.com/Gliese129/Isct-Data"
          target="_blank"
          rel="noreferrer"
        >
          Gliese129 / Isct-Data
        </a>
        . JSON files are organized by category and year.
      </p>
      {definitions.map((definition) => {
        const row = data.subscriptions.find(
          (s) => s.kind === definition.kind && !s.deletedAt,
        );
        const url =
          urls[definition.kind] ??
          (row?.url.startsWith("https://") ? row.url : PUBLIC_DATA_INDEX);
        const year =
          definition.kind === "departments"
            ? data.academicProfile.admissionYear
            : academicYear;
        const path = `${definition.path}/${year ?? "{year}"}.json`;
        let resolvedUrl = row?.dataUrl;
        if (!resolvedUrl && year) {
          try { resolvedUrl = new URL(path, url).href; } catch { /* Keep an invalid custom URL editable. */ }
        }
        return (
          <section className="referenceSource" key={definition.kind}>
            <Row title={definition.title} hint={definition.detail}>
              <Toggle
                disabled={!available || !!busy}
                checked={row?.enabled ?? true}
                onChange={(enabled) => save(definition.kind, { enabled })}
              />
            </Row>
            <p className="muted">
              Type: {definition.kind} ·{" "}
              {definition.kind === "departments"
                ? "Admission year"
                : "Academic year"}
              : {year ?? "Choose in General"}. Toggle enables refresh; cached
              data stays available.
            </p>
            <label>
              Source index URL
              <UiInput
                value={url}
                type="url"
                disabled={!available}
                onChange={(event) =>
                  setUrls((prior) => ({
                    ...prior,
                    [definition.kind]: event.target.value,
                  }))
                }
                onBlur={() => {
                  if (urls[definition.kind] !== undefined && url !== row?.url)
                    save(definition.kind, {
                      url,
                      dataUrl: undefined,
                      lastError: undefined,
                    });
                }}
              />
            </label>
            <p className="referenceLocation">
              Repository path:{" "}
              <code>
                {path}
              </code>
            </p>
            {resolvedUrl && (
              <p className="referenceLocation">
                Data file:{" "}
                <a href={resolvedUrl} target="_blank" rel="noreferrer">
                  {resolvedUrl}
                </a>
              </p>
            )}
            <div className="actions">
              <span className="muted" role="status">
                {row?.lastError ??
                  (row?.fetchedAt
                    ? `Updated ${new Date(row.fetchedAt).toLocaleString()}`
                    : "Not refreshed from this source yet")}
              </span>
              <UiButton
                className="button"
                disabled={
                  !available || !year || row?.enabled === false || !!busy
                }
                onClick={async () => {
                  setBusy(definition.kind);
                  try {
                    const result = await invoke<{
                      changed: number;
                      dataUrl: string;
                    }>("refresh_reference_source", {
                      kind: definition.kind,
                      indexUrl: url,
                      year,
                    });
                    await workspaceStore.reload();
                    save(definition.kind, {
                      url,
                      dataUrl: result.dataUrl,
                      fetchedAt: new Date().toISOString(),
                      lastError: undefined,
                    });
                    notify(
                      `${definition.title} updated (${result.changed} changes).`,
                    );
                  } catch (error) {
                    const message =
                      error instanceof Error ? error.message : String(error);
                    save(definition.kind, { url, lastError: message });
                    notify(message);
                  } finally {
                    setBusy(undefined);
                  }
                }}
              >
                {busy === definition.kind ? "Refreshing…" : "Refresh source"}
              </UiButton>
            </div>
          </section>
        );
      })}
      <section className="referenceSource">
        <Row
          title="Important dates"
          hint="Campus festivals, ceremonies and other official dates. An event does not imply classes are cancelled."
          children={null}
        />
        <p className="referenceLocation">
          Type: important-dates · Repository path:{" "}
          <code>important-dates/{academicYear}.json</code>
        </p>
        <p className="muted">
          Available dataset files are listed in the index; app import is pending.
        </p>
        <Row
          title="Examinations"
          hint="Individual courses: course code, exam date, published time or periods, and room."
          children={null}
        />
        <p className="referenceLocation">
          Type: exams · Repository path:{" "}
          <code>
            exam/undergraduate/{academicYear}-{"{quarter}"}.json
            {" or "}exam/graduate/{academicYear}-{"{quarter}"}.json
          </code>
        </p>
        <p className="muted">
          Available dataset files are listed in the index; app import is pending.
        </p>
      </section>
      {!available && (
        <p className="muted">Source refresh is available in the desktop app.</p>
      )}
    </>
  );
}
