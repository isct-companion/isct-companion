import { useEffect, useState } from "react";
import { t } from "../../i18n";
import type { Workspace } from "../../lib/model";
import { errorText } from "../../app/shared";
import { desktopBridge, saveExportFile } from "../../app/native";
import { workspaceActions } from "../../store/workspace";
import { resolveCapabilities } from "../../platform/capabilities";
import type {
  CloudSyncProvider,
  CloudSyncState,
  CloudSyncVersion,
} from "../../platform/desktop-bridge";
import { UiButton, UiInput } from "../../components/mantine";
import { confirmAction } from "../../components/confirm";
import {
  Row,
  SettingsGroup,
  SettingsHeader,
  Toggle,
} from "../../components/forms";
import { formatDay } from "./subscription-status";
import { useDeveloperMode } from "./DeveloperSettings";

const providerName: Record<CloudSyncProvider, string> = {
  icloud: "iCloud",
  "google-drive": "Google Drive",
};

/**
 * Sync runs on its own (docs/cloud-sync.md): this page only chooses the
 * services, shows queued operations, and offers cloud and local recovery points.
 */
export function BackupSettings({
  data,
  caps,
  notify,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
  caps: ReturnType<typeof resolveCapabilities>;
  notify: (message: string) => void;
}) {
  const locale = data.settings.locale;
  // Version hashes and task ids help debugging, not everyday backup.
  const [developer] = useDeveloperMode();
  const [sync, setSync] = useState<CloudSyncState | null>(null);
  const [syncError, setSyncError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const bridge = desktopBridge;
  const syncAvailable =
    caps["cloud-sync"].support === "supported" &&
    Boolean(bridge?.cloudSyncState);

  useEffect(() => {
    if (!syncAvailable || !bridge?.cloudSyncState) return;
    let disposed = false;
    void bridge
      .cloudSyncState()
      .then((state) => {
        if (!disposed) setSync(state);
      })
      .catch((error) => {
        if (!disposed)
          setSyncError(errorText(error) ?? t("Sync is unavailable."));
      });
    const stop = bridge.onCloudSyncState?.((state) => {
      if (!disposed) setSync(state);
    });
    return () => {
      disposed = true;
      stop?.();
    };
  }, [syncAvailable, bridge]);

  const when = (iso: string) =>
    formatDay(iso, locale, { hour: "2-digit", minute: "2-digit" });
  const run = async (failure: string, action: () => Promise<void>) => {
    try {
      setBusy(true);
      await action();
    } catch (error) {
      notify(errorText(error) ?? t(failure));
    } finally {
      setBusy(false);
    }
  };
  const setProvider = (provider: CloudSyncProvider, enabled: boolean) =>
    run("Could not change sync.", async () => {
      const next = await bridge?.setCloudSyncProvider?.(provider, enabled);
      if (next) setSync(next);
    });
  const setAutomatic = (automatic: boolean) =>
    run("Could not change sync.", async () => {
      const next = await bridge?.setCloudSyncAutomatic?.(automatic);
      if (next) setSync(next);
    });
  const syncNow = (direction: "upload" | "pull") =>
    run("Could not sync.", async () => {
      const next = await bridge?.syncCloudNow?.(direction);
      if (next) setSync(next);
    });
  const restoreVersion = (version: Pick<CloudSyncVersion, "id" | "savedAt">) =>
    run("Could not restore.", async () => {
      if (
        !(await confirmAction(
          t(
            "Restore the version from {{date}} on all connected devices? Unsynced work on older devices will be kept in recovery files.",
            {
              date: when(version.savedAt),
            },
          ),
          t("Restore"),
        ))
      )
        return;
      const next = await bridge?.restoreCloudVersion?.(version.id);
      if (next) setSync(next);
      await workspaceActions.refreshSchool();
      notify(t("Restored the earlier version."));
    });

  const exportWorkspace = () =>
    saveExportFile("IsctCompanion.json", workspaceActions.exportData, notify);
  const importFile = async (file: File) => {
    if (
      !(await confirmAction(
        t(
          "Restore this file on all connected devices? A recovery copy of current data will be kept.",
        ),
        t("Restore"),
      ))
    )
      return;
    try {
      await workspaceActions.importData(await file.text());
      notify(t("Backup restored."));
    } catch (error) {
      notify(
        t("Import failed: {{reason}}", {
          reason: errorText(error) ?? t("unknown error"),
        }),
      );
    }
  };
  const erase = async () => {
    if (
      !(await confirmAction(
        t(
          "Erase all data on this device and start over? This cannot be undone.",
        ),
        t("Erase"),
      ))
    )
      return;
    await workspaceActions.reset();
    notify(t("This device was reset."));
  };

  const providers = sync?.providers ?? [];
  const enabled = providers.filter((item) => item.enabled);
  // Keychain sync only helps once iCloud itself works on this device.
  const icloud = enabled.some(
    (item) => item.provider === "icloud" && !item.error,
  );
  const drive = enabled.some((item) => item.provider === "google-drive");
  const pending =
    sync?.tasks?.filter((task) =>
      task.deliveries.some((d) => d.status !== "completed"),
    ) ?? [];
  const upToDate =
    enabled.length > 0 &&
    pending.length === 0 &&
    Boolean(sync?.localVersion) &&
    enabled.every((p) => p.syncedVersion === sync?.localVersion);
  const versions = [
    ...new Map(
      (sync?.versions ?? []).map((version) => [
        version.version || version.id,
        version,
      ]),
    ).values(),
  ];
  const lastSynced = enabled
    .map((item) => item.lastSyncedAt)
    .filter((value): value is string => Boolean(value))
    .sort()
    .at(-1);

  return (
    <>
      <SettingsHeader title="Backup" />
      <SettingsGroup title="Sync">
        {(!syncAvailable || syncError) && (
          <Row
            id="cloud"
            group
            title="Cloud sync"
            hint={syncError ?? caps["cloud-sync"].reason}
          >
            <span />
          </Row>
        )}
        {syncAvailable && (
          <Row
            id="sync-automatic"
            title="Sync automatically"
            hint={sync?.automatic ? undefined : "Upload and pull by hand"}
          >
            <Toggle
              checked={sync?.automatic ?? false}
              disabled={busy || !sync}
              onChange={(value) => void setAutomatic(value)}
            />
          </Row>
        )}
        {providers.map((item) => (
          <Row
            key={item.provider}
            id={item.provider === "google-drive" ? "cloud" : "icloud"}
            title={providerName[item.provider]}
            hint={
              item.error ? (
                <span className="danger">{item.error}</span>
              ) : !item.enabled ? undefined : item.lastSyncedAt ? (
                t("Synced {{date}}", { date: when(item.lastSyncedAt) })
              ) : (
                t("Waiting for the first sync")
              )
            }
          >
            <Toggle
              checked={item.enabled}
              disabled={busy}
              onChange={(value) => void setProvider(item.provider, value)}
            />
          </Row>
        ))}
        {enabled.length > 0 && (
          <Row
            id="sync-now"
            group
            title={
              sync?.syncing
                ? "Syncing…"
                : pending.length > 0
                  ? "Waiting to sync"
                  : upToDate
                    ? "Up to date"
                    : "Saved on this device"
            }
            hint={
              lastSynced
                ? t("Last synced {{date}}", { date: when(lastSynced) })
                : undefined
            }
          >
            <div className="actions">
              <UiButton
                className="textButton"
                disabled={busy || sync?.syncing}
                onClick={() => void syncNow("upload")}
              >
                {t("Upload")}
              </UiButton>
              <UiButton
                className="textButton"
                disabled={busy || sync?.syncing}
                onClick={() => void syncNow("pull")}
              >
                {t("Pull")}
              </UiButton>
            </div>
          </Row>
        )}
        {developer && sync?.localVersion && (
          <p className="settingsNote">
            {t("Local version")} · <code>{sync.localVersion.slice(0, 15)}</code>
          </p>
        )}
        {pending.map((task) => (
          <Row
            key={task.id}
            group
            title={task.direction === "upload" ? "Upload" : "Pull"}
            hint={
              developer
                ? `${when(task.createdAt)} · ${task.version.slice(0, 15)} · ${task.id}`
                : when(task.createdAt)
            }
          >
            <span className="syncDeliveries">
              {task.deliveries.map((d) => (
                <small key={d.provider}>
                  {providerName[d.provider]} ·{" "}
                  {t(
                    d.status === "completed"
                      ? "Completed"
                      : d.status === "running"
                        ? "Syncing…"
                        : d.attempts > 0
                          ? "Waiting to retry"
                          : "Queued",
                  )}{" "}
                  · {t("Attempts")}: {d.attempts}
                  {d.error ? ` · ${d.error}` : ""}
                </small>
              ))}
            </span>
          </Row>
        ))}
      </SettingsGroup>
      {versions.length > 0 && (
        <SettingsGroup title="Earlier versions">
          {versions.map((version) => (
            <Row
              key={version.id}
              group
              title={when(version.savedAt)}
              hint={`${version.device} · ${version.version?.slice(0, 15) ?? ""} · ${(
                sync?.versions ?? []
              )
                .filter(
                  (v) =>
                    (v.version || v.id) === (version.version || version.id),
                )
                .map((v) => providerName[v.provider])
                .join(" / ")}`}
            >
              <div className="actions">
                <UiButton
                  className="button small"
                  disabled={busy || sync?.syncing || pending.length > 0}
                  onClick={() => void restoreVersion(version)}
                >
                  {t("Restore")}
                </UiButton>
              </div>
            </Row>
          ))}
        </SettingsGroup>
      )}
      {(sync?.recoveryPoints?.length ?? 0) > 0 && (
        <SettingsGroup
          title="Local recovery files"
          footer={t(
            "Saved before cloud changes or a restore replace local content.",
          )}
        >
          {sync!.recoveryPoints!.map((point) => (
            <Row
              key={point.id}
              group
              title={when(point.savedAt)}
              hint={point.version.slice(0, 15)}
            >
              <div className="actions">
                <UiButton
                  className="textButton"
                  onClick={() =>
                    void saveExportFile(
                      `IsctCompanion-${point.version.slice(0, 15)}.json`,
                      () => bridge!.cloudRecoveryDocument!(point.id),
                      notify,
                    )
                  }
                >
                  {t("Export…")}
                </UiButton>
                <UiButton
                  className="textButton"
                  disabled={busy || sync?.syncing || pending.length > 0}
                  onClick={() => void restoreVersion(point)}
                >
                  {t("Restore")}
                </UiButton>
              </div>
            </Row>
          ))}
        </SettingsGroup>
      )}
      {(icloud || drive) && (
        <SettingsGroup title="Login credentials">
          {icloud && (
            <Row id="credential-keychain" group title="iCloud Keychain">
              <small className="muted">
                {t("School logins sync with iCloud Keychain.")}
              </small>
            </Row>
          )}
          {drive && <DriveCredentialSync notify={notify} />}
        </SettingsGroup>
      )}
      <SettingsGroup>
        <Row id="file" group title="Backup file">
          <div className="actions">
            <UiButton
              className="button small"
              onClick={() => void exportWorkspace()}
            >
              {t("Export…")}
            </UiButton>
            <label className="button small">
              {t("Import…")}
              <input
                hidden
                type="file"
                accept="application/json,.json"
                onChange={(event) => {
                  const file = event.target.files?.[0];
                  event.target.value = "";
                  if (file) void importFile(file);
                }}
              />
            </label>
          </div>
        </Row>
      </SettingsGroup>
      <SettingsGroup tone="danger">
        <Row
          id="reset"
          group
          title="Erase data on this device"
          hint="Can't be undone."
        >
          <div className="actions">
            <UiButton className="button small danger" onClick={erase}>
              {t("Erase…")}
            </UiButton>
          </div>
        </Row>
      </SettingsGroup>
    </>
  );
}

/** School logins in an encrypted Drive file, unlocked by an 8–12 character password. */
function DriveCredentialSync({
  notify,
}: {
  notify: (message: string) => void;
}) {
  const provider = "google-drive";
  const [configured, setConfigured] = useState(false);
  const [backupExists, setBackupExists] = useState<boolean | null>(null);
  // Password fields show only while setting up, changing or restoring.
  const [editing, setEditing] = useState(false);
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const length = Array.from(password).length;
  const matches = password === confirmPassword;
  const restoring = !configured && backupExists === true;
  const showFields = editing || restoring;

  useEffect(() => {
    const bridge = desktopBridge;
    if (!bridge) return;
    let disposed = false;
    void bridge
      .cloudCredentialSyncStatus()
      .then((value) => {
        if (!disposed) setConfigured(value);
      })
      .catch(() => undefined);
    void bridge
      .cloudCredentialBackupStatus(provider)
      .then((value) => {
        if (!disposed) setBackupExists(value);
      })
      .catch(() => {
        if (!disposed) setBackupExists(null);
      });
    return () => {
      disposed = true;
    };
  }, []);

  const reset = () => {
    setPassword("");
    setConfirmPassword("");
    setEditing(false);
  };
  const run = async (failure: string, action: () => Promise<void>) => {
    try {
      setBusy(true);
      await action();
    } catch (error) {
      notify(errorText(error) ?? t(failure));
    } finally {
      setBusy(false);
    }
  };
  const save = () =>
    run("Could not enable credential sync.", async () => {
      const bridge = desktopBridge!;
      const wasConfigured = configured;
      await bridge.setCloudCredentialSyncPassword(password);
      try {
        await bridge.syncCloudCredentials(provider, restoring);
      } catch (error) {
        if (!wasConfigured) await bridge.clearCloudCredentialSyncPassword();
        throw error;
      }
      reset();
      setConfigured(true);
      setBackupExists(true);
      notify(t("Login credentials have been synced."));
    });
  const turnOff = () =>
    run("Could not disable credential sync.", async () => {
      await desktopBridge!.clearCloudCredentialSyncPassword();
      reset();
      setConfigured(false);
      // Only this device's key is removed; the Drive file stays unlockable.
      setBackupExists(true);
      notify(t("Login credential sync is disabled on this device."));
    });

  const title = configured
    ? "Login credential sync"
    : restoring
      ? "Restore login credentials"
      : "Login credential sync";
  const hint =
    backupExists === null && !configured
      ? "Checking Google Drive…"
      : configured
        ? editing
          ? "Choose a new 8–12 character password"
          : "On"
        : restoring
          ? "Enter the password set on your other device"
          : editing
            ? "Choose an 8–12 character password"
            : "Off";
  const valid =
    length >= 8 && length <= 12 && (restoring || (confirmPassword && matches));

  return (
    <>
      <Row id="credential-sync" group title={title} hint={hint}>
        <div className="actions">
          {showFields && (
            <>
              <UiInput
                type="password"
                value={password}
                minLength={8}
                autoComplete={restoring ? "current-password" : "new-password"}
                placeholder={t(
                  restoring ? "Encryption password" : "New password",
                )}
                aria-label={t("Credential encryption password")}
                disabled={busy}
                onChange={(event) => setPassword(event.currentTarget.value)}
              />
              {!restoring && (
                <UiInput
                  type="password"
                  value={confirmPassword}
                  minLength={8}
                  autoComplete="new-password"
                  placeholder={t("Confirm password")}
                  aria-label={t("Confirm credential encryption password")}
                  disabled={busy}
                  onChange={(event) =>
                    setConfirmPassword(event.currentTarget.value)
                  }
                />
              )}
              <UiButton
                className="button small primary"
                disabled={busy || !valid}
                onClick={() => void save()}
              >
                {t(restoring ? "Restore" : configured ? "Save" : "Turn on")}
              </UiButton>
              {editing && (
                <UiButton
                  className="textButton"
                  disabled={busy}
                  onClick={reset}
                >
                  {t("Cancel")}
                </UiButton>
              )}
            </>
          )}
          {!showFields && backupExists !== null && !configured && (
            <UiButton
              className="button small"
              disabled={busy}
              onClick={() => setEditing(true)}
            >
              {t("Set up")}
            </UiButton>
          )}
          {!showFields && configured && (
            <>
              <UiButton
                className="textButton"
                disabled={busy}
                onClick={() => setEditing(true)}
              >
                {t("Change password")}
              </UiButton>
              <UiButton
                className="textButton"
                disabled={busy}
                onClick={() => void turnOff()}
              >
                {t("Turn off")}
              </UiButton>
            </>
          )}
        </div>
      </Row>
      {showFields && !restoring && confirmPassword && !matches && (
        <div className="alert" role="alert">
          {t("The two passwords do not match.")}
        </div>
      )}
      {showFields && (
        <div className="alert" role="note">
          {t(
            "Encrypted with this password before upload. The password stays on this device.",
          )}
        </div>
      )}
    </>
  );
}
