import { useEffect, useState } from "react";
import { Pagination } from "../../components/ui";
import { t, type MessageKey } from "../../i18n";
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
import { UiButton, UiInput } from "../../components/ui";
import { confirmAction } from "../../components/confirm";
import {
  Row,
  SettingsGroup,
  SettingsHeader,
  Toggle,
} from "../../components/forms";
import { formatDay } from "./subscription-status";
import { ResetSettings } from "./ResetSettings";

const providerName: Record<CloudSyncProvider, string> = {
  icloud: "iCloud",
  "google-drive": "Google Drive",
};

/**
 * Sync runs on its own (slotledger approach/cloud-upload-queue): this page only chooses the
 * services and offers confirmed cloud versions and local recovery points.
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
  const [sync, setSync] = useState<CloudSyncState | null>(null);
  const [syncError, setSyncError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [versionPage, setVersionPage] = useState(1);
  const [recoveryPage, setRecoveryPage] = useState(1);
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
          setSyncError(
            errorText(error) ?? t("settings.backup.syncIsUnavailable"),
          );
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
  const run = async (failure: MessageKey, action: () => Promise<void>) => {
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
    run("settings.backup.couldNotChangeSync", async () => {
      const next = await bridge?.setCloudSyncProvider?.(provider, enabled);
      if (next) setSync(next);
    });
  const setAutomatic = (automatic: boolean) =>
    run("settings.backup.couldNotChangeSync", async () => {
      const next = await bridge?.setCloudSyncAutomatic?.(automatic);
      if (next) setSync(next);
    });
  const syncNow = (direction: "upload" | "pull") =>
    run("settings.backup.couldNotSync", async () => {
      const next = await bridge?.syncCloudNow?.(direction);
      if (next) setSync(next);
    });
  const restoreVersion = (version: Pick<CloudSyncVersion, "id" | "savedAt">) =>
    run("settings.backup.couldNotRestore", async () => {
      if (
        !(await confirmAction(
          t("settings.backup.restoreTheVersionFromDate", {
            date: when(version.savedAt),
          }),
          t("common.restore"),
        ))
      )
        return;
      const next = await bridge?.restoreCloudVersion?.(version.id);
      if (next) setSync(next);
      await workspaceActions.refreshSchool();
      notify(t("settings.backup.restoredTheEarlierVersion"));
    });

  const exportWorkspace = () =>
    saveExportFile("IsctCompanion.json", workspaceActions.exportData, notify);
  const importFile = async (file: File) => {
    if (
      !(await confirmAction(
        t("settings.backup.restoreThisFileOnAll"),
        t("common.restore"),
      ))
    )
      return;
    try {
      await workspaceActions.importData(await file.text());
      notify(t("settings.backup.backupRestored"));
    } catch (error) {
      notify(
        t("settings.backup.importFailedReason", {
          reason: errorText(error) ?? t("common.unknownError"),
        }),
      );
    }
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
  versions.sort((a, b) => Date.parse(b.savedAt) - Date.parse(a.savedAt));
  const recoveryPoints = [...(sync?.recoveryPoints ?? [])].sort(
    (a, b) => Date.parse(b.savedAt) - Date.parse(a.savedAt),
  );
  const versionPages = Math.max(1, Math.ceil(versions.length / 4));
  const recoveryPages = Math.max(1, Math.ceil(recoveryPoints.length / 4));
  const shownVersionPage = Math.min(versionPage, versionPages);
  const shownRecoveryPage = Math.min(recoveryPage, recoveryPages);
  const needsAttention = enabled.some((item) => item.error);
  const lastSynced = enabled
    .map((item) => item.lastSyncedAt)
    .filter((value): value is string => Boolean(value))
    .sort()
    .at(-1);

  return (
    <>
      <SettingsHeader title="settings.main.backup" />
      <SettingsGroup title="settings.backup.sync">
        {(!syncAvailable || syncError) && (
          <Row
            id="cloud"
            group
            title="settings.backup.cloudSync"
            hint={syncError ?? t(caps["cloud-sync"].reason)}
          >
            <span />
          </Row>
        )}
        {syncAvailable && (
          <Row
            id="sync-automatic"
            title="settings.backup.syncAutomatically"
            hint={
              sync?.automatic
                ? t("settings.backup.pullsOtherDevicesChangesWhen")
                : t("settings.backup.uploadAndPullByHand")
            }
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
            titleText={providerName[item.provider]}
            hint={
              !item.enabled ? undefined : item.error ? (
                <span>
                  <span className="danger">
                    {t("settings.backup.couldNotUpdateThisCloud")}
                  </span>
                  <br />
                  {/* The provider's own reason (signed out, another workspace,
                      offline): without it every failure looks the same. */}
                  {item.error}
                  {item.lastSyncedAt && (
                    <>
                      <br />
                      {t("settings.backup.lastSyncedDate", {
                        date: when(item.lastSyncedAt),
                      })}
                    </>
                  )}
                </span>
              ) : item.lastSyncedAt ? (
                t("settings.backup.syncedDate", {
                  date: when(item.lastSyncedAt),
                })
              ) : (
                t("settings.backup.waitingForTheFirstSync")
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
                ? "settings.backup.syncing"
                : needsAttention
                  ? "settings.backup.syncNeedsAttention"
                  : upToDate
                    ? "syncStatus.upToDate"
                    : "settings.backup.savedOnThisDevice"
            }
            hint={
              lastSynced
                ? t("settings.backup.lastSyncedDate", {
                    date: when(lastSynced),
                  })
                : undefined
            }
          >
            <div className="actions">
              <UiButton
                className="textButton"
                disabled={busy || sync?.syncing}
                onClick={() => void syncNow("upload")}
              >
                {t("settings.backup.upload")}
              </UiButton>
              <UiButton
                className="textButton"
                disabled={busy || sync?.syncing}
                onClick={() => void syncNow("pull")}
              >
                {t("settings.backup.pull")}
              </UiButton>
            </div>
          </Row>
        )}
      </SettingsGroup>
      {versions.length > 0 && (
        <SettingsGroup
          title="settings.backup.earlierVersions"
          footer={t("settings.backup.backupsOlderThan30Days")}
        >
          {versions
            .slice((shownVersionPage - 1) * 4, shownVersionPage * 4)
            .map((version) => (
              <Row
                key={version.id}
                group
                titleText={when(version.savedAt)}
                hint={`${version.device} · ${(sync?.versions ?? [])
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
                    {t("common.restore")}
                  </UiButton>
                </div>
              </Row>
            ))}
          {versionPages > 1 && (
            <Pagination
              size="sm"
              value={shownVersionPage}
              onChange={setVersionPage}
              total={versionPages}
              aria-label={t("settings.backup.earlierVersionPages")}
            />
          )}
        </SettingsGroup>
      )}
      {(sync?.recoveryPoints?.length ?? 0) > 0 && (
        <SettingsGroup
          title="settings.backup.localRecoveryFiles"
          footer={t("settings.backup.savedBeforeCloudChangesOr")}
        >
          {recoveryPoints
            .slice((shownRecoveryPage - 1) * 4, shownRecoveryPage * 4)
            .map((point) => (
              <Row key={point.id} group titleText={when(point.savedAt)}>
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
                    {t("settings.backup.export")}
                  </UiButton>
                  <UiButton
                    className="textButton"
                    disabled={busy || sync?.syncing || pending.length > 0}
                    onClick={() => void restoreVersion(point)}
                  >
                    {t("common.restore")}
                  </UiButton>
                </div>
              </Row>
            ))}
          {recoveryPages > 1 && (
            <Pagination
              size="sm"
              value={shownRecoveryPage}
              onChange={setRecoveryPage}
              total={recoveryPages}
              aria-label={t("settings.backup.localRecoveryPages")}
            />
          )}
        </SettingsGroup>
      )}
      {(icloud || drive) && (
        <SettingsGroup title="settings.backup.loginCredentials">
          {icloud && (
            <Row
              id="credential-keychain"
              group
              title="settings.backup.icloudKeychain"
            >
              <small className="muted">
                {t("settings.backup.schoolLoginsSyncWithICloud")}
              </small>
            </Row>
          )}
          {drive && <DriveCredentialSync notify={notify} />}
        </SettingsGroup>
      )}
      <SettingsGroup>
        <Row id="file" group title="settings.backup.backupFile">
          <div className="actions">
            <UiButton
              className="button small"
              onClick={() => void exportWorkspace()}
            >
              {t("settings.backup.export")}
            </UiButton>
            <label className="button small">
              {t("settings.backup.import")}
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
      <ResetSettings
        locale={locale}
        cloudProviders={enabled.map((item) => item.provider)}
        notify={notify}
      />
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
  const run = async (failure: MessageKey, action: () => Promise<void>) => {
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
    run("settings.backup.couldNotEnableCredentialSync", async () => {
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
      notify(t("settings.backup.loginCredentialsHaveBeenSynced"));
    });
  const turnOff = () =>
    run("settings.backup.couldNotDisableCredentialSync", async () => {
      await desktopBridge!.clearCloudCredentialSyncPassword();
      reset();
      setConfigured(false);
      // Only this device's key is removed; the Drive file stays unlockable.
      setBackupExists(true);
      notify(t("settings.backup.loginCredentialSyncIsDisabled"));
    });

  const title: MessageKey = configured
    ? "settings.backup.loginCredentialSync"
    : restoring
      ? "settings.backup.restoreLoginCredentials"
      : "settings.backup.loginCredentialSync";
  const hint: MessageKey =
    backupExists === null && !configured
      ? "settings.backup.checkingGoogleDrive"
      : configured
        ? editing
          ? "settings.backup.chooseANew812"
          : "settings.extensions.on"
        : restoring
          ? "settings.backup.enterThePasswordSetOn"
          : editing
            ? "settings.backup.chooseAn812Character"
            : "settings.main.off";
  const valid =
    length >= 8 && length <= 12 && (restoring || (confirmPassword && matches));

  return (
    <>
      <Row id="credential-sync" group title={title} hint={t(hint)}>
        <div className="actions">
          {showFields && (
            <>
              <UiInput
                type="password"
                value={password}
                minLength={8}
                autoComplete={restoring ? "current-password" : "new-password"}
                placeholder={t(
                  restoring
                    ? "settings.backup.encryptionPassword"
                    : "settings.backup.newPassword",
                )}
                aria-label={t("settings.backup.credentialEncryptionPassword")}
                disabled={busy}
                onChange={(event) => setPassword(event.currentTarget.value)}
              />
              {!restoring && (
                <UiInput
                  type="password"
                  value={confirmPassword}
                  minLength={8}
                  autoComplete="new-password"
                  placeholder={t("settings.backup.confirmPassword")}
                  aria-label={t(
                    "settings.backup.confirmCredentialEncryptionPassword",
                  )}
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
                {t(
                  restoring
                    ? "common.restore"
                    : configured
                      ? "common.save"
                      : "settings.backup.turnOn",
                )}
              </UiButton>
              {editing && (
                <UiButton
                  className="textButton"
                  disabled={busy}
                  onClick={reset}
                >
                  {t("common.cancel")}
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
              {t("settings.backup.setUp")}
            </UiButton>
          )}
          {!showFields && configured && (
            <>
              <UiButton
                className="textButton"
                disabled={busy}
                onClick={() => setEditing(true)}
              >
                {t("settings.backup.changePassword")}
              </UiButton>
              <UiButton
                className="textButton"
                disabled={busy}
                onClick={() => void turnOff()}
              >
                {t("settings.backup.turnOff")}
              </UiButton>
            </>
          )}
        </div>
      </Row>
      {showFields && !restoring && confirmPassword && !matches && (
        <div className="alert" role="alert">
          {t("settings.backup.theTwoPasswordsDoNot")}
        </div>
      )}
      {showFields && (
        <div className="alert" role="note">
          {t("settings.backup.encryptedWithThisPasswordBefore")}
        </div>
      )}
    </>
  );
}
