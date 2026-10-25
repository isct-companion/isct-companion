/** Words for Core's sync snapshot. Core sends i18n keys and facts; this file
 * names them and decides nothing about what is due. */
import { formatJst } from "../lib/civil-date";
import { relativeTimeAgo, t } from "../i18n";
import type {
  RunView,
  SyncFailure,
  SyncHeld,
  SyncLabel,
} from "../platform/sync";

/** One name per `sync_state::Dataset`. */
const DATASET_NAMES: Record<string, string> = {
  "lms.enrolment": "Enrolled courses",
  "lms.deadlines": "Deadlines",
  "lms.course": "Course content",
  "lms.assign": "Assignment status",
  "lms.history": "Past courses",
  "kyomu.page": "教務Web pages",
  "catalog.official": "Course catalog",
  reference: "Reference data",
};

const SOURCE_NAMES: Record<string, string> = {
  lms: "LMS",
  kyomu: "教務Web",
  references: "Reference data",
  catalog: "Course catalog",
};

export const datasetName = (dataset: string) =>
  t(DATASET_NAMES[dataset] ?? dataset);
export const sourceName = (source: string) =>
  t(SOURCE_NAMES[source] ?? source);

/** An identifier-style key (`sync.lms.course`) no locale knows yet. */
const untranslated = (label: SyncLabel, text: string) =>
  text === label.key && /^[\w.-]+$/.test(label.key);

/** A Core label in the UI language; an unknown key falls back to the
 * dataset's own name so a new Core key never shows as an identifier. */
export function labelText(label: SyncLabel, dataset?: string) {
  const text = t(label.key, label.args ?? undefined);
  if (!untranslated(label, text)) return text;
  if (dataset) return datasetName(dataset);
  const named = Object.values(label.args ?? {}).find(
    (value) => typeof value === "string" && value.trim(),
  );
  return typeof named === "string" ? named : text;
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
      return t("Waiting for a connection");
    case "cellular":
      return t("Waiting for Wi-Fi · on cellular only what you open is read");
    case "power-saver":
      return t("Paused while power saver is on");
    case "background":
      return t("Resumes when the app is in front");
    case "backoff":
      return t("Retries at {{time}}", { time: clock(held.until, locale) });
    case "host-paused":
      return t("{{host}} is not answering · retries at {{time}}", {
        host: held.host,
        time: clock(held.until, locale),
      });
    case "sign-in-required":
      return t("Sign-in required");
    case "resetting":
      return t("Resetting…");
    case "prerequisite":
      return t("Waiting for the data it depends on");
  }
}

const FAILURE_KINDS: Record<string, string> = {
  timeout: "timed out",
  offline: "no connection",
  "sign-in": "sign-in required",
  "site-changed": "the school page changed",
};

function failureReason(kind: string) {
  if (FAILURE_KINDS[kind]) return t(FAILURE_KINDS[kind]);
  const status = /^http-(\d{3})$/.exec(kind)?.[1];
  if (status)
    return status.startsWith("5")
      ? t("the school server failed ({{status}})", { status })
      : t("the school server refused ({{status}})", { status });
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
      ? t("{{dataset}}: {{count}} parts failed", {
          dataset: datasetName(dataset),
          count: failure.keysFailed,
        })
      : t("{{dataset}} failed", { dataset: datasetName(dataset) }),
    failureReason(failure.kind),
    failure.retryAt
      ? t("retries at {{time}}", { time: clock(failure.retryAt, locale) })
      : "",
  ]
    .filter(Boolean)
    .join(" · ");
}

/** "3 hours ago", "just now", or "never". */
export function ageText(at: string | null, now: string, locale: string) {
  if (!at) return t("Never updated");
  return relativeTimeAgo(at, now, locale) ?? t("just now");
}
