import { notify as appNotify } from "../../app/notify";
import { errorText } from "../../app/shared";
import { useState } from "react";
import { t } from "../../i18n";
import type { ScheduledTask, Workspace } from "../../lib/model";
import type { WorkspaceIntent } from "../../lib/workspace-intents";
import { UiButton } from "../../components/mantine";
import {
  Row,
  SettingsGroup,
  SettingsHeader,
  Toggle,
} from "../../components/forms";
import { formatDay } from "./subscription-status";

const TASKS: Partial<
  Record<ScheduledTask["action"], { title: string; hint: string }>
> = {
  "reference-refresh": {
    title: "Check for official data updates",
    hint: "Calendar, exams and study guide · once a day",
  },
  "lms-refresh": {
    title: "Refresh LMS in the background",
    hint: "Every 4 hours on Wi-Fi while you use the app",
  },
  "school-refresh": {
    title: "Refresh school records",
    hint: "LMS and 教務Web",
  },
  export: {
    title: "Save a backup file",
    hint: "Writes a backup in the app folder",
  },
};

/** Plain-language summary of the simple RRULEs the app creates. */
function describeSchedule(schedule: string) {
  if (schedule === "launch") return t("When the app starts");
  const rule = Object.fromEntries(
    schedule
      .replace(/^RRULE:/, "")
      .split(";")
      .map((part) => part.split("=") as [string, string]),
  );
  const interval = Number(rule.INTERVAL ?? 1);
  const time =
    rule.BYHOUR !== undefined
      ? `${rule.BYHOUR.padStart(2, "0")}:${(rule.BYMINUTE ?? "0").padStart(2, "0")}`
      : undefined;
  if (rule.FREQ === "HOURLY")
    return interval === 1
      ? t("Every hour")
      : t("Every {{count}} hours", { count: interval });
  if (rule.FREQ === "DAILY")
    return time ? t("Every day at {{time}}", { time }) : t("Every day");
  if (rule.FREQ === "WEEKLY") return t("Every week");
  return schedule;
}

export function AutomationSettings({
  data,
  mutate,
  runScheduledTask,
}: {
  data: Workspace;
  mutate: (intent: WorkspaceIntent) => Promise<unknown>;
  runScheduledTask?: (task: ScheduledTask) => Promise<string>;
}) {
  const [running, setRunning] = useState<string>();
  const locale = data.settings.locale;
  const tasks = data.scheduledTasks.filter((task) => !task.deletedAt);
  const runNow = async (task: ScheduledTask) => {
    if (!runScheduledTask) return;
    setRunning(task.id);
    let result: string;
    try {
      result = await runScheduledTask(task);
    } catch (error) {
      result = errorText(error) ?? t("Run failed");
    } finally {
      setRunning(undefined);
    }
  };
  return (
    <>
      <SettingsHeader
        title="Automation"
        description="Jobs the app runs on its own while it is open. Nothing runs when the app is closed."
      />
      <SettingsGroup>
        {tasks.length === 0 && (
          <p className="paneEmpty">{t("No scheduled tasks")}</p>
        )}
        {tasks.map((task, index) => {
          const known = TASKS[task.action];
          const last = task.lastRunAt
            ? `${t("Last run")} ${formatDay(task.lastRunAt, locale, {
                hour: "2-digit",
                minute: "2-digit",
              })}${task.lastResult ? ` · ${t(task.lastResult)}` : ""}`
            : t("Not run yet");
          return (
            <Row
              key={task.id}
              id={index === 0 ? "tasks" : undefined}
              group
              title={known?.title ?? task.name}
              hint={`${known ? t(known.hint) : describeSchedule(task.schedule)} · ${last}`}
            >
              <div className="actions">
                <UiButton
                  className="button small"
                  disabled={!runScheduledTask || running === task.id}
                  onClick={() => void runNow(task)}
                >
                  {running === task.id ? t("Running…") : t("Run now")}
                </UiButton>
                <Toggle
                  checked={task.enabled}
                  onChange={(enabled) =>
                    void mutate({
                      kind: "saveScheduledTask",
                      draft: { ...task, enabled },
                      existingId: task.id,
                    }).catch(appNotify.error)
                  }
                />
              </div>
            </Row>
          );
        })}
      </SettingsGroup>
    </>
  );
}
