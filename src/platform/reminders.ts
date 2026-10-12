import { t } from "../i18n";
import {
  contentLanguage,
  pickLocalized,
  type TermId,
  type Workspace,
} from "../core/model";
import {
  resolveDayAgenda,
  scheduledCourses,
  tokyoDate,
} from "../core/schedule";

export const CLASS_LEAD_CHOICES = [5, 10, 15, 30] as const;
export const DEFAULT_CLASS_LEAD = 10;
/** Fixed assignment lead times: one day and one hour before the deadline. */
const ASSIGNMENT_LEADS = [24 * 60, 60] as const;
/** A reminder missed by more than this (sleep, app closed) is skipped. */
export const REMINDER_GRACE_MS = 5 * 60_000;

export type Reminder = { key: string; at: number; title: string; body: string };

/** The term whose timetable Today shows for this moment. */
export function reminderTerm(data: Workspace, _now: Date): TermId {
  const current = data.currentTerm;
  return data.courses.some((row) => row.termId === current && !row.deletedAt)
    ? current
    : data.selectedTerm;
}

const atTokyo = (date: string, time: string) =>
  Date.parse(`${date}T${time}:00+09:00`);

/**
 * Reminders whose time has come within the grace window. Classes and exams
 * come from the same agenda as Today, so cancelled days and replacement
 * timetables are already applied. Pure: callers de-duplicate by `key`.
 */
export function dueReminders(data: Workspace, now: Date): Reminder[] {
  const nowMs = now.getTime();
  const due = (at: number) => at <= nowMs && at > nowMs - REMINDER_GRACE_MS;
  const reminders: Reminder[] = [];
  const settings = data.settings.notifications;
  const locale = contentLanguage(data.settings);

  if (settings.classes) {
    const lead = (settings.classLeadMinutes ?? DEFAULT_CLASS_LEAD) * 60_000;
    const term = reminderTerm(data, now);
    const today = tokyoDate(now);
    const courses = scheduledCourses(data, term);
    for (const block of resolveDayAgenda(data, term, today, {
      courses,
      events: [],
      includePersonal: false,
    })) {
      if (
        block.cancelled ||
        (block.kind !== "course" && block.kind !== "exam") ||
        !/^\d{2}:\d{2}$/.test(block.startTime)
      )
        continue;
      const start = atTokyo(block.date, block.startTime);
      if (!due(start - lead)) continue;
      const minutes = Math.round((start - nowMs) / 60_000);
      reminders.push({
        key: `class:${block.key}`,
        at: start - lead,
        title:
          block.kind === "exam" ? `${t("Exam")} · ${block.title}` : block.title,
        body: [
          minutes > 0
            ? t("Starts in {{minutes}} min", { minutes })
            : t("Starting now"),
          block.startTime,
          block.room,
        ]
          .filter(Boolean)
          .join(" · "),
      });
    }
  }

  if (settings.assignments) {
    for (const assignment of data.assignments) {
      if (
        assignment.deletedAt ||
        assignment.missingSince ||
        assignment.hidden ||
        assignment.submission === "submitted" ||
        assignment.submission === "graded" ||
        assignment.submissionStatus === "submitted" ||
        !assignment.dueAt
      )
        continue;
      const dueAt = Date.parse(assignment.dueAt);
      if (!Number.isFinite(dueAt)) continue;
      const course = data.courses.find((row) => row.id === assignment.courseId);
      const courseTitle = course
        ? pickLocalized(course.names?.title, locale, course.title)
        : "";
      for (const lead of ASSIGNMENT_LEADS) {
        const at = dueAt - lead * 60_000;
        if (!due(at)) continue;
        reminders.push({
          key: `assignment:${assignment.id}:${lead}`,
          at,
          title: assignment.title,
          body: [
            lead >= 24 * 60 ? t("Due in 24 hours") : t("Due in 1 hour"),
            courseTitle,
          ]
            .filter(Boolean)
            .join(" · "),
        });
      }
    }
  }
  return reminders;
}

const FIRED_KEY = "isct-reminders-fired";
const FIRED_TTL_MS = 3 * 24 * 60 * 60_000;

/** Device-local record of delivered reminders so none fires twice. */
export function firedReminderLog(
  storage: Pick<Storage, "getItem" | "setItem"> | undefined,
) {
  let fired: Record<string, number> = {};
  try {
    fired = JSON.parse(storage?.getItem(FIRED_KEY) ?? "{}") ?? {};
  } catch {
    fired = {};
  }
  return {
    has: (key: string) => key in fired,
    add(key: string, now: number) {
      fired[key] = now;
      for (const [entry, at] of Object.entries(fired))
        if (now - at > FIRED_TTL_MS) delete fired[entry];
      try {
        storage?.setItem(FIRED_KEY, JSON.stringify(fired));
      } catch {
        /* Session-only when storage is blocked. */
      }
    },
  };
}

/**
 * Delivers due reminders while the app is open. Runs every 30 seconds and
 * immediately after wake; nothing is scheduled without notification permission.
 */
export function startReminderRuntime({
  getWorkspace,
  deliver,
  storage = typeof localStorage === "undefined" ? undefined : localStorage,
  intervalMs = 30_000,
  now = () => new Date(),
}: {
  getWorkspace: () => Workspace | null | undefined;
  deliver: (reminder: Reminder) => Promise<unknown> | unknown;
  storage?: Pick<Storage, "getItem" | "setItem">;
  intervalMs?: number;
  now?: () => Date;
}) {
  const log = firedReminderLog(storage);
  const tick = () => {
    const data = getWorkspace();
    if (!data || data.settings.notifications.localPermission !== "granted")
      return;
    const current = now();
    for (const reminder of dueReminders(data, current)) {
      if (log.has(reminder.key)) continue;
      log.add(reminder.key, current.getTime());
      void Promise.resolve(deliver(reminder)).catch(() => undefined);
    }
  };
  tick();
  const timer = setInterval(tick, intervalMs);
  const wake = () => {
    if (
      typeof document === "undefined" ||
      document.visibilityState === "visible"
    )
      tick();
  };
  if (typeof document !== "undefined")
    document.addEventListener("visibilitychange", wake);
  return () => {
    clearInterval(timer);
    if (typeof document !== "undefined")
      document.removeEventListener("visibilitychange", wake);
  };
}
