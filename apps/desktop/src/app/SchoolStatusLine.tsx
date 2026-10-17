import { UiButton } from "../components/mantine";
import { formatJst } from "../lib/civil-date";
import { relativeTimeAgo, t } from "../i18n";
import { useClock } from "../platform/use-clock";
import type { SchoolStatus } from "./usePullState";

/** The right half of the workstation footer: what the school data is doing. */
export function SchoolStatusLine({
  status,
  locale,
  refresh,
}: {
  status: SchoolStatus;
  locale: string;
  refresh?: { busy: boolean; run: () => void };
}) {
  const now = useClock(status.kind === "idle");
  const line =
    status.kind === "idle" ? (
      <span className="schoolUpdated">
        {status.updated.map(({ source, at }) => (
          <span
            key={source}
            title={
              at
                ? formatJst(at, locale, {
                    month: "short",
                    day: "numeric",
                    hour: "2-digit",
                    minute: "2-digit",
                  })
                : undefined
            }
          >
            <b>{source}</b>
            {at
              ? (relativeTimeAgo(at, now, locale) ?? t("just now"))
              : t("Never updated")}
          </span>
        ))}
      </span>
    ) : (
      <>
        {status.kind === "background" ? (
          <i className="schoolStatusSpinner" aria-hidden="true" />
        ) : null}
        <b>{status.source}</b>
        <span
          title={
            status.kind === "problem" && status.detail
              ? status.detail
              : status.phase
          }
        >
          {status.phase}
          {status.kind === "problem" && status.detail
            ? ` · ${status.detail}`
            : ""}
        </span>
      </>
    );
  return (
    <div className="lmsStatus" data-kind={status.kind} aria-live="polite">
      <div className="lmsStatusLine">
        {line}
        {refresh && (
          <UiButton
            type="button"
            className="textButton"
            disabled={refresh.busy}
            onClick={refresh.run}
          >
            {t(refresh.busy ? "Refreshing…" : "Refresh now")}
          </UiButton>
        )}
      </div>
      {status.kind === "busy" && (
        <progress aria-label={status.phase} max={100} value={status.progress} />
      )}
    </div>
  );
}
