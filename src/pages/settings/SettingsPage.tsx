import { Badge, Switch } from "@mantine/core";
import { t } from "../../i18n";
import { SYLLABUS_DEPARTMENTS, departmentById } from "../../core/departments";
import { UiButton, UiInput, UiSelect } from "../../components/mantine";
import { ReferenceSubscriptions } from "./ReferenceSubscriptions";
import { ConfigsSettings, useDeveloperMode } from "./ConfigsSettings";
import { ExtensionsSettings } from "./ExtensionsSettings";
import { FilesSettings } from "./FilesSettings";
import { HiddenItems } from "./HiddenItems";
import { SchoolAccounts } from "./SchoolAccounts";

import { Fragment, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { workspaceActions, workspaceStore } from "../../store/workspace";
import {
  detectPlatform,
  resolveCapabilities,
  type CapabilityId,
} from "../../platform/capabilities";
import {
  CloudTransportError,
  createCloudAdapter,
  resolveCloudClientId,
  type CloudAdapter,
} from "../../connectors/cloud";

import { performAction } from "../../platform/actions";

import { CalendarManager } from "./calendar";

import type {
  Course,
  PersonalEvent,
  ScheduledTask,
  TermId,
  Workspace,
} from "../../core/model";

import { desktopBridge } from "../../app/native";
import type { SchoolAuthProfile } from "../../platform/desktop-bridge";
import { type Section, type Platform } from "../../app/shared";
import { subscriptionNetwork } from "../../platform/subscription-polling";

import { Row, Toggle, Capability } from "../../components/forms";

type ActiveCloudProvider = "google-drive" | "onedrive";

const labels: Record<CapabilityId, string> = {
  "local-notifications": "Local reminders",
  "remote-push": "Remote push",
  "school-web-session": "School web session",
  "cloud-sync": "Cloud sync",
  "background-sync": "Background refresh",
  "site-customization": "School-site customization",
};

function bytesToBase64(bytes: Uint8Array) {
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 0x8000)
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  return btoa(binary);
}

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
  runScheduledTask,
  matrixOcrAvailable,
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
  runScheduledTask?: (task: ScheduledTask) => Promise<string>;
  matrixOcrAvailable: boolean;
}) {
  const [cloudConflict, setCloudConflict] = useState<string | null>(null);
  const [authorizedProviders, setAuthorizedProviders] = useState<
    ActiveCloudProvider[]
  >([]);
  const [cloudBusy, setCloudBusy] = useState(false);
  const [agentCommand, setAgentCommand] = useState(
    "isct-companion-mcp --db /absolute/path/to/school.db",
  );
  useEffect(() => {
    if (!desktopBridge) return;
    void invoke<string>("agent_command")
      .then(setAgentCommand)
      .catch(() => undefined);
  }, []);
  useEffect(() => {
    if (!desktopBridge) return;
    let disposed = false;
    void desktopBridge
      .cloudAuthorizationStatus()
      .then((providers) => {
        if (!disposed) setAuthorizedProviders(providers);
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
    };
  }, []);
  const pendingCloud = useRef<CloudAdapter | null>(null);
  const cloudAvailable =
    platform === "browser-preview" ||
    caps["cloud-sync"].support === "supported";
  const selectedCloudProvider =
    data.settings.cloud.provider === "none"
      ? null
      : data.settings.cloud.provider;
  const cloudConnected = Boolean(
    selectedCloudProvider &&
    (authorizedProviders.includes(selectedCloudProvider) ||
      (!desktopBridge && data.syncStatus.state === "idle")),
  );
  const desktop = platform === "native-desktop";
  const mobile = platform === "native-mobile";
  const extensionsAvailable =
    desktop && caps["site-customization"].support === "supported";
  const referenceNetwork = subscriptionNetwork(platform);
  const [developer, setDeveloper] = useDeveloperMode();
  const sections: { id: Section; label: string; advanced?: true }[] = [
    { id: "accounts", label: "Accounts" },
    { id: "general", label: "General" },
    { id: "sync", label: "Sync" },
    { id: "notifications", label: "Notifications" },
    ...(!mobile
      ? [{ id: "calendar" as const, label: "Academic calendar" }]
      : []),
    { id: "subscriptions", label: "Subscriptions" },
    { id: "hidden", label: "Hidden items" },
    ...(desktop ? [{ id: "files" as const, label: "Files" }] : []),
    { id: "data", label: "Data", advanced: true },
    { id: "scheduled", label: "Scheduled tasks", advanced: true },
    ...(!mobile
      ? [
          {
            id: "agent" as const,
            label: "Agent access",
            advanced: true as const,
          },
        ]
      : []),
    ...(extensionsAvailable
      ? [
          {
            id: "extensions" as const,
            label: "Extensions",
            advanced: true as const,
          },
        ]
      : []),
    ...(!mobile
      ? [{ id: "configs" as const, label: "Configs", advanced: true as const }]
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
    const clientId = resolveCloudClientId(
      data.settings.cloud.provider,
      data.settings.cloud.clientId,
    );
    const bridge = desktopBridge;
    const desktopOAuth = bridge
      ? {
          oauth: {
            connect: () =>
              bridge
                .connectCloud(
                  data.settings.cloud.provider as "google-drive" | "onedrive",
                  clientId,
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
                bodyBase64: binary ? bytesToBase64(binary) : undefined,
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
    return createCloudAdapter(data.settings.cloud.provider, clientId, {
      redirectUri: desktopBridge
        ? data.settings.cloud.redirectUri
        : `${location.origin}${location.pathname}`,
      readOnly: !desktopBridge,
      deviceId: data.deviceId,
      ...desktopOAuth,
    });
  };
  const markCloud = (
    state: Workspace["syncStatus"]["state"],
    message: string,
    synced = false,
  ) =>
    update((d) => {
      d.syncStatus.state = state;
      d.syncStatus.message = message;
      if (synced) d.syncStatus.lastSyncedAt = new Date().toISOString();
      return d;
    });
  const providerName = (provider: ActiveCloudProvider) =>
    provider === "google-drive" ? "Google Drive" : "OneDrive";
  const selectCloudProvider = async (
    next: Workspace["settings"]["cloud"]["provider"],
  ) => {
    if (next === data.settings.cloud.provider) return;
    const providersToDisconnect = authorizedProviders.filter(
      (provider) => provider !== next,
    );
    if (
      providersToDisconnect.length > 0 &&
      !confirm(
        `Switch cloud provider? This will sign out ${providersToDisconnect
          .map(providerName)
          .join(" and ")} on this device.`,
      )
    )
      return;
    try {
      setCloudBusy(true);
      const bridge = desktopBridge;
      if (bridge)
        await Promise.all(
          providersToDisconnect.map((provider) =>
            bridge.disconnectCloud(provider),
          ),
        );
      const remaining = authorizedProviders.filter(
        (provider) => !providersToDisconnect.includes(provider),
      );
      setAuthorizedProviders(remaining);
      update((d) => {
        d.settings.cloud.provider = next;
        d.syncStatus.state =
          next !== "none" && remaining.includes(next)
            ? "idle"
            : "needs-configuration";
        d.syncStatus.lastSyncedAt = undefined;
        d.syncStatus.message =
          next === "none"
            ? "Cloud sync is off."
            : remaining.includes(next)
              ? `Connected to ${providerName(next)}.`
              : `Authorize ${providerName(next)} to begin.`;
        return d;
      });
    } catch (error) {
      notify(
        error instanceof Error ? error.message : "Cloud provider switch failed",
      );
    } finally {
      setCloudBusy(false);
    }
  };
  const saveCloud = async () => {
    try {
      setCloudBusy(true);
      const a = await cloudAdapter();
      await a.connect();
      const provider = data.settings.cloud.provider as ActiveCloudProvider;
      setAuthorizedProviders([provider]);
      markCloud("idle", `Connected to ${providerName(provider)}. No sync yet.`);
      notify("Cloud authorized. Nothing uploaded yet.");
    } catch (e) {
      markCloud("error", e instanceof Error ? e.message : "Cloud setup failed");
      notify(e instanceof Error ? e.message : "Cloud setup failed");
    } finally {
      setCloudBusy(false);
    }
  };
  const pullCloud = async () => {
    try {
      setCloudBusy(true);
      const adapter = await cloudAdapter();
      const remote = await adapter.pull();
      const choice = confirm(
        "Replace this device's workspace with the latest cloud backup?",
      );
      if (!choice) return notify("Kept the current workspace.");
      await workspaceActions.importData(remote);
      markCloud("idle", "Restored the latest cloud workspace.", true);
      notify("Using the latest cloud backup.");
    } catch (error) {
      markCloud(
        "error",
        error instanceof Error ? error.message : "Cloud restore failed",
      );
      notify(error instanceof Error ? error.message : "Cloud restore failed");
    } finally {
      setCloudBusy(false);
    }
  };
  const pushCloud = async () => {
    try {
      setCloudBusy(true);
      const adapter = await cloudAdapter();
      try {
        const remote = await adapter.pull();
        const current = await workspaceActions.exportData();
        const remoteTables = JSON.stringify(JSON.parse(remote).tables);
        const currentTables = JSON.stringify(JSON.parse(current).tables);
        if (remoteTables !== currentTables) {
          pendingCloud.current = adapter;
          setCloudConflict(remote);
          return;
        }
      } catch (error) {
        if (!(error instanceof CloudTransportError) || error.status !== 404)
          throw error;
      }
      await performAction("push-cloud", detectPlatform(), () =>
        workspaceActions
          .exportData()
          .then((document) => adapter.push(document)),
      );
      markCloud("idle", "Cloud workspace is up to date.", true);
      notify("Cloud workspace uploaded.");
    } catch (error) {
      markCloud(
        "error",
        error instanceof Error ? error.message : "Cloud backup failed",
      );
      notify(error instanceof Error ? error.message : "Cloud backup failed");
    } finally {
      setCloudBusy(false);
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
        "IsctCompanion",
        "Local reminders are enabled.",
      );
  };
  const refreshSchool = async () => {
    if (!desktopBridge || caps["school-web-session"].support !== "supported")
      return notify(caps["school-web-session"].reason);
    if (!desktopBridge.reloadSchoolWeb)
      return notify("Open LMS or 教務 Web once before refreshing records.");
    const [lms, kyomu] = await Promise.all([
      desktopBridge.reloadSchoolWeb("science-lms"),
      desktopBridge.reloadSchoolWeb("kyomu"),
    ]);
    if (!lms && !kyomu)
      return notify("Open LMS or 教務 Web once before refreshing records.");
    notify("Refreshing signed-in LMS and 教務 records.");
    window.setTimeout(() => void workspaceStore.reload(), 2_000);
  };
  const exportData = async () => {
    const blob = new Blob([await workspaceActions.exportData()], {
      type: "application/json",
    });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = "IsctCompanion.json";
    a.click();
    URL.revokeObjectURL(a.href);
  };
  const resetData = async () => {
    if (
      !confirm(
        "Reset to an empty workspace? Everything on this device is replaced.",
      )
    )
      return;
    await workspaceActions.reset();
    notify("Workspace reset");
  };
  const importData = async (file: File) => {
    try {
      await workspaceActions.importData(await file.text());
      notify("Import completed.");
    } catch (error) {
      notify(error instanceof Error ? error.message : "Import failed.");
    }
  };
  const customizeSite = (fn: (d: Workspace) => Workspace) => {
    void performAction("customize-school-site", platform, () =>
      update(fn),
    ).catch((error) =>
      notify(error instanceof Error ? error.message : "Action denied"),
    );
  };
  return (
    <div className="page settings">
      <nav className="settingsNav" aria-label="Settings sections">
        {sections.map((s, index) => (
          <Fragment key={s.id}>
            {s.advanced && !sections[index - 1]?.advanced && (
              <span className="settingsNavGroup">{t("Advanced")}</span>
            )}
            <UiButton
              className={`${section === s.id ? "active" : ""}${s.advanced ? " nested" : ""}`}
              onClick={() => setSection(s.id)}
            >
              {t(s.label)}
            </UiButton>
          </Fragment>
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
            <Row title="Department">
              <UiSelect
                value={departmentById(data.settings.departmentId)?.id ?? ""}
                onChange={(event) =>
                  update((draft) => {
                    draft.settings.departmentId =
                      event.target.value || undefined;
                    return draft;
                  })
                }
              >
                <option value="">Not selected</option>
                {SYLLABUS_DEPARTMENTS.map((department) => (
                  <option key={department.id} value={department.id}>
                    {department.name[
                      data.settings.locale === "ja" ? "ja" : "en"
                    ] ?? department.id}
                  </option>
                ))}
              </UiSelect>
            </Row>
            <Row
              title="Admission year"
              hint="Selects the matching official guide_{year}; it does not follow the current year."
            >
              <UiInput
                type="number"
                min="2000"
                max="2200"
                value={data.academicProfile.admissionYear ?? ""}
                placeholder="2025"
                onChange={(event) =>
                  advancedCalendarUpdate((draft) => {
                    draft.academicProfile.admissionYear = event.target.value
                      ? Number(event.target.value)
                      : undefined;
                    return draft;
                  })
                }
              />
            </Row>
            <Row
              title={t("Allow subscription updates on cellular data")}
              hint={
                referenceNetwork === "cellular" &&
                !data.settings.subscriptionRefresh.allowCellular
                  ? t("Paused on cellular · checks a small hash daily")
                  : t("Checks a small hash daily · Wi-Fi preferred")
              }
            >
              <Toggle
                checked={data.settings.subscriptionRefresh.allowCellular}
                onChange={(enabled) =>
                  update((draft) => {
                    draft.settings.subscriptionRefresh.allowCellular = enabled;
                    return draft;
                  })
                }
              />
            </Row>
            <Row title={t("Background image URL")}>
              <UiInput
                value={data.settings.customization?.background ?? ""}
                placeholder="https://…"
                onChange={(event) =>
                  update((d) => {
                    if (d.settings.customization)
                      d.settings.customization.background = event.target.value;
                    return d;
                  })
                }
              />
            </Row>
            {desktop && (
              <Row title={t("Reopen last school page on launch")}>
                <Toggle
                  disabled={!desktopBridge}
                  checked={data.settings.restoreSchoolPage ?? false}
                  onChange={(enabled) => {
                    update((draft) => {
                      draft.settings.restoreSchoolPage = enabled;
                      if (!enabled) draft.settings.activeSchoolSite = undefined;
                      return draft;
                    });
                    void desktopBridge?.configureSchoolWebPolicy({
                      restoreAcrossRestarts: enabled,
                      idleMinutes: 10,
                    });
                  }}
                />
              </Row>
            )}
            <Row
              title={t("Shortcuts")}
              hint={t("⌘1–⌘4 switch pages · Esc closes panels")}
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
            <SchoolAccounts
              notify={notify}
              showLoginPage={showLoginPage}
              matrixOcrAvailable={matrixOcrAvailable}
            />
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
              {t("Cloud sync is separate from school accounts.")}
            </p>
            <div className="cloudNotice" role="note">
              <b>{t("Readable cloud copy")}</b>
              <span>
                {t(
                  "For convenience, IsctCompanion stores readable JSON without adding encryption. Your cloud account protects access.",
                )}
              </span>
            </div>
            <div className="cloudStatus" role="status">
              <span>
                <Badge
                  variant="light"
                  color={cloudConnected ? "green" : "gray"}
                >
                  {t(cloudConnected ? "Connected" : "Not connected")}
                </Badge>
                <b>
                  {selectedCloudProvider
                    ? providerName(selectedCloudProvider)
                    : t("Cloud sync off")}
                </b>
              </span>
              <span>
                <small>{t("Last sync")}</small>
                <b>
                  {data.syncStatus.lastSyncedAt
                    ? new Date(data.syncStatus.lastSyncedAt).toLocaleString(
                        data.settings.locale,
                      )
                    : t("Never")}
                </b>
              </span>
              {(selectedCloudProvider || data.syncStatus.message) && (
                <p>
                  {selectedCloudProvider
                    ? data.syncStatus.message
                    : t("Cloud sync is off.")}
                </p>
              )}
            </div>
            <Row
              title={t("Cloud provider")}
              hint={t("Private app folder, readable JSON")}
            >
              <UiSelect
                value={data.settings.cloud.provider}
                disabled={cloudBusy}
                onChange={(event) =>
                  void selectCloudProvider(
                    event.target
                      .value as Workspace["settings"]["cloud"]["provider"],
                  )
                }
              >
                <option value="none">{t("None")}</option>
                <option value="google-drive">Google Drive</option>
                <option value="onedrive">OneDrive</option>
              </UiSelect>
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
            <div className="actions end">
              <UiButton
                className="button"
                disabled={
                  !cloudAvailable || !selectedCloudProvider || cloudBusy
                }
                onClick={saveCloud}
              >
                {t(cloudConnected ? "Reauthorize" : "Authorize")}
              </UiButton>
              <UiButton
                className="button"
                disabled={!cloudAvailable || !cloudConnected || cloudBusy}
                onClick={pullCloud}
              >
                {t("Restore latest")}
              </UiButton>
              {desktopBridge && cloudAvailable && (
                <UiButton
                  className="button primary"
                  disabled={!cloudConnected || cloudBusy}
                  onClick={pushCloud}
                >
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
                      const document = await workspaceActions.exportData();
                      await pendingCloud.current?.push(document);
                      setCloudConflict(null);
                      markCloud("idle", "Cloud workspace is up to date.", true);
                      notify("Kept this device and uploaded it.");
                    }}
                  >
                    {t("Keep this device")}
                  </UiButton>
                  <UiButton
                    className="button"
                    onClick={async () => {
                      await workspaceActions.importData(cloudConflict);
                      setCloudConflict(null);
                      markCloud(
                        "idle",
                        "Restored the latest cloud workspace.",
                        true,
                      );
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
            <Row title={t("System notifications")}>
              {caps["local-notifications"].readiness === "ready" ? (
                <span className="rowValue">{t("Allowed")}</span>
              ) : caps["local-notifications"].support === "supported" ||
                platform === "browser-preview" ? (
                <UiButton
                  className="button small"
                  onClick={requestNotifications}
                >
                  {t("Allow")}
                </UiButton>
              ) : (
                <span className="rowValue">{t("Unavailable")}</span>
              )}
            </Row>
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
          </>
        ) : section === "calendar" ? (
          <CalendarManager
            data={data}
            update={advancedCalendarUpdate}
            notify={notify}
            readOnly={platform !== "native-desktop"}
          />
        ) : section === "subscriptions" ? (
          <ReferenceSubscriptions
            data={data}
            update={advancedCalendarUpdate}
            available={platform === "native-desktop" && !!desktopBridge}
            notify={notify}
          />
        ) : section === "hidden" ? (
          <HiddenItems data={data} update={update} />
        ) : section === "scheduled" ? (
          <>
            <h2>{t("Scheduled tasks")}</h2>
            {!data.scheduledTasks.some((task) => !task.deletedAt) && (
              <p className="paneEmpty">{t("No scheduled tasks")}</p>
            )}
            {data.scheduledTasks
              .filter((task) => !task.deletedAt)
              .map((task) => (
                <Row
                  key={task.id}
                  title={task.name}
                  hint={
                    task.lastRunAt
                      ? `${t("Last run")} ${task.lastRunAt.slice(0, 10)} · ${task.lastResult ?? task.schedule}`
                      : (task.lastResult ??
                        (task.action === "reference-refresh"
                          ? t("Daily hash check · Wi-Fi preferred")
                          : task.action === "lms-refresh"
                            ? t("Every 4 hours · Wi-Fi and active app only")
                            : task.schedule))
                  }
                >
                  <div className="actions">
                    <Toggle
                      checked={task.enabled}
                      onChange={(enabled) =>
                        update((draft) => {
                          const target = draft.scheduledTasks.find(
                            (item) => item.id === task.id,
                          );
                          if (target) {
                            target.enabled = enabled;
                            target.rev += 1;
                            target.updatedAt = new Date().toISOString();
                          }
                          return draft;
                        })
                      }
                    />
                    <UiButton
                      className="button small"
                      onClick={async () => {
                        let result = "Completed";
                        try {
                          if (runScheduledTask)
                            result = await runScheduledTask(task);
                          else if (task.action === "school-refresh")
                            await refreshSchool();
                          else if (task.action === "export") {
                            await exportData();
                            result = "Export completed";
                          } else if (task.action === "reference-refresh") {
                            result = "Desktop app required";
                          } else if (task.action === "lms-refresh") {
                            result = "Desktop app required";
                          } else result = "Hook queued";
                        } catch (error) {
                          result =
                            error instanceof Error
                              ? error.message
                              : "Run failed";
                        }
                        update((draft) => {
                          const target = draft.scheduledTasks.find(
                            (item) => item.id === task.id,
                          );
                          if (target) {
                            target.lastRunAt = new Date().toISOString();
                            target.lastResult = result;
                            target.rev += 1;
                            target.updatedAt = target.lastRunAt;
                          }
                          return draft;
                        });
                      }}
                    >
                      {t("Run now")}
                    </UiButton>
                  </div>
                </Row>
              ))}
          </>
        ) : section === "data" ? (
          <>
            <h2>{t("Data")}</h2>
            <div className="row">
              <span>
                <b>{t("Backup")}</b>
              </span>
              <div className="actions">
                <UiButton className="button" onClick={exportData}>
                  {t("Export JSON")}
                </UiButton>
                <label className="button">
                  {t("Import JSON")}
                  <input
                    hidden
                    type="file"
                    accept="application/json,.json"
                    onChange={(event) => {
                      const file = event.target.files?.[0];
                      if (file) void importData(file);
                    }}
                  />
                </label>
                <UiButton className="button danger" onClick={resetData}>
                  {t("Reset")}
                </UiButton>
              </div>
            </div>
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
                  <span className="muted">{t("No activity yet")}</span>
                )}
              </div>
            </details>
          </>
        ) : section === "agent" ? (
          <>
            <h2>{t("Agent access")}</h2>
            <Row
              title={t("Local MCP server")}
              hint={t("Reads school data; edits only plans and hidden items")}
            >
              <Toggle
                checked={data.settings.agentAccess?.enabled === true}
                onChange={(enabled) =>
                  update((draft) => {
                    draft.settings.agentAccess ??= {
                      enabled: false,
                      lastWrites: [],
                    };
                    draft.settings.agentAccess.enabled = enabled;
                    return draft;
                  })
                }
              />
            </Row>
            <div className="commandLine">
              <code>{agentCommand}</code>
              <UiButton
                className="button small"
                onClick={() =>
                  void navigator.clipboard
                    .writeText(agentCommand)
                    .then(() => notify(t("Copied")))
                    .catch(() => notify(t("Copy failed")))
                }
              >
                {t("Copy")}
              </UiButton>
            </div>
          </>
        ) : section === "files" ? (
          <FilesSettings data={data} update={update} />
        ) : section === "extensions" && extensionsAvailable ? (
          <ExtensionsSettings data={data} update={customizeSite} />
        ) : section === "configs" ? (
          <ConfigsSettings
            data={data}
            update={update}
            calendarUpdate={advancedCalendarUpdate}
            developer={developer}
            setDeveloper={setDeveloper}
            notify={notify}
          />
        ) : null}
      </section>
    </div>
  );
}
