import { UiButton } from "../components/mantine";
import { formatJst } from "../lib/civil-date";
import { t } from "../i18n";
import { useClock } from "../platform/use-clock";
import {
  failedRuns,
  progressLine,
  retryScope,
  type SyncSnapshot,
} from "../platform/sync";
import { refresh as sendRefresh } from "./useSync";
import {
  ageText,
  failureText,
  heldText,
  runText,
  sourceName,
} from "./sync-text";

/** One line of what Core's planner is doing now. A bar appears only when
 * Core knows the run's size; background-only work is a small spinner. Both
 * the workstation footer and the compact top pill render this. */
export function SyncProgressLine({
  snapshot,
  locale,
}: {
  snapshot: SyncSnapshot;
  locale: string;
}) {
  const line = progressLine(snapshot);
  if (snapshot.resetting)
    return (
      <span className="syncProgress" data-kind="busy">
        <i className="schoolStatusSpinner" aria-hidden="true" />
        <span>{t("Resetting…")}</span>
      </span>
    );
  switch (line.kind) {
    case "idle":
      return null;
    case "bar":
      return (
        <span className="syncProgress" data-kind="bar">
          <span title={runText(line.run)}>
            {runText(line.run)} · {line.done} / {line.total}
          </span>
          <progress
            aria-label={runText(line.run)}
            max={Math.max(line.total, 1)}
            value={Math.min(line.done, line.total)}
          />
        </span>
      );
    case "busy":
      return (
        <span className="syncProgress" data-kind="busy">
          <i className="schoolStatusSpinner" aria-hidden="true" />
          <span title={runText(line.run)}>{runText(line.run)}</span>
        </span>
      );
    case "background":
      return (
        <span
          className="syncProgress"
          data-kind="background"
          title={runText(line.run)}
        >
          <i className="schoolStatusSpinner" aria-hidden="true" />
          <span>{t("Updating in the background")}</span>
        </span>
      );
    case "held":
      return (
        <span className="syncProgress" data-kind="held">
          <span title={runText(line.run)}>
            {heldText(line.held, locale)}
          </span>
        </span>
      );
  }
}

/** Each failed dataset by name, when Core retries, and a Retry that asks
 * for that scope now. */
export function SyncFailures({
  snapshot,
  locale,
}: {
  snapshot: SyncSnapshot;
  locale: string;
}) {
  const failures = failedRuns(snapshot);
  if (!failures.length) return null;
  return (
    <ul className="syncFailures" role="status">
      {failures.map((run) => (
        <li key={run.run}>
          <span>{failureText(run.dataset, run.failure!, locale)}</span>
          <UiButton
            type="button"
            className="textButton"
            disabled={snapshot.resetting}
            onClick={() => sendRefresh(retryScope(run))}
          >
            {t("Retry")}
          </UiButton>
        </li>
      ))}
    </ul>
  );
}

/** The right half of the workstation footer: how old each source is, what
 * is running, and what failed. */
export function SchoolStatusLine({
  snapshot,
  locale,
  refresh,
}: {
  snapshot: SyncSnapshot;
  locale: string;
  /** Sends `Refresh { everything }`; absent outside the desktop app. */
  refresh?: () => void;
}) {
  const line = progressLine(snapshot);
  const now = useClock(line.kind === "idle");
  const failed = failedRuns(snapshot).length > 0;
  return (
    <div
      className="lmsStatus"
      data-kind={failed ? "problem" : line.kind}
      aria-live="polite"
    >
      <div className="lmsStatusLine">
        {line.kind === "idle" && !snapshot.resetting ? (
          <span className="schoolUpdated">
            {snapshot.sources.map((source) => (
              <span
                key={source.source}
                title={
                  source.lastUpdated
                    ? formatJst(source.lastUpdated, locale, {
                        month: "short",
                        day: "numeric",
                        hour: "2-digit",
                        minute: "2-digit",
                      })
                    : undefined
                }
              >
                <b>{sourceName(source.source)}</b>
                {source.held
                  ? heldText(source.held, locale)
                  : ageText(source.lastUpdated, now, locale)}
              </span>
            ))}
          </span>
        ) : (
          <SyncProgressLine snapshot={snapshot} locale={locale} />
        )}
        {refresh && (
          <UiButton
            type="button"
            className="textButton"
            disabled={snapshot.resetting}
            onClick={refresh}
          >
            {t(snapshot.resetting ? "Resetting…" : "Refresh now")}
          </UiButton>
        )}
      </div>
      <SyncFailures snapshot={snapshot} locale={locale} />
    </div>
  );
}
