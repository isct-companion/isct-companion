import { useEffect, useState } from "react";
import { desktopBridge } from "./native";
import { t } from "../i18n";
import { decodeEntities } from "../lib/text";
import {
  schoolPullForDisplay,
  type PullSnapshot,
} from "../platform/desktop-bridge";

const LOGIN_STEP_LABELS: Record<string, string> = {
  agree: "Accepting portal terms",
  identifier: "Entering account",
  password: "Checking password",
  method: "Selecting sign-in method",
  otp: "Verifying one-time code",
  matrix: "Verifying matrix code",
};

/** Why Core is holding an automatic pull (`pull_coordinator/queue.rs`). */
const BLOCKER_LABELS: Record<string, string> = {
  offline: "Waiting for a connection",
  cellular: "Waiting for Wi-Fi",
  background: "Resumes when the app is in front",
  schoolVisible: "Resumes after you leave the school page",
  suspended: "Paused",
};

function loginStepLabel(step: string) {
  return t(LOGIN_STEP_LABELS[step] ?? step);
}

function waitingLabel(pull: PullSnapshot) {
  return pull.state === "blocked"
    ? t(BLOCKER_LABELS[pull.blockedBy ?? ""] ?? "Waiting")
    : t("Waiting for school refresh");
}

/** One footer line. A pull the user started shows its progress; Core's own
 * automatic pulls stay a quiet note, and an idle footer says how old the data is. */
export type SchoolStatus =
  | { kind: "busy"; source: string; phase: string; progress: number }
  | { kind: "background"; source: string; phase: string }
  | { kind: "problem"; source: string; phase: string; detail?: string }
  | { kind: "idle"; updated: { source: string; at: string | null }[] };

const latest = (values: (string | undefined)[]) =>
  values
    .filter((value): value is string => Boolean(value))
    .sort()
    .at(-1) ?? null;

export function usePullState(lastRuns: Record<string, string> = {}) {
  // View progress also includes the timetable review after the native pull.
  const [lmsViewProgress, setLmsProgress] = useState<number | null>(null);
  const [nativePulls, setNativePulls] = useState<PullSnapshot[]>([]);
  const [nativeLmsProgress, setNativeLmsProgress] = useState(5);
  const activeLms = schoolPullForDisplay(nativePulls, "science-lms");
  const activeKyomu = schoolPullForDisplay(nativePulls, "kyomu");
  // Core publishes history in admission order. Render its terminal outcome;
  // removing active jobs alone would turn a partial failure into “Ready”.
  const latestPulls = new Map<string, PullSnapshot>();
  for (const pull of nativePulls) {
    if (pull.target.type === "school")
      latestPulls.set(`${pull.target.site}:${pull.target.courseId}`, pull);
  }
  const failedPull = [...latestPulls.values()]
    .reverse()
    .find((pull) => pull.state === "failed");
  const lmsProgress = activeLms
    ? Math.max(lmsViewProgress ?? 5, nativeLmsProgress)
    : lmsViewProgress;
  const [kyomuViewProgress, setKyomuProgress] = useState<number | null>(null);
  const [nativeKyomuProgress, setNativeKyomuProgress] = useState(5);
  const kyomuProgress = activeKyomu
    ? Math.max(kyomuViewProgress ?? 5, nativeKyomuProgress)
    : kyomuViewProgress;
  useEffect(() => desktopBridge?.onPullState?.(setNativePulls), []);
  useEffect(() => setNativeLmsProgress(5), [activeLms?.id]);
  useEffect(() => setNativeKyomuProgress(5), [activeKyomu?.id]);
  const [lmsPhase, setLmsPhase] = useState("Ready");
  const [kyomuMaintenance, setKyomuMaintenance] = useState<string | null>(null);
  const [loginSteps, setLoginSteps] = useState<
    Partial<Record<"science" | "tokyo-tech", string>>
  >({});
  const pulling = lmsProgress !== null;
  const kyomuPhase =
    activeKyomu && ["blocked", "queued"].includes(activeKyomu.state)
      ? waitingLabel(activeKyomu)
      : t("Reading 教務Web pages");
  useEffect(() => {
    const offLogin = desktopBridge?.onSchoolLoginStatus?.((event) => {
      setLoginSteps((current) => ({
        ...current,
        [event.profile]:
          event.state === "connected"
            ? t("Signed in")
            : event.state === "progress"
              ? `${t("Signing in")} · ${loginStepLabel(event.message)}`
              : event.state === "needs-user"
                ? t("Waiting for you")
                : event.message,
      }));
    });
    let disposed = false;
    let observedKyomu = false;
    const offIngest = desktopBridge?.onSchoolIngest?.((outcome) => {
      if (outcome.site === "kyomu") {
        observedKyomu = true;
        if (outcome.state === "maintenance") {
          setKyomuMaintenance(outcome.message);
          setKyomuProgress(null);
        } else if (outcome.state === "ok" || outcome.state === "unavailable") {
          setKyomuMaintenance(null);
        }
      }
      if (
        outcome.site === "kyomu" &&
        outcome.kind !== "service" &&
        (outcome.state === "ok" || outcome.state === "unavailable")
      )
        setNativeKyomuProgress((value) =>
          Math.min(95, Math.max(value, 15) + 20),
        );
    });
    void desktopBridge
      ?.schoolSessionSnapshot?.()
      .then((snapshot) => {
        if (!disposed && !observedKyomu)
          setKyomuMaintenance(snapshot.kyomuMaintenance ?? null);
      })
      .catch(() => undefined);
    const offProgress = desktopBridge?.onSchoolPullProgress?.((progress) => {
      const ratio = progress.total
        ? Math.min(1, progress.completed / progress.total)
        : 1;
      const ranges = {
        catalog: [5, 8],
        course: [8, 25],
        material: [25, 38],
        assignment: [38, 44],
        timetable: [44, 95],
      } as const;
      const [start, end] = ranges[progress.phase];
      setNativeLmsProgress((current) =>
        Math.max(current, start + (end - start) * ratio),
      );
      const course = decodeEntities(progress.courseTitle).trim();
      const item = decodeEntities(progress.itemTitle).trim();
      const context = [progress.term, course, item].filter(Boolean).join(" · ");
      const count = progress.total
        ? `${progress.completed}/${progress.total}`
        : "0/0";
      const label =
        progress.phase === "timetable"
          ? t("Preparing your timetable")
          : progress.phase === "catalog"
            ? t("Reading enrolled courses")
            : progress.phase === "course"
              ? t("Loading course")
              : progress.phase === "material"
                ? t("Reading material")
                : t("Checking assignment");
      setLmsPhase(`${label} · ${context || "LMS"} · ${count}`);
    });
    return () => {
      disposed = true;
      offLogin?.();
      offIngest?.();
      offProgress?.();
    };
  }, []);
  const lmsWaiting =
    activeLms && ["blocked", "queued"].includes(activeLms.state)
      ? waitingLabel(activeLms)
      : null;
  const lmsLine = t(
    lmsWaiting ??
      (lmsPhase === "Ready" ? "Reading enrolled courses" : lmsPhase),
  );
  // The view progress is only set by actions the user started.
  const kyomuManual =
    kyomuViewProgress !== null || activeKyomu?.priority === "manual";
  const lmsManual =
    lmsViewProgress !== null || activeLms?.priority === "manual";
  const idle = !activeKyomu && !activeLms && !pulling && kyomuProgress === null;
  const schoolStatus: SchoolStatus =
    kyomuMaintenance && idle
      ? {
          kind: "problem",
          source: "教務Web",
          phase: t("Under maintenance"),
          detail: kyomuMaintenance,
        }
      : failedPull && idle
        ? {
            kind: "problem",
            source:
              failedPull.target.type === "school" &&
              failedPull.target.site === "kyomu"
                ? "教務Web"
                : "LMS",
            phase: t(
              failedPull.phase === "partial"
                ? "Refresh incomplete"
                : "Refresh failed",
            ),
            detail: failedPull.lastError ?? undefined,
          }
        : kyomuProgress !== null && kyomuManual
          ? {
              kind: "busy",
              source: "教務Web",
              phase: kyomuPhase,
              progress: kyomuProgress,
            }
          : lmsProgress !== null && lmsManual
            ? {
                kind: "busy",
                source: "LMS",
                phase: lmsLine,
                progress: lmsProgress,
              }
            : activeKyomu || activeLms
              ? {
                  kind: "background",
                  source: activeKyomu ? "教務Web" : "LMS",
                  phase:
                    (activeKyomu
                      ? ["blocked", "queued"].includes(activeKyomu.state) &&
                        waitingLabel(activeKyomu)
                      : lmsWaiting) || t("Updating in the background"),
                }
              : {
                  kind: "idle",
                  updated: [
                    {
                      source: "LMS",
                      at: latest([
                        lastRuns["lms:all"],
                        lastRuns["lms:summary"],
                      ]),
                    },
                    {
                      source: "教務Web",
                      at: latest(
                        Object.entries(lastRuns)
                          .filter(([key]) => key.startsWith("kyomu:"))
                          .map(([, at]) => at),
                      ),
                    },
                  ],
                };
  return {
    lmsProgress,
    kyomuProgress,
    setLmsProgress,
    setKyomuProgress,
    lmsPhase: lmsLine,
    setLmsPhase,
    loginSteps,
    pulling,
    schoolStatus,
  };
}
