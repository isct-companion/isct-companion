import { MultiSelect } from "@mantine/core";
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
import { UiButton, UiSelect } from "../../components/mantine";
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
        ? t("1 day before")
        : t("{{days}} days before", { days: minutes / 1440 })
      : minutes % 60 === 0
        ? minutes === 60
          ? t("1 hour before")
          : t("{{hours}} hours before", { hours: minutes / 60 })
        : t("{{minutes}} minutes before", { minutes });
  const capability = caps["local-notifications"];
  const allowed = settings.localPermission === "granted";
  const canAsk = capability.support === "supported";

  // Asking again returns the system's real answer (and prompts only the first
  // time), so the stored permission follows System Settings.
  const requestPermission = async (
    body = t("Notifications are on."),
    confirmDelivery = false,
  ) => {
    if (!desktopBridge || !canAsk) return notify(capability.reason);
    try {
      const permission = await desktopBridge.requestNotificationPermission();
      update((draft) => {
        draft.settings.notifications.localPermission = permission;
        return draft;
      });
      if (permission === "granted") {
        await desktopBridge.notify("IsctCompanion", body);
        if (confirmDelivery)
          notify(
            t(
              "The system accepted the test notification. If it is only in Notification Center, enable banners for IsctCompanion in system settings.",
            ),
          );
      } else notify(t("Notifications are blocked in system settings."));
    } catch (error) {
      notify(String(error));
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
      <SettingsHeader title="Notifications" />
      {settings.localPermission === "denied" ? (
        <Status id="permission" tone="error">
          {t("Notifications are blocked in system settings.")}
        </Status>
      ) : (
        !allowed &&
        canAsk && (
          <Status
            id="permission"
            tone="warn"
            action={
              <UiButton
                className="button small primary"
                onClick={() => void requestPermission()}
              >
                {t("Allow notifications")}
              </UiButton>
            }
          >
            {t("Notifications are off.")}
          </Status>
        )
      )}
      <SettingsGroup>
        <Row id="classes" title="Class reminders">
          <Toggle
            checked={settings.classes}
            onChange={(value) => set((n) => (n.classes = value))}
          />
        </Row>
        {settings.classes && (
          <div
            className="settingsSuboptions"
            role="group"
            aria-label={t("Class reminders")}
          >
            <Row id="class-lead" title="Remind me">
              <UiSelect
                value={settings.classLeadMinutes ?? DEFAULT_CLASS_LEAD}
                onChange={(value) => {
                  const minutes = Number(value);
                  set((n) => (n.classLeadMinutes = minutes));
                }}
                data={CLASS_LEAD_CHOICES.map((minutes) => ({
                  value: String(minutes),
                  label: t("{{minutes}} minutes before", { minutes }),
                }))}
              />
            </Row>
          </div>
        )}
        <Row id="exams" title="Exam reminders">
          <Toggle
            checked={settings.exams ?? settings.classes}
            onChange={(value) => set((n) => (n.exams = value))}
          />
        </Row>
        {(settings.exams ?? settings.classes) && (
          <div
            className="settingsSuboptions"
            role="group"
            aria-label={t("Exam reminders")}
          >
            <Row id="exam-lead" title="Remind me">
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
        <Row id="assignments" title="Assignment reminders">
          <Toggle
            checked={settings.assignments}
            onChange={(value) => set((n) => (n.assignments = value))}
          />
        </Row>
        {settings.assignments && (
          <div
            className="settingsSuboptions"
            role="group"
            aria-label={t("Assignment reminders")}
          >
            <Row id="assignment-lead" title="Remind me" group>
              <MultiSelect
                className="assignmentTimes"
                aria-label={t("Assignment reminder times")}
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
          <Row id="test-notification" title="Test notification">
            <UiButton
              className="button small"
              onClick={() =>
                void requestPermission(t("This is a test notification."), true)
              }
            >
              {t("Send")}
            </UiButton>
          </Row>
        )}
      </SettingsGroup>
    </>
  );
}
