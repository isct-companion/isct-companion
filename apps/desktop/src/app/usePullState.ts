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

function loginStepLabel(step: string) {
  return LOGIN_STEP_LABELS[step] ?? step;
}

export function usePullState() {
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
  useEffect(() => {
    if (activeLms?.state === "blocked")
      setLmsPhase(`Waiting · ${activeLms.blockedBy ?? "school view"}`);
    else if (activeLms?.state === "queued")
      setLmsPhase("Waiting for school refresh");
  }, [activeLms?.id, activeLms?.state, activeLms?.blockedBy]);
  const [lmsPhase, setLmsPhase] = useState("Ready");
  const [kyomuMaintenance, setKyomuMaintenance] = useState<string | null>(null);
  const [loginSteps, setLoginSteps] = useState<
    Partial<Record<"science" | "tokyo-tech", string>>
  >({});
  const pulling = lmsProgress !== null;
  const kyomuPhase =
    activeKyomu?.state === "blocked"
      ? `Waiting · ${activeKyomu.blockedBy ?? "school view"}`
      : activeKyomu?.state === "queued"
        ? "Waiting for school refresh"
        : "Preparing your timetable";
  useEffect(() => {
    const offLogin = desktopBridge?.onSchoolLoginStatus?.((event) => {
      setLoginSteps((current) => ({
        ...current,
        [event.profile]:
          event.state === "connected"
            ? "Signed in"
            : event.state === "progress"
              ? `Signing in · ${loginStepLabel(event.message)}`
              : event.state === "needs-user"
                ? "Waiting for you"
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
  return {
    lmsProgress,
    kyomuProgress,
    setLmsProgress,
    setKyomuProgress,
    lmsPhase: t(lmsPhase),
    setLmsPhase,
    loginSteps,
    pulling,
    schoolStatus:
      kyomuMaintenance && !activeKyomu && !activeLms && !pulling
        ? {
            source: "教務Web",
            phase: `${t("Under maintenance")} · ${kyomuMaintenance}`,
            progress: null,
          }
        : failedPull &&
            !activeKyomu &&
            !activeLms &&
            !pulling &&
            kyomuProgress === null
          ? {
              source:
                failedPull.target.type === "school" &&
                failedPull.target.site === "kyomu"
                  ? "教務Web"
                  : "LMS",
              phase: `${t(failedPull.phase === "partial" ? "Refresh incomplete" : "Refresh failed")}${failedPull.lastError ? ` · ${failedPull.lastError}` : ""}`,
              progress: null,
            }
          : kyomuProgress !== null
            ? {
                source: "教務Web",
                phase: t(kyomuPhase),
                progress: kyomuProgress,
              }
            : {
                source: "LMS",
                phase: t(
                  pulling && lmsPhase === "Ready"
                    ? "Reading enrolled courses"
                    : lmsPhase,
                ),
                progress: lmsProgress,
              },
  };
}
