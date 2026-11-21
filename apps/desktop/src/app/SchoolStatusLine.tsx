import type { SyncSnapshot } from "../generated/persistence";
import { UiButton } from "../components/ui";
import { t } from "../i18n";
import { failedRuns, progressLine } from "../platform/sync";
import { retry } from "../platform/sync-intents";
import {
  sourceFailureText,
  failureText,
  heldText,
  runText,
  sourceName,
  signInText,
  signInReason,
} from "../lib/sync-text";

/** One line of what Core's planner is doing now. A bar appears only when
 * Core knows the run's size; background-only work is a small spinner. */
function SyncProgressLine({
  snapshot,
  locale,
}: {
  snapshot: SyncSnapshot;
  locale: string;
}) {
  const line = progressLine(snapshot);
  switch (line.kind) {
    case "idle":
      return null;
    case "resetting":
      return (
        <span className="syncProgress" data-kind="busy">
          <i className="schoolStatusSpinner" aria-hidden="true" />
          <span>{t("app.resetting")}</span>
        </span>
      );
    case "problem": {
      const problem = snapshot.attention.problem;
      if (problem?.kind !== "account") return null;
      const presentation = problem.presentation;
      const text = presentation
        ? [
            signInText(presentation, problem.retryAt, locale),
            signInReason(presentation),
          ]
            .filter(Boolean)
            .join(" · ")
        : t("syncStatus.held.person");
      return (
        <span className="syncProgress" data-kind="held">
          {text}
        </span>
      );
    }
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
          <span>{t("app.updatingInTheBackground")}</span>
        </span>
      );
    case "held":
      return (
        <span className="syncProgress" data-kind="held">
          <span title={runText(line.run)}>{heldText(line.held, locale)}</span>
        </span>
      );
  }
}

/** Each failed dataset by name, when Core retries, and a Retry that asks
 * for that scope now. */
function SyncFailures({
  snapshot,
  locale,
}: {
  snapshot: SyncSnapshot;
  locale: string;
}) {
  const failures = [
    ...failedRuns(snapshot).flatMap((run) =>
      run.failure
        ? [
            {
              id: run.run,
              failure: run.failure,
              retry: run.retry,
              text: failureText(run.dataset, run.failure, locale),
            },
          ]
        : [],
    ),
    ...snapshot.sources.flatMap((source) =>
      source.failure
        ? [
            {
              id: source.source,
              failure: source.failure,
              retry: source.retry,
              text: sourceFailureText(
                sourceName(source.source),
                source.failure,
                locale,
              ),
            },
          ]
        : [],
    ),
  ];
  if (!failures.length) return null;
  return (
    <ul className="syncFailures" role="status">
      {failures.map(({ id, failure, retry: plan, text }) => (
        <li key={id}>
          <span title={failure.detail ?? undefined}>{text}</span>
          {plan && (
            <UiButton
              type="button"
              className="textButton"
              disabled={snapshot.resetting}
              onClick={() => retry(plan)}
            >
              {t("common.retry")}
            </UiButton>
          )}
        </li>
      ))}
    </ul>
  );
}

/** The status panel's sync half: what is running and what failed. A quiet,
 * healthy sync shows nothing; the accounts above already say "connected". */
export function SchoolStatusLine({
  snapshot,
  locale,
}: {
  snapshot: SyncSnapshot;
  locale: string;
}) {
  const line = progressLine(snapshot);
  const failed = snapshot.attention.problem !== null;
  const anyFailure =
    failedRuns(snapshot).some((run) => run.failure) ||
    snapshot.sources.some((source) => source.failure);
  if (line.kind === "idle" && !failed && !anyFailure) return null;
  return (
    <div
      className="lmsStatus"
      data-kind={failed ? "problem" : line.kind}
      aria-live="polite"
    >
      {line.kind !== "idle" && (
        <div className="lmsStatusLine">
          <SyncProgressLine snapshot={snapshot} locale={locale} />
        </div>
      )}
      <SyncFailures snapshot={snapshot} locale={locale} />
    </div>
  );
}
