import { MultiSelect } from "../../components/ui";
import { t } from "../../i18n";
import type { Workspace } from "../../lib/model";
import { desktopBridge } from "../../app/native";
import { resolveCapabilities } from "../../platform/capabilities";
import {
  CLASS_LEAD_CHOICES,
  DEFAULT_CLASS_LEAD,
  ASSIGNMENT_LEAD_CHOICES,
  DEFAULT_ASSIGNMENT_LEADS,
} from "../../platform/reminders";
import { UiButton, UiSelect } from "../../components/ui";
import {
  Row,
  SettingsGroup,
  SettingsHeader,
  Status,
  Toggle,
} from "../../components/forms";

export function NotificationSettings({
  data,
  update,
  caps,
  notify,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
  caps: ReturnType<typeof resolveCapabilities>;
  notify: (message: string) => void;
}) {
  const settings = data.settings.notifications;
  const assignmentLeads =
    settings.assignmentLeadMinutes ?? DEFAULT_ASSIGNMENT_LEADS;
  const leadLabel = (minutes: number) =>
    minutes % 1440 === 0
      ? minutes === 1440
        ? t("settings.notification.oneDayBefore")
        : t("settings.notification.daysBefore", { days: minutes / 1440 })
      : minutes % 60 === 0
        ? minutes === 60
          ? t("settings.notification.oneHourBefore")
          : t("settings.notification.hoursBefore", { hours: minutes / 60 })
        : t("settings.notification.minutesBefore", { minutes });
  const capability = caps["local-notifications"];
  const allowed = settings.localPermission === "granted";
  const canAsk = capability.support === "supported";

  // Asking again returns the system's real answer (and prompts only the first
  // time), so the stored permission follows System Settings.
  const requestPermission = async (
    body = t("settings.notification.notificationsAreOn"),
    confirmDelivery = false,
  ) => {
    if (!desktopBridge || !canAsk) return notify(t(capability.reason));
    try {
      const permission = await desktopBridge.requestNotificationPermission();
      update((draft) => {
        draft.settings.notifications.localPermission = permission;
        return draft;
      });
      if (permission === "granted") {
        await desktopBridge.notify("IsctCompanion", body);
        if (confirmDelivery)
          notify(t("settings.notification.theSystemAcceptedTheTest"));
      } else if (permission === "denied") {
        await desktopBridge.openNotificationSettings();
        notify(
          t("settings.notification.allowIsctCompanionNotificationsInSystem"),
        );
      } else notify(t("settings.notification.notificationsAreOff"));
    } catch (error) {
      notify(error instanceof Error ? error.message : String(error));
    }
  };

  const set = (
    fn: (notifications: Workspace["settings"]["notifications"]) => void,
  ) =>
    update((draft) => {
      fn(draft.settings.notifications);
      return draft;
    });

  return (
    <>
      <SettingsHeader title="common.notifications" />
      {!canAsk ? (
        <Status id="permission" tone="warn">
          {t(capability.reason)}
        </Status>
      ) : (
        !allowed && (
          <Status
            id="permission"
            tone={settings.localPermission === "denied" ? "error" : "warn"}
            action={
              <UiButton
                className="button small primary"
                onClick={() => void requestPermission()}
              >
                {t(
                  settings.localPermission === "denied"
                    ? "settings.notification.openNotificationSettings"
                    : "settings.notification.allowNotifications",
                )}
              </UiButton>
            }
          >
            {t(
              settings.localPermission === "denied"
                ? "settings.notification.notificationsAreBlockedInSystem"
                : "settings.notification.notificationsAreOff",
            )}
          </Status>
        )
      )}
      <SettingsGroup>
        <Row id="classes" title="settings.notification.classReminders">
          <Toggle
            checked={settings.classes}
            onChange={(value) => set((n) => (n.classes = value))}
          />
        </Row>
        {settings.classes && (
          <div
            className="settingsSuboptions"
            role="group"
            aria-label={t("settings.notification.classReminders")}
          >
            <Row id="class-lead" title="settings.main.remindMe">
              <UiSelect
                value={settings.classLeadMinutes ?? DEFAULT_CLASS_LEAD}
                onChange={(value) => {
                  const minutes = Number(value);
                  set((n) => (n.classLeadMinutes = minutes));
                }}
                data={CLASS_LEAD_CHOICES.map((minutes) => ({
                  value: String(minutes),
                  label: t("settings.notification.minutesBefore", { minutes }),
                }))}
              />
            </Row>
          </div>
        )}
        <Row id="exams" title="settings.notification.examReminders">
          <Toggle
            checked={settings.exams ?? settings.classes}
            onChange={(value) => set((n) => (n.exams = value))}
          />
        </Row>
        {(settings.exams ?? settings.classes) && (
          <div
            className="settingsSuboptions"
            role="group"
            aria-label={t("settings.notification.examReminders")}
          >
            <Row id="exam-lead" title="settings.main.remindMe">
              <UiSelect
                value={
                  settings.examLeadMinutes ??
                  settings.classLeadMinutes ??
                  DEFAULT_CLASS_LEAD
                }
                onChange={(value) =>
                  set((n) => (n.examLeadMinutes = Number(value)))
                }
                data={[
                  ...new Set([
                    10,
                    30,
                    60,
                    1440,
                    2880,
                    settings.examLeadMinutes ??
                      settings.classLeadMinutes ??
                      DEFAULT_CLASS_LEAD,
                  ]),
                ]
                  .sort((a, b) => a - b)
                  .map((minutes) => ({
                    value: String(minutes),
                    label: leadLabel(minutes),
                  }))}
              />
            </Row>
          </div>
        )}
        <Row
          id="quizzes"
          title="settings.main.countQuizzesAsExams"
          hint={t("settings.main.quizzesAppearInUpcomingExams")}
        >
          <Toggle
            checked={settings.quizzes ?? true}
            onChange={(value) => set((n) => (n.quizzes = value))}
          />
        </Row>
        <Row id="assignments" title="settings.notification.assignmentReminders">
          <Toggle
            checked={settings.assignments}
            onChange={(value) => set((n) => (n.assignments = value))}
          />
        </Row>
        {settings.assignments && (
          <div
            className="settingsSuboptions"
            role="group"
            aria-label={t("settings.notification.assignmentReminders")}
          >
            <Row id="assignment-lead" title="settings.main.remindMe" group>
              <MultiSelect
                className="assignmentTimes"
                aria-label={t("settings.notification.assignmentReminderTimes")}
                value={assignmentLeads.map(String)}
                data={[
                  ...new Set([...ASSIGNMENT_LEAD_CHOICES, ...assignmentLeads]),
                ]
                  .sort((a, b) => b - a)
                  .map((minutes) => ({
                    value: String(minutes),
                    label: leadLabel(minutes),
                  }))}
                maxValues={8}
                onChange={(values) => {
                  if (!values.length) return;
                  const minutes = values.map(Number);
                  set((n) => (n.assignmentLeadMinutes = minutes));
                }}
              />
            </Row>
          </div>
        )}
        {canAsk && (
          <Row id="test-notification" title="settings.main.testNotification">
            <UiButton
              className="button small"
              onClick={() =>
                void requestPermission(
                  t("settings.notification.thisIsATestNotification"),
                  true,
                )
              }
            >
              {t("settings.notification.send")}
            </UiButton>
          </Row>
        )}
      </SettingsGroup>
    </>
  );
}
