import { useEffect, useState } from "react";
import { desktopBridge } from "./native";
import { t } from "../i18n";
import { decodeEntities } from "../lib/text";
import type {
  PullStep,
  SchoolSource,
  SchoolSourceStatus,
  SchoolStatus as CoreSchoolStatus,
  SignInState,
  SignInStep,
  SourceActivity,
  WaitReason,
} from "../generated/persistence";

const SIGN_IN_STEP_LABELS: Partial<Record<SignInStep, string>> = {
  agree: "Accepting portal terms",
  identifier: "Entering account",
  password: "Checking password",
  method: "Selecting sign-in method",
  otp: "Verifying one-time code",
  matrix: "Verifying matrix code",
};

/** Why Core is holding a pull (`pull_coordinator/queue.rs`). */
const WAIT_LABELS: Record<WaitReason, string> = {
  queued: "Waiting for school refresh",
  offline: "Waiting for a connection",
  cellular: "Waiting for Wi-Fi",
  background: "Resumes when the app is in front",
  schoolVisible: "Resumes after you leave the school page",
  suspended: "Paused",
  other: "Waiting",
};

const STEP_LABELS: Record<PullStep["kind"], string> = {
  catalog: "Reading enrolled courses",
  course: "Loading course",
  material: "Reading material",
  assignment: "Checking assignment",
  timetable: "Preparing your timetable",
  saving: "Saving LMS changes",
  kyomu: "Reading 教務Web pages",
};

const sourceName = (site: SchoolSource) =>
  site === "kyomu" ? "教務Web" : "LMS";

/** A step with nothing reported yet reads as its label alone. */
function stepLabel(step: PullStep) {
  const label = t(STEP_LABELS[step.kind]);
  const course = decodeEntities(step.courseTitle).trim();
  const item = decodeEntities(step.itemTitle).trim();
  if (!step.total && !step.term && !course && !item) return label;
  const context = [step.term, course, item].filter(Boolean).join(" · ");
  return `${label} · ${context || "LMS"} · ${step.completed}/${step.total}`;
}

function signInLabel(state: SignInState) {
  switch (state.kind) {
    case "connected":
      return t("Signed in");
    case "step": {
      const step = SIGN_IN_STEP_LABELS[state.step];
      return step ? `${t("Signing in")} · ${t(step)}` : t("Signing in");
    }
    case "needsUser":
      return t("Waiting for you");
    case "failed":
      return state.message;
  }
}

function activityProgress(activity: SourceActivity | undefined) {
  if (activity?.kind === "running") return activity.progress;
  return activity?.kind === "waiting" ? 5 : null;
}

/** One footer line. A pull the user started shows its progress; Core's own
 * automatic pulls stay a quiet note, and an idle footer says how old the data is. */
export type SchoolStatus =
  | { kind: "busy"; source: string; phase: string; progress: number }
  | { kind: "background"; source: string; phase: string }
  | { kind: "problem"; source: string; phase: string; detail?: string }
  | { kind: "idle"; updated: { source: string; at: string | null }[] };

/** Words for Core's school status (`school_status.rs`), which decides what
 * is shown; this hook only names it. */
export function schoolStatusLine(status: CoreSchoolStatus): SchoolStatus {
  const { headline } = status;
  switch (headline.kind) {
    case "idle":
      return {
        kind: "idle",
        updated: status.sources.map((source) => ({
          source: sourceName(source.site),
          at: source.lastUpdated,
        })),
      };
    case "busy": {
      const phase = headline.waiting
        ? t(WAIT_LABELS[headline.waiting])
        : stepLabel(headline.step);
      return {
        kind: "busy",
        source: sourceName(headline.site),
        phase: headline.queued
          ? `${phase} · ${t("{{source}} refresh queued", {
              source: sourceName(headline.queued),
            })}`
          : phase,
        progress: headline.progress,
      };
    }
    case "background":
      return {
        kind: "background",
        source: sourceName(headline.site),
        phase: t(
          headline.waiting
            ? WAIT_LABELS[headline.waiting]
            : "Updating in the background",
        ),
      };
    case "problem": {
      const { problem } = headline;
      return {
        kind: "problem",
        source: sourceName(headline.site),
        phase: t(
          problem.kind === "maintenance"
            ? "Under maintenance"
            : problem.kind === "partial"
              ? "Refresh incomplete"
              : "Refresh failed",
        ),
        detail: problem.detail ?? undefined,
      };
    }
  }
}

const IDLE: CoreSchoolStatus = {
  headline: { kind: "idle" },
  sources: [],
};

export function usePullState() {
  const [status, setStatus] = useState<CoreSchoolStatus>(IDLE);
  useEffect(() => desktopBridge?.onSchoolStatus?.(setStatus), []);
  const source = (site: SchoolSource): SchoolSourceStatus | undefined =>
    status.sources.find((row) => row.site === site);
  const lms = source("science-lms");
  const kyomu = source("kyomu");
  const lmsProgress = activityProgress(lms?.activity);
  const signIn = (row: SchoolSourceStatus | undefined) =>
    row?.signIn ? signInLabel(row.signIn) : undefined;
  return {
    lmsProgress,
    kyomuProgress: activityProgress(kyomu?.activity),
    lmsPhase:
      lms?.activity.kind === "waiting"
        ? t(WAIT_LABELS[lms.activity.reason])
        : lms?.activity.kind === "running"
          ? stepLabel(lms.activity.step)
          : t(STEP_LABELS.catalog),
    loginSteps: {
      science: signIn(lms),
      "tokyo-tech": signIn(kyomu),
    },
    pulling: lmsProgress !== null,
    schoolStatus: schoolStatusLine(status),
  };
}
