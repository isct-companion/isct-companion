import { t, intlLocale } from "../../i18n";
import { formatTimestamp } from "../../lib/civil-date";
import type { Workspace } from "../../lib/model";
import {
  resolveCapabilities,
  type CapabilityId,
} from "../../platform/capabilities";
import {
  Capability,
  SettingsGroup,
  SettingsHeader,
} from "../../components/forms";

const labels: Record<CapabilityId, string> = {
  "local-notifications": "Local reminders",
  "remote-push": "Remote push",
  "school-web-session": "School web session",
  "cloud-sync": "Cloud sync",
  "background-sync": "Background refresh",
  "site-customization": "School-site customization",
};

export function DiagnosticsSettings({
  data,
  caps,
}: {
  data: Workspace;
  caps: ReturnType<typeof resolveCapabilities>;
}) {
  const logs = data.logs ?? [];
  return (
    <>
      <SettingsHeader
        title="Diagnostics"
        description="What this build and platform support, and recent background activity."
      />
      <SettingsGroup title="Capabilities">
        <div data-setting="capabilities">
          {Object.entries(caps).map(([id, value]) => (
            <Capability
              key={id}
              name={t(labels[id as CapabilityId])}
              value={value}
            />
          ))}
        </div>
      </SettingsGroup>
      <SettingsGroup title="Recent activity">
        <div className="logList">
          {logs.length ? (
            logs.slice(0, 12).map((log, index) => (
              <div key={`${log.at}-${index}`}>
                <b>
                  {log.source} · {log.phase} · {log.state}
                </b>
                <span>
                  {formatTimestamp(log.at, intlLocale(data.settings.locale)) ??
                    log.at}{" "}
                  · {log.message}
                </span>
              </div>
            ))
          ) : (
            <span className="muted">{t("No activity yet")}</span>
          )}
        </div>
      </SettingsGroup>
    </>
  );
}
