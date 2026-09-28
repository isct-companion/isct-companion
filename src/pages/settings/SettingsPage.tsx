import { t } from "../../i18n";
import { UiButton, UiInput, UiSelect } from "../../components/mantine";
import { AdvancedSettings } from "./AdvancedSettings";
import { SchoolAccounts } from "./SchoolAccounts";

import { useRef, useState } from "react";

import { workspaceActions } from "../../store/workspace";
import {
  detectPlatform,
  resolveCapabilities,
  type CapabilityId,
} from "../../platform/capabilities";
import {
  CloudTransportError,
  createCloudAdapter,
  type CloudAdapter,
} from "../../connectors/cloud";
import { createRecoveryCipher, type RecoveryKey } from "../../sync/crypto";

import { performAction } from "../../platform/actions";

import { CalendarManager } from "./calendar";

import type {
  Course,
  PersonalEvent,
  TermId,
  Workspace,
} from "../../core/model";

import { desktopBridge } from "../../app/native";
import type { SchoolAuthProfile } from "../../platform/desktop-bridge";
import { type Section, type Platform } from "../../app/shared";

import { Row, Toggle, Capability } from "../../components/forms";

const labels: Record<CapabilityId, string> = {
  "local-notifications": "Local reminders",
  "remote-push": "Remote push",
  "school-web-session": "School web session",
  "cloud-sync": "Cloud sync",
  "background-sync": "Background refresh",
  "site-customization": "School-site customization",
};

export function Settings({
  platform,
  data,
  update,
  advancedCalendarUpdate,
  caps,
  section,
  setSection,
  notify,
  showLoginPage,
}: {
  platform: Platform;
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
  advancedCalendarUpdate: (f: (d: Workspace) => Workspace) => void;
  caps: ReturnType<typeof resolveCapabilities>;
  section: Section;
  setSection: (s: Section) => void;
  notify: (s: string) => void;
  showLoginPage: (profile: SchoolAuthProfile) => void;
}) {
  const [recoveryPhrase, setRecoveryPhrase] = useState("");
  const [cloudConflict, setCloudConflict] = useState<Workspace | null>(null);
  const pendingCloud = useRef<CloudAdapter | null>(null);
  const cloudAvailable =
    platform === "browser-preview" ||
    caps["cloud-sync"].support === "supported";
  const advancedAvailable =
    platform === "native-desktop" &&
    caps["site-customization"].support === "supported";
  const sections: { id: Section; label: string }[] = [
    { id: "accounts", label: "Accounts" },
    { id: "general", label: "General" },
    { id: "sync", label: "Sync" },
    { id: "notifications", label: "Notifications" },
    { id: "calendar", label: "Academic calendar" },
    { id: "data", label: "Data" },
    ...(advancedAvailable
      ? [{ id: "advanced" as const, label: "Advanced" }]
      : []),
  ];
  const cloudAdapter = async () => {
    if (
      detectPlatform() !== "browser-preview" &&
      caps["cloud-sync"].support !== "supported"
    )
      throw new Error(caps["cloud-sync"].reason);
    if (data.settings.cloud.provider === "none")
      throw new Error("Choose Google Drive or OneDrive first.");
    if (!recoveryPhrase)
      throw new Error("Enter your recovery phrase for this session.");
    const stored = localStorage.getItem("stws-cloud-recovery");
    const prior = stored ? (JSON.parse(stored) as RecoveryKey) : undefined;
    const { cipher, recovery } = await createRecoveryCipher(
      recoveryPhrase,
      prior,
    );
    if (!prior)
      localStorage.setItem("stws-cloud-recovery", JSON.stringify(recovery));
    const bridge = desktopBridge;
    const desktopOAuth = bridge
      ? {
          oauth: {
            connect: () =>
              bridge
                .connectCloud(
                  data.settings.cloud.provider as "google-drive" | "onedrive",
                  data.settings.cloud.clientId,
                  data.settings.cloud.redirectUri!,
                )
                .then(() => undefined),
            requireAccessToken: async () => "protected-main-process-token",
          },
          fetch: async (url: string | URL | Request, init?: RequestInit) => {
            const body = init?.body;
            const binary =
              body instanceof Blob
                ? new Uint8Array(await body.arrayBuffer())
                : body instanceof ArrayBuffer
                  ? new Uint8Array(body)
                  : body instanceof Uint8Array
                    ? body
                    : undefined;
            if (body && typeof body !== "string" && !binary)
              throw new Error("Unsupported cloud request body.");
            const request = await desktopBridge!.cloudRequest(
              data.settings.cloud.provider as "google-drive" | "onedrive",
              url instanceof Request ? url.url : String(url),
              {
                method: init?.method,
                headers: Object.fromEntries(new Headers(init?.headers)),
                bodyText: typeof body === "string" ? body : undefined,
                bodyBase64: binary
                  ? btoa(String.fromCharCode(...binary))
                  : undefined,
              },
            );
            return new Response(
              Uint8Array.from(atob(request.body), (c) => c.charCodeAt(0)),
              {
                status: request.status,
                statusText: request.statusText,
                headers: request.headers,
              },
            );
          },
        }
      : {};
    return createCloudAdapter(
      data.settings.cloud.provider,
      data.settings.cloud.clientId,
      {
        redirectUri: desktopBridge
          ? data.settings.cloud.redirectUri
          : `${location.origin}${location.pathname}`,
        cipher,
        readOnly: !desktopBridge,
        deviceId: data.deviceId,
        ...desktopOAuth,
      },
    );
  };
  const saveCloud = async () => {
    try {
      const a = await cloudAdapter();
      await a.connect();
      notify("Cloud authorized. Nothing uploaded yet.");
    } catch (e) {
      notify(e instanceof Error ? e.message : "Cloud setup failed");
    }
  };
  const pullCloud = async () => {
    try {
      const adapter = await cloudAdapter();
      const remote = await adapter.pull();
      const choice = confirm(
        "Replace this device's workspace with the latest cloud backup?",
      );
      if (!choice) return notify("Kept the current workspace.");
      workspaceActions.replaceWorkspace(remote);
      notify("Using the latest cloud backup.");
    } catch (error) {
      notify(error instanceof Error ? error.message : "Cloud restore failed");
    }
  };
  const pushCloud = async () => {
    try {
      const adapter = await cloudAdapter();
      try {
        const remote = await adapter.pull();
        if (JSON.stringify(remote) !== JSON.stringify(data)) {
          pendingCloud.current = adapter;
          setCloudConflict(remote);
          return;
        }
      } catch (error) {
        if (!(error instanceof CloudTransportError) || error.status !== 404)
          throw error;
      }
      await performAction("push-cloud", detectPlatform(), () =>
        adapter.push(data),
      );
      notify("Encrypted backup uploaded.");
    } catch (error) {
      notify(error instanceof Error ? error.message : "Cloud backup failed");
    }
  };
  const requestNotifications = async () => {
    let p: NotificationPermission;
    if (detectPlatform() === "browser-preview") {
      if (!("Notification" in window)) return;
      p = await Notification.requestPermission();
    } else {
      if (!desktopBridge || caps["local-notifications"].support !== "supported")
        return notify(caps["local-notifications"].reason);
      p = await desktopBridge.requestNotificationPermission();
    }
    update((d) => {
      d.settings.notifications.localPermission = p;
      return d;
    });
    if (p === "granted" && desktopBridge)
      await desktopBridge.notify(
        "Science Tokyo Workstation",
        "Local reminders are enabled.",
      );
  };
  const refreshSchool = async () => {
    if (!desktopBridge || caps["school-web-session"].support !== "supported")
      return notify(caps["school-web-session"].reason);
    const result = await performAction("import-school", detectPlatform(), () =>
      desktopBridge!.importSchool(),
    );
    workspaceActions.applySchoolImport(result);
    notify(result.message);
  };
  const exportData = async () => {
    const blob = new Blob([workspaceActions.exportData()], {
      type: "application/json",
    });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = "science-tokyo-workspace.json";
    a.click();
    URL.revokeObjectURL(a.href);
  };
  const resetData = async () => {
    if (
      !confirm(
        "Reset to the sample workspace? Everything on this device is replaced.",
      )
    )
      return;
    workspaceActions.reset();
    notify("Workspace reset");
  };
  return (
    <div className="page settings">
      <nav className="settingsNav" aria-label="Settings sections">
        {sections.map((s) => (
          <UiButton
            className={section === s.id ? "active" : ""}
            onClick={() => setSection(s.id)}
            key={s.id}
          >
            {t(s.label)}
          </UiButton>
        ))}
      </nav>
      <section className="settingsPane">
        {section === "general" ? (
          <>
            <h2>{t("General")}</h2>
            <Row title={t("Theme")}>
              <UiSelect
                value={data.settings.appearance}
                onChange={(e) =>
                  update((d) => {
                    d.settings.appearance = e.target
                      .value as Workspace["settings"]["appearance"];
                    return d;
                  })
                }
              >
                <option value="system">{t("System")}</option>
                <option value="light">{t("Light")}</option>
                <option value="dark">{t("Dark")}</option>
              </UiSelect>
            </Row>
            <Row title={t("Density")}>
              <UiSelect
                value={data.settings.density}
                onChange={(e) =>
                  update((d) => {
                    d.settings.density = e.target
                      .value as Workspace["settings"]["density"];
                    return d;
                  })
                }
              >
                <option value="comfortable">{t("Comfortable")}</option>
                <option value="compact">{t("Compact")}</option>
              </UiSelect>
            </Row>
            <Row title={t("Text size")}>
              <UiSelect
                value={data.settings.textSize ?? "medium"}
                onChange={(e) =>
                  update((d) => {
                    d.settings.textSize = e.target.value as
                      "small" | "medium" | "large";
                    return d;
                  })
                }
              >
                <option value="small">{t("Small")}</option>
                <option value="medium">{t("Medium")}</option>
                <option value="large">{t("Large")}</option>
              </UiSelect>
            </Row>
            <Row
              title={t("Language")}
              hint={t("Interface and official syllabus language")}
            >
              <UiSelect
                value={data.settings.locale === "ja" ? "ja" : "en"}
                onChange={(event) =>
                  update((d) => {
                    d.settings.locale = event.target.value as
                      "en" | "ja" | "zh";
                    return d;
                  })
                }
              >
                <option value="en">{t("English")}</option>
                <option value="ja">日本語</option>
              </UiSelect>
            </Row>
            <Row
              title={t("Shortcuts")}
              hint={t("⌘1–⌘5 switch pages · Esc closes panels")}
            >
              <span />
            </Row>
          </>
        ) : section === "accounts" ? (
          <>
            <h2>{t("Accounts")}</h2>
            <p className="settingsIntro">
              Connect once. School services reuse the same signed-in session.
            </p>
            <SchoolAccounts notify={notify} showLoginPage={showLoginPage} />
            {desktopBridge &&
              caps["school-web-session"].support === "supported" && (
                <div className="actions end">
                  <UiButton className="button" onClick={refreshSchool}>
                    {t("Refresh records")}
                  </UiButton>
                </div>
              )}
          </>
        ) : section === "sync" ? (
          <>
            <h2>{t("Sync")}</h2>
            <p className="settingsIntro">
              Encrypted workspace sync is separate from school accounts.
            </p>
            <Row
              title={t("Cloud provider")}
              hint={t("Private app folder, encrypted")}
            >
              <UiSelect
                value={data.settings.cloud.provider}
                onChange={(e) =>
                  update((d) => {
                    d.settings.cloud.provider = e.target
                      .value as Workspace["settings"]["cloud"]["provider"];
                    d.syncStatus.state = "needs-configuration";
                    return d;
                  })
                }
              >
                <option value="none">{t("None")}</option>
                <option value="google-drive">Google Drive</option>
                <option value="onedrive">OneDrive</option>
              </UiSelect>
            </Row>
            <Row title={t("OAuth client ID")}>
              <UiInput
                value={data.settings.cloud.clientId}
                placeholder={t("Public client ID")}
                onChange={(e) =>
                  update((d) => {
                    d.settings.cloud.clientId = e.target.value;
                    return d;
                  })
                }
              />
            </Row>
            <Row title={t("When the cloud copy differs")}>
              <UiSelect
                value={data.settings.cloud.conflict}
                onChange={(event) =>
                  update((d) => {
                    d.settings.cloud.conflict = event.target.value as
                      "keep-current" | "use-latest" | "resolve-manually";
                    return d;
                  })
                }
              >
                <option value="keep-current">{t("Keep this device")}</option>
                <option value="use-latest">{t("Use latest")}</option>
                {platform === "native-desktop" && (
                  <option value="resolve-manually">{t("Ask me")}</option>
                )}
              </UiSelect>
            </Row>
            <Row title={t("Recovery phrase")} hint={t("Kept in memory only")}>
              <UiInput
                type="password"
                value={recoveryPhrase}
                placeholder={t("At least 12 characters")}
                onChange={(event) => setRecoveryPhrase(event.target.value)}
              />
            </Row>
            <div className="actions end">
              <UiButton
                className="button"
                disabled={!cloudAvailable}
                onClick={saveCloud}
              >
                {t("Authorize")}
              </UiButton>
              <UiButton
                className="button"
                disabled={!cloudAvailable}
                onClick={pullCloud}
              >
                {t("Restore latest")}
              </UiButton>
              {desktopBridge && cloudAvailable && (
                <UiButton className="button primary" onClick={pushCloud}>
                  {t("Back up now")}
                </UiButton>
              )}
            </div>
            {cloudConflict && (
              <div className="alert" role="alert">
                <b>{t("The cloud backup changed")}</b>
                <div className="actions">
                  <UiButton
                    className="button"
                    onClick={async () => {
                      await pendingCloud.current?.push(data);
                      setCloudConflict(null);
                      notify("Kept this device and uploaded it.");
                    }}
                  >
                    {t("Keep this device")}
                  </UiButton>
                  <UiButton
                    className="button"
                    onClick={() => {
                      workspaceActions.replaceWorkspace(cloudConflict);
                      setCloudConflict(null);
                      notify("Using the latest cloud backup.");
                    }}
                  >
                    {t("Use latest")}
                  </UiButton>
                  {platform === "native-desktop" && (
                    <UiButton
                      className="button"
                      onClick={() => {
                        setSection("data");
                        notify("Export this device's data, then choose.");
                      }}
                    >
                      {t("Export first")}
                    </UiButton>
                  )}
                </div>
              </div>
            )}
          </>
        ) : section === "notifications" ? (
          <>
            <h2>{t("Notifications")}</h2>
            <Capability
              name="Local reminders"
              value={caps["local-notifications"]}
            >
              {caps["local-notifications"].readiness !== "ready" && (
                <UiButton
                  className="button small"
                  onClick={requestNotifications}
                >
                  {t("Allow")}
                </UiButton>
              )}
            </Capability>
            <Row title={t("Class reminders")}>
              <Toggle
                checked={data.settings.notifications.classes}
                onChange={(v) =>
                  update((d) => {
                    d.settings.notifications.classes = v;
                    return d;
                  })
                }
              />
            </Row>
            <Row title={t("Assignment reminders")}>
              <Toggle
                checked={data.settings.notifications.assignments}
                onChange={(v) =>
                  update((d) => {
                    d.settings.notifications.assignments = v;
                    return d;
                  })
                }
              />
            </Row>
            <Capability name="Remote push" value={caps["remote-push"]} />
            <Capability
              name="Background refresh"
              value={caps["background-sync"]}
            />
          </>
        ) : section === "calendar" ? (
          <CalendarManager
            data={data}
            update={advancedCalendarUpdate}
            notify={notify}
            readOnly={platform !== "native-desktop"}
          />
        ) : section === "data" ? (
          <>
            <h2>{t("Data")}</h2>
            <Row
              title={
                platform === "browser-preview"
                  ? "Browser preview"
                  : platform === "native-mobile"
                    ? "Native mobile"
                    : "Desktop"
              }
              hint={t("Everything is stored on this device.")}
            >
              <div className="actions">
                <UiButton className="button" onClick={exportData}>
                  {t("Export JSON")}
                </UiButton>
                <UiButton className="button danger" onClick={resetData}>
                  {t("Reset")}
                </UiButton>
              </div>
            </Row>
            <details className="diagnostics">
              <summary>{t("Diagnostics")}</summary>
              {Object.entries(caps).map(([k, v]) => (
                <Capability
                  key={k}
                  name={t(labels[k as CapabilityId])}
                  value={v}
                />
              ))}
              <h3>{t("Recent activity")}</h3>
              <div className="logList">
                {(data.logs ?? []).length ? (
                  (data.logs ?? []).slice(0, 12).map((log, index) => (
                    <div key={`${log.at}-${index}`}>
                      <b>
                        {log.source} · {log.phase} · {log.state}
                      </b>
                      <span>
                        {new Date(log.at).toLocaleString()} · {log.message}
                      </span>
                    </div>
                  ))
                ) : (
                  <span className="muted">
                    {t("No import or cloud activity yet.")}
                  </span>
                )}
              </div>
            </details>
          </>
        ) : advancedAvailable ? (
          <AdvancedSettings
            data={data}
            update={(fn) => {
              void performAction("customize-school-site", platform, () =>
                update(fn),
              ).catch((error) =>
                notify(
                  error instanceof Error ? error.message : "Action denied",
                ),
              );
            }}
          />
        ) : (
          <p className="muted">
            Customization is available in the desktop app.
          </p>
        )}
      </section>
    </div>
  );
}
