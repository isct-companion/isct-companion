/** Words for Core's sync snapshot and school sessions. Core sends i18n keys,
 * states and facts; this file names them and decides nothing about what is
 * due or when to sign in. */
import type {
  AccountWait,
  RunView,
  SchoolAuthProfile,
  SchoolProfileSession,
  SignInPresentation,
  SyncFailure,
  SyncHeld,
  SyncLabel,
  SyncSnapshot,
} from "../generated/persistence";
import type { ProgressLine } from "../platform/sync";
import { formatJst } from "./civil-date";
import { lookup, relativeTimeAgo, t, type MessageKey } from "../i18n";

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

/** What an account's waiting reads wait for. */
function accountWaitText(wait: AccountWait, locale: string) {
  switch (wait.kind) {
    case "person":
      return t("syncStatus.held.person");
    case "paused":
      return t("schoolSession.state.unavailableRetry", {
        time: clock(wait.until, locale),
      });
  }
}

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
        time: clock(held.until, locale),
      });
    case "account":
      return accountWaitText(held.wait, locale);
    case "resetting":
      return t("app.resetting");
    case "prerequisite":
      return t("syncStatus.waitingForTheDataIt");
  }
}

/** "Grades page failed · timed out · retries at 14:05". */
export function failureText(
  dataset: string,
  failure: SyncFailure,
  locale: string,
) {
  return sourceFailureText(datasetName(dataset), failure, locale);
}

export function sourceFailureText(
  name: string,
  failure: SyncFailure,
  locale: string,
) {
  return [
    failure.keysFailed > 1
      ? t("app.datasetCountPartsFailed", {
          dataset: name,
          count: failure.keysFailed,
        })
      : t("app.datasetFailed", { dataset: name }),
    labelText(failure.reason),
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

/** The floating status's one line, kept to what a glance needs: whether the
 * school sites are connected, updating, paused or need the person. `problem`
 * counts failed runs and sources and accounts that wait on the person (or
 * Core's account blocker); ages and run names live in the panel. */
export function statusOverview(
  snapshot: SyncSnapshot,
  line: ProgressLine,
  sessions: (SchoolProfileSession | undefined)[],
  locale: string,
): { kind: "problem" | "held" | "busy" | "idle"; text: string } {
  if (snapshot.resetting) return { kind: "busy", text: t("app.resetting") };
  const flagged =
    snapshot.runs.filter((run) => run.failure).length +
      snapshot.sources.filter((source) => source.failure).length +
      sessions.filter(
        (session) =>
          session?.state === "refused" || session?.state === "needs-web",
      ).length || (snapshot.attention.problem ? 1 : 0);
  if (flagged)
    return {
      kind: "problem",
      text: t("syncStatus.summary.problems", { count: flagged }),
    };
  if (line.kind === "held")
    return { kind: "held", text: heldText(line.held, locale) };
  if (line.kind !== "idle")
    return { kind: "busy", text: t("syncStatus.summary.updating") };
  const held = snapshot.sources.find((source) => source.held)?.held;
  if (held) return { kind: "held", text: heldText(held, locale) };
  if (sessions.some((session) => !session || session.state === "signing-in"))
    return { kind: "busy", text: t("app.checkingSession") };
  if (sessions.some((session) => session?.state === "unavailable"))
    return { kind: "held", text: t("schoolSession.state.unavailable") };
  return sessions.some((session) => session?.state === "normal")
    ? { kind: "idle", text: t("schoolSession.state.normal") }
    : { kind: "held", text: t("common.notConnected") };
}

/** The site each account's maintenance pauses. */
const MAINTENANCE_SOURCES: Record<SchoolAuthProfile, MessageKey> = {
  science: "syncStatus.source.lms",
  "tokyo-tech": "syncStatus.source.kyomu",
};

export function signInText(
  presentation: SignInPresentation,
  retryAt: string | null,
  locale: string,
) {
  return lookup(
    presentation.stateKey,
    retryAt ? { time: clock(retryAt, locale) } : undefined,
  );
}

export function signInReason(presentation: SignInPresentation) {
  return presentation.reasonKey ? lookup(presentation.reasonKey) : undefined;
}

export function sessionText(session: SchoolProfileSession, locale: string) {
  return signInText(session.presentation, session.retryAt, locale);
}

/** The sentences under the label: the reason by kind, what the person can
 * do about it, and a maintenance pause with its end. */
export function sessionDetails(
  profile: SchoolAuthProfile,
  session: SchoolProfileSession,
  locale: string,
): string[] {
  const lines: string[] = [];
  // A kind this build does not know yet shows Core's own diagnostic.
  const reason = signInReason(session.presentation);
  if (reason) lines.push(reason);
  if (session.refusalUnsaved) lines.push(t("schoolSession.refusalUnsaved"));
  if (session.presentation.recovery.includes("open-school-page"))
    lines.push(t("schoolSession.needsWeb.signInYourself"));
  if (session.maintenance)
    lines.push(
      t("schoolSession.maintenance.until", {
        site: t(MAINTENANCE_SOURCES[profile]),
        time: clock(session.maintenance.until, locale),
      }),
      ...(session.maintenance.notice ? [session.maintenance.notice] : []),
    );
  return lines;
}
