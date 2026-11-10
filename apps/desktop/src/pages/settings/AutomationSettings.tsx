import { Clock } from "lucide-react";
import { notify as appNotify } from "../../app/notify";
import { errorText } from "../../app/shared";
import { useState } from "react";
import { t, type MessageKey } from "../../i18n";
import type { ScheduledTask, Workspace } from "../../lib/model";
import type { WorkspaceIntent } from "../../lib/workspace-intents";
import { UiButton, EmptyState } from "../../components/ui";
import { Row, SettingsGroup, Toggle } from "../../components/forms";
import { formatDay } from "./subscription-status";

/** What Core stores as a task's `lastResult`; any other text shows as stored. */
const RESULT_KEYS: Record<string, MessageKey> = {
  Running: "settings.automation.result.running",
  "Export completed": "settings.automation.result.exportCompleted",
};

const TASKS: Partial<
  Record<ScheduledTask["action"], { title: MessageKey; hint: MessageKey }>
> = {
  "reference-refresh": {
    title: "settings.automation.checkForOfficialDataUpdates",
    hint: "settings.automation.calendarExamsAndStudyGuide",
  },
  "lms-refresh": {
    title: "settings.automation.refreshLMSInTheBackground",
    hint: "settings.automation.every4HoursOnWi",
  },
  "school-refresh": {
    title: "settings.automation.refreshSchoolRecords",
    hint: "settings.automation.lmsAndWeb",
  },
  export: {
    title: "settings.automation.saveABackupFile",
    hint: "settings.automation.writesABackupInThe",
  },
};

/** Jobs the app runs on its own while it is open; shown inside Data updates. */
/** Plain-language summary of the simple RRULEs the app creates. */
function describeSchedule(schedule: string) {
  if (schedule === "launch") return t("settings.automation.whenTheAppStarts");
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
      ? t("settings.automation.everyHour")
      : t("settings.automation.everyCountHours", { count: interval });
  if (rule.FREQ === "DAILY")
    return time
      ? t("settings.automation.everyDayAtTime", { time })
      : t("settings.automation.everyDay");
  if (rule.FREQ === "WEEKLY") return t("settings.automation.everyWeek");
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
      result = errorText(error) ?? t("settings.automation.runFailed");
    } finally {
      setRunning(undefined);
    }
  };
  return (
    <>
      <SettingsGroup title="settings.automation.scheduledTasks">
        {tasks.length === 0 && (
          <EmptyState compact icon={Clock}>
            {t("settings.automation.noScheduledTasks")}
          </EmptyState>
        )}
        {tasks.map((task, index) => {
          const known = TASKS[task.action];
          const last = task.lastRunAt
            ? `${t("settings.automation.lastRun")} ${formatDay(
                task.lastRunAt,
                locale,
                {
                  hour: "2-digit",
                  minute: "2-digit",
                },
              )}${task.lastResult ? ` · ${RESULT_KEYS[task.lastResult] ? t(RESULT_KEYS[task.lastResult]) : task.lastResult}` : ""}`
            : t("settings.automation.notRunYet");
          return (
            <Row
              key={task.id}
              id={index === 0 ? "tasks" : undefined}
              group
              {...(known ? { title: known.title } : { titleText: task.name })}
              hint={`${known ? t(known.hint) : describeSchedule(task.schedule)} · ${last}`}
            >
              <div className="actions">
                <UiButton
                  className="button small"
                  disabled={!runScheduledTask || running === task.id}
                  onClick={() => void runNow(task)}
                >
                  {running === task.id
                    ? t("settings.automation.running")
                    : t("settings.automation.runNow")}
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
