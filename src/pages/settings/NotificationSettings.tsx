import { t } from "../../i18n";
import type { Workspace } from "../../core/model";
import { desktopBridge } from "../../app/native";
import { resolveCapabilities } from "../../platform/capabilities";
import {
  CLASS_LEAD_CHOICES,
  DEFAULT_CLASS_LEAD,
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
  const capability = caps["local-notifications"];
  const allowed = settings.localPermission === "granted";
  const canAsk = capability.support === "supported";

  const requestPermission = async () => {
    if (!desktopBridge || !canAsk) return notify(capability.reason);
    const permission = await desktopBridge.requestNotificationPermission();
    update((draft) => {
      draft.settings.notifications.localPermission = permission;
      return draft;
    });
    if (permission === "granted")
      await desktopBridge.notify("IsctCompanion", t("Notifications are on."));
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
      <SettingsHeader
        title="Notifications"
        description="Reminders for classes and deadlines, shown while the app is open."
      />
      {allowed ? (
        <Status id="permission" tone="ok">
          {t("Notifications are allowed on this device.")}
        </Status>
      ) : settings.localPermission === "denied" ? (
        <Status id="permission" tone="error">
          {t("Notifications are blocked. Allow them in your system settings.")}
        </Status>
      ) : canAsk ? (
        <Status
          id="permission"
          tone="warn"
          action={
            <UiButton
              className="button small primary"
              onClick={requestPermission}
            >
              {t("Allow notifications")}
            </UiButton>
          }
        >
          {t("Allow notifications so reminders can appear.")}
        </Status>
      ) : (
        <Status id="permission" tone="idle">
          {t("Notifications are not available here.")}
        </Status>
      )}
      <SettingsGroup title="Reminders">
        <Row
          id="classes"
          title="Class reminders"
          hint="Before each class and exam"
        >
          <Toggle
            checked={settings.classes}
            onChange={(value) => set((n) => (n.classes = value))}
          />
        </Row>
        {settings.classes && (
          <Row id="class-lead" title="Remind me">
            <UiSelect
              value={settings.classLeadMinutes ?? DEFAULT_CLASS_LEAD}
              onChange={(event) =>
                set((n) => (n.classLeadMinutes = Number(event.target.value)))
              }
            >
              {CLASS_LEAD_CHOICES.map((minutes) => (
                <option key={minutes} value={minutes}>
                  {t("{{minutes}} minutes before", { minutes })}
                </option>
              ))}
            </UiSelect>
          </Row>
        )}
        <Row
          id="assignments"
          title="Assignment reminders"
          hint="One day and one hour before the deadline, until you submit"
        >
          <Toggle
            checked={settings.assignments}
            onChange={(value) => set((n) => (n.assignments = value))}
          />
        </Row>
      </SettingsGroup>
    </>
  );
}
