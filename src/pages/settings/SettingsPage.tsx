import { t } from "../../i18n";
import { SYLLABUS_DEPARTMENTS, departmentById } from "../../core/departments";
import { UiButton, UiInput, UiSelect } from "../../components/mantine";
import { AdvancedSettings } from "./AdvancedSettings";
import { SchoolAccounts } from "./SchoolAccounts";

import { useEffect, useRef, useState } from "react";
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
  type CloudAdapter,
} from "../../connectors/cloud";
import { createRecoveryCipher, type RecoveryKey } from "../../sync/crypto";

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
  const [recoveryPhrase, setRecoveryPhrase] = useState("");
  const [cloudConflict, setCloudConflict] = useState<string | null>(null);
  const [agentCommand, setAgentCommand] = useState("campus-mcp --db /absolute/path/to/school.db");
  useEffect(() => {
    if (!desktopBridge) return;
    void invoke<string>("agent_command").then(setAgentCommand).catch(() => undefined);
  }, []);
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
    { id: "subscriptions", label: "Subscriptions" },
    { id: "hidden", label: "Hidden items" },
    { id: "scheduled", label: "Scheduled tasks" },
    { id: "data", label: "Data" },
    { id: "agent", label: "Agent access" },
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
      await workspaceActions.importData(remote);
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
        workspaceActions.exportData().then((document) => adapter.push(document)),
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
    a.download = "science-tokyo-workspace.json";
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
                      const document = await workspaceActions.exportData();
                      await pendingCloud.current?.push(document);
                      setCloudConflict(null);
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
        ) : section === "subscriptions" ? (
          <>
            <h2>Subscriptions</h2>
            {data.subscriptions.map((subscription) => (
              <Row
                key={subscription.id}
                title={subscription.kind}
                hint={
                  subscription.lastError ??
                  (subscription.fetchedAt
                    ? `Updated ${new Date(subscription.fetchedAt).toLocaleString()}`
                    : subscription.url || "Bundled 2026 data")
                }
              >
                <Toggle
                  checked={subscription.enabled}
                  onChange={(enabled) =>
                    update((draft) => {
                      const target = draft.subscriptions.find(
                        (item) => item.id === subscription.id,
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
              </Row>
            ))}
            {!data.subscriptions.length && (
              <p className="muted">
                No reference data has been applied for this profile.
              </p>
            )}
            <div className="actions end">
              <UiButton
                className="button primary"
                disabled={
                  !desktopBridge ||
                  data.academicProfile.admissionYear === undefined
                }
                onClick={async () => {
                  const academicYear = Number(data.selectedTerm.slice(0, 4));
                  const admissionYear = data.academicProfile.admissionYear;
                  if (!admissionYear)
                    return notify("Choose your admission year in General first.");
                  try {
                    const changed = await invoke<number>(
                      "apply_bundled_subscriptions",
                      { academicYear, admissionYear },
                    );
                    await workspaceStore.reload();
                    notify(`Verified reference data updated (${changed} changes).`);
                  } catch (error) {
                    notify(
                      error instanceof Error
                        ? error.message
                        : String(error),
                    );
                  }
                }}
              >
                Update verified data
              </UiButton>
            </div>
          </>
        ) : section === "hidden" ? (
          <>
            <h2>Hidden items</h2>
            {data.assignments
              .filter((assignment) => assignment.hidden && !assignment.deletedAt)
              .map((assignment) => (
                <Row
                  key={assignment.id}
                  title={assignment.title}
                  hint={new Date(assignment.dueAt).toLocaleString()}
                >
                  <UiButton
                    className="button"
                    onClick={() =>
                      update((draft) => {
                        const target = draft.assignments.find(
                          (item) => item.id === assignment.id,
                        );
                        if (target) {
                          target.hidden = false;
                          target.rev += 1;
                          target.updatedAt = new Date().toISOString();
                        }
                        return draft;
                      })
                    }
                  >
                    Restore
                  </UiButton>
                </Row>
              ))}
            <UiButton
              className="button"
              onClick={() =>
                update((draft) => {
                  for (const assignment of draft.assignments) {
                    if (assignment.hidden) {
                      assignment.hidden = false;
                      assignment.rev += 1;
                      assignment.updatedAt = new Date().toISOString();
                    }
                  }
                  return draft;
                })
              }
            >
              Clear all
            </UiButton>
          </>
        ) : section === "scheduled" ? (
          <>
            <h2>Scheduled tasks</h2>
            {data.scheduledTasks.map((task) => (
              <Row
                key={task.id}
                title={task.name}
                hint={
                  task.lastRunAt
                    ? `Last run ${task.lastRunAt.slice(0, 10)} · ${task.lastResult ?? task.schedule}`
                    : task.lastResult ?? task.schedule
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
                        else if (task.action === "school-refresh") await refreshSchool();
                        else if (task.action === "export") {
                          await exportData();
                          result = "Export completed";
                        } else result = "Hook queued";
                      } catch (error) {
                        result = error instanceof Error ? error.message : "Run failed";
                      }
                      update((draft) => {
                        const target = draft.scheduledTasks.find((item) => item.id === task.id);
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
                    Run now
                  </UiButton>
                </div>
              </Row>
            ))}
          </>
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
                <label className="button">
                  Import JSON
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
        ) : section === "agent" ? (
          <>
            <h2>Agent access</h2>
            <Row
              title="Local MCP server"
              hint="Read school data and write only personal plans or hidden-item state."
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
            <code>{agentCommand}</code>
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
