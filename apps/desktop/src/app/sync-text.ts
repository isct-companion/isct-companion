/** Words for Core's sync snapshot. Core sends i18n keys and facts; this file
 * names them and decides nothing about what is due. */
import { formatJst } from "../lib/civil-date";
import { lookup, relativeTimeAgo, t, type MessageKey } from "../i18n";
import type {
  RunView,
  SyncFailure,
  SyncHeld,
  SyncLabel,
} from "../platform/sync";

/** One name per `sync_state::Dataset`. */
const DATASET_NAMES: Record<string, MessageKey> = {
  "lms.enrolment": "syncStatus.dataset.enrolledCourses",
  "lms.deadlines": "syncStatus.dataset.deadlines",
  "lms.course": "syncStatus.dataset.courseContent",
  "lms.assign": "syncStatus.dataset.assignmentStatus",
  "lms.history": "syncStatus.dataset.pastCourses",
  "kyomu.page": "syncStatus.dataset.kyomuPages",
  "catalog.official": "syncStatus.dataset.courseCatalog",
  reference: "syncStatus.dataset.referenceData",
};

const SOURCE_NAMES: Record<string, MessageKey> = {
  lms: "syncStatus.source.lms",
  kyomu: "syncStatus.source.kyomu",
  references: "syncStatus.dataset.referenceData",
  catalog: "syncStatus.dataset.courseCatalog",
};

export const datasetName = (dataset: string) =>
  DATASET_NAMES[dataset] ? t(DATASET_NAMES[dataset]) : dataset;
export const sourceName = (source: string) =>
  SOURCE_NAMES[source] ? t(SOURCE_NAMES[source]) : source;

/** A Core label in the UI language. Core owns these identifier-style keys
 * (`sync.run.reference`), so a key no catalog knows yet falls back to the
 * dataset's own name and a new Core key never shows as an identifier. */
export function labelText(label: SyncLabel, dataset?: string) {
  const text = lookup(label.key, label.args ?? undefined);
  if (text !== undefined) return text;
  if (dataset) return datasetName(dataset);
  const named = Object.values(label.args ?? {}).find(
    (value) => typeof value === "string" && value.trim(),
  );
  return typeof named === "string" ? named : label.key;
}

export const runText = (run: RunView) => {
  const label = labelText(run.label, run.dataset);
  const current = run.current ? labelText(run.current) : "";
  return current ? `${label} · ${current}` : label;
};

const clock = (at: string, locale: string) =>
  formatJst(at, locale, {
    ...(Math.abs(Date.parse(at) - Date.now()) > 12 * 3_600_000
      ? { month: "numeric", day: "numeric" }
      : {}),
    hour: "2-digit",
    minute: "2-digit",
  });

export function heldText(held: SyncHeld, locale: string) {
  switch (held.kind) {
    case "offline":
      return t("syncStatus.waitingForAConnection");
    case "cellular":
      return t("app.waitingForWiFiOn");
    case "power-saver":
      return t("app.pausedWhilePowerSaverIs");
    case "background":
      return t("syncStatus.resumesWhenTheAppIs");
    case "backoff":
      return t("syncStatus.held.retriesAt", {
        time: clock(held.until, locale),
      });
    case "host-paused":
      return t("app.hostIsNotAnsweringRetries", {
        host: held.host,
        time: clock(held.until, locale),
      });
    case "sign-in-required":
      return t("app.signInRequired");
    case "resetting":
      return t("app.resetting");
    case "prerequisite":
      return t("syncStatus.waitingForTheDataIt");
  }
}

const FAILURE_KINDS: Record<string, MessageKey> = {
  timeout: "syncStatus.failure.timedOut",
  offline: "syncStatus.failure.noConnection",
  "sign-in": "syncStatus.failure.signInRequired",
  "site-changed": "syncStatus.failure.siteChanged",
};

function failureReason(kind: string) {
  if (FAILURE_KINDS[kind]) return t(FAILURE_KINDS[kind]);
  const status = /^http-(\d{3})$/.exec(kind)?.[1];
  if (status)
    return status.startsWith("5")
      ? t("syncStatus.failure.serverFailed", { status })
      : t("syncStatus.failure.serverRefused", { status });
  return kind;
}

/** "Grades page failed · timed out · retries at 14:05". */
export function failureText(
  dataset: string,
  failure: SyncFailure,
  locale: string,
) {
  return [
    failure.keysFailed > 1
      ? t("app.datasetCountPartsFailed", {
          dataset: datasetName(dataset),
          count: failure.keysFailed,
        })
      : t("app.datasetFailed", { dataset: datasetName(dataset) }),
    failureReason(failure.kind),
    failure.retryAt
      ? t("syncStatus.failure.retriesAt", {
          time: clock(failure.retryAt, locale),
        })
      : "",
  ]
    .filter(Boolean)
    .join(" · ");
}

/** "3 hours ago", "just now", or "never". */
export function ageText(at: string | null, now: string, locale: string) {
  if (!at) return t("common.neverUpdated");
  return relativeTimeAgo(at, now, locale) ?? t("syncStatus.justNow");
}
