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
  const lmsProgress = activeLms
    ? Math.max(lmsViewProgress ?? 5, nativeLmsProgress)
    : lmsViewProgress;
  const [kyomuViewProgress, setKyomuProgress] = useState<number | null>(null);
  const kyomuProgress = kyomuViewProgress ?? (activeKyomu ? 5 : null);
  useEffect(() => desktopBridge?.onPullState?.(setNativePulls), []);
  useEffect(() => setNativeLmsProgress(5), [activeLms?.id]);
  useEffect(() => {
    if (activeLms?.state === "blocked")
      setLmsPhase(`Waiting · ${activeLms.blockedBy ?? "school view"}`);
    else if (activeLms?.state === "queued")
      setLmsPhase("Waiting for school refresh");
  }, [activeLms?.id, activeLms?.state, activeLms?.blockedBy]);
  const [lmsPhase, setLmsPhase] = useState("Ready");
  const [loginSteps, setLoginSteps] = useState<
    Partial<Record<"science" | "tokyo-tech", string>>
  >({});
  const pulling = lmsProgress !== null;
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
    const offIngest = desktopBridge?.onSchoolIngest?.((outcome) => {
      if (
        outcome.site === "kyomu" &&
        (outcome.state === "ok" || outcome.state === "unavailable")
      )
        setKyomuProgress((value) =>
          value === null ? value : Math.min(95, Math.max(value, 15) + 20),
        );
    });
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
  };
}
