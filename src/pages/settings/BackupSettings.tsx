import { useEffect, useState } from "react";
import { t } from "../../i18n";
import type { Workspace } from "../../core/model";
import { errorText } from "../../app/shared";
import { desktopBridge, saveExportFile } from "../../app/native";
import { workspaceActions } from "../../store/workspace";
import { resolveCapabilities } from "../../platform/capabilities";
import {
  nativeOs,
  type CloudProvider,
  type CloudSyncResult,
} from "../../platform/desktop-bridge";
import { UiButton, UiInput } from "../../components/mantine";
import { Row, SettingsGroup, SettingsHeader } from "../../components/forms";
import { formatDay } from "./subscription-status";

export function BackupSettings({
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
  const locale = data.settings.locale;
  const [conflict, setConflict] = useState<CloudSyncResult | null>(null);
  const [authorized, setAuthorized] = useState<CloudProvider[]>([]);
  const [credentialSync, setCredentialSync] = useState(false);
  const [credentialBackupExists, setCredentialBackupExists] = useState<
    boolean | null
  >(null);
  const [encryptionPassword, setEncryptionPassword] = useState("");
  const [confirmEncryptionPassword, setConfirmEncryptionPassword] =
    useState("");
  const encryptionPasswordLength = Array.from(encryptionPassword).length;
  const passwordsMatch = encryptionPassword === confirmEncryptionPassword;
  const [busy, setBusy] = useState(false);
  const os = nativeOs();
  const appleDevice = os === "macos" || os === "ios";
  const [provider, setProvider] = useState<CloudProvider>(() =>
    data.settings.cloud.provider === "icloud" ? "icloud" : "google-drive",
  );
  useEffect(() => {
    if (!desktopBridge) return;
    let disposed = false;
    void desktopBridge
      .cloudAuthorizationStatus()
      .then((providers) => {
        if (!disposed) setAuthorized(providers);
      })
      .catch(() => undefined);
    void desktopBridge
      .cloudCredentialSyncStatus()
      .then((configured) => {
        if (!disposed) setCredentialSync(configured);
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
    };
  }, []);

  const exportWorkspace = () =>
    saveExportFile("IsctCompanion.json", workspaceActions.exportData, notify);

  const cloud = data.settings.cloud;
  const cloudAvailable = caps["cloud-sync"].support === "supported";
  const clientId = cloud.clientId.trim() || __GOOGLE_DRIVE_CLIENT_ID__.trim();
  const connected =
    cloud.provider === provider && authorized.includes(provider);
  const serviceName = provider === "icloud" ? "iCloud" : "Google Drive";

  useEffect(() => {
    if (!desktopBridge || !connected || provider === "icloud") {
      setCredentialBackupExists(null);
      return;
    }
    let disposed = false;
    setCredentialBackupExists(null);
    void desktopBridge
      .cloudCredentialBackupStatus(provider)
      .then((exists) => {
        if (!disposed) setCredentialBackupExists(exists);
      })
      .catch(() => {
        if (!disposed) setCredentialBackupExists(null);
      });
    return () => {
      disposed = true;
    };
  }, [connected, provider]);

  const requireCloud = () => {
    if (!cloudAvailable || !desktopBridge)
      throw new Error(caps["cloud-sync"].reason);
    return { bridge: desktopBridge, provider };
  };
  const mark = (
    state: Workspace["syncStatus"]["state"],
    message: string,
    synced = false,
  ) =>
    update((draft) => {
      draft.syncStatus.state = state;
      draft.syncStatus.message = message;
      if (synced) draft.syncStatus.lastSyncedAt = new Date().toISOString();
      return draft;
    });
  const run = async (failure: string, action: () => Promise<void>) => {
    try {
      setBusy(true);
      await action();
    } catch (error) {
      const message = errorText(error) ?? t(failure);
      mark("error", message);
      notify(message);
    } finally {
      setBusy(false);
    }
  };

  const disconnect = () =>
    run("Could not disconnect.", async () => {
      const bridge = desktopBridge;
      if (bridge)
        await Promise.all(
          authorized.map((service) => bridge.disconnectCloud(service)),
        );
      setAuthorized([]);
      setCredentialBackupExists(null);
      update((draft) => {
        draft.settings.cloud.provider = "none";
        draft.syncStatus.state = "needs-configuration";
        draft.syncStatus.lastSyncedAt = undefined;
        draft.syncStatus.message = undefined;
        return draft;
      });
    });
  const connect = () =>
    run("Could not connect.", async () => {
      const { bridge, provider } = requireCloud();
      await bridge.connectCloud(
        provider,
        provider === "google-drive" ? clientId : "",
        provider === "google-drive" ? cloud.redirectUri! : "",
      );
      setAuthorized([provider]);
      if (provider !== "icloud")
        setCredentialBackupExists(
          await bridge.cloudCredentialBackupStatus(provider),
        );
      update((draft) => {
        draft.settings.cloud.provider = provider;
        draft.syncStatus.state = "idle";
        draft.syncStatus.message = undefined;
        return draft;
      });
      notify(t("Connected to {{service}}.", { service: serviceName }));
    });
  const restore = () =>
    run("Could not restore.", async () => {
      if (!confirm(t("Replace the data on this device with the cloud backup?")))
        return;
      const { bridge, provider } = requireCloud();
      await bridge.restoreCloudWorkspace(provider);
      await workspaceActions.refreshSchool();
      mark("idle", t("Restored from the cloud backup."), true);
      notify(t("Restored from the cloud backup."));
    });
  const backUp = () =>
    run("Could not back up.", async () => {
      const { bridge, provider } = requireCloud();
      const result = await bridge.backupCloudWorkspace(provider, false);
      if (result.state === "conflict") {
        setConflict(result);
        return;
      }
      mark("idle", t("Backup is up to date."), true);
      notify(t("Backed up."));
    });
  const keepThisDevice = () =>
    run("Could not back up.", async () => {
      const { bridge, provider } = requireCloud();
      await bridge.backupCloudWorkspace(provider, true);
      setConflict(null);
      mark("idle", t("Backup is up to date."), true);
      notify(t("Kept this device and backed it up."));
    });
  const useCloud = () =>
    run("Could not restore.", async () => {
      const { bridge, provider } = requireCloud();
      await bridge.restoreCloudWorkspace(provider);
      await workspaceActions.refreshSchool();
      setConflict(null);
      mark("idle", t("Restored from the cloud backup."), true);
      notify(t("Restored from the cloud backup."));
    });
  const saveCredentialSync = () =>
    run("Could not enable credential sync.", async () => {
      const { bridge, provider } = requireCloud();
      const wasConfigured = credentialSync;
      const restoreExisting = credentialBackupExists === true && !wasConfigured;
      await bridge.setCloudCredentialSyncPassword(encryptionPassword);
      try {
        await bridge.syncCloudCredentials(provider, restoreExisting);
      } catch (error) {
        if (!wasConfigured) await bridge.clearCloudCredentialSyncPassword();
        throw error;
      }
      setEncryptionPassword("");
      setConfirmEncryptionPassword("");
      setCredentialSync(true);
      setCredentialBackupExists(true);
      notify(t("Login credentials have been synced."));
    });
  const disableCredentialSync = () =>
    run("Could not disable credential sync.", async () => {
      const { bridge } = requireCloud();
      await bridge.clearCloudCredentialSyncPassword();
      setEncryptionPassword("");
      setConfirmEncryptionPassword("");
      setCredentialSync(false);
      // Disabling removes only this device's key. The encrypted Drive backup
      // remains available to unlock again here or on another device.
      setCredentialBackupExists(true);
      notify(t("Login credential sync is disabled on this device."));
    });
  const importFile = async (file: File) => {
    if (!confirm(t("Replace the data on this device with this file?"))) return;
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
      !confirm(
        t(
          "Erase all data on this device and start over? This cannot be undone.",
        ),
      )
    )
      return;
    await workspaceActions.reset();
    notify(t("This device was reset."));
  };

  const lastBackup = data.syncStatus.lastSyncedAt;
  return (
    <>
      <SettingsHeader title="Backup" />
      <SettingsGroup
        title={serviceName}
        footer={
          provider === "icloud"
            ? t("Stored privately in your iCloud account with CloudKit.")
            : t("Stored unencrypted in a private app folder in your Drive.")
        }
      >
        {!connected && (
          <Row id="cloud-provider" group title="Cloud service">
            <div className="actions">
              <UiButton
                className={
                  provider === "google-drive"
                    ? "button small primary"
                    : "button small"
                }
                disabled={busy}
                onClick={() => setProvider("google-drive")}
              >
                Google Drive
              </UiButton>
              {appleDevice && (
                <UiButton
                  className={
                    provider === "icloud"
                      ? "button small primary"
                      : "button small"
                  }
                  disabled={busy}
                  onClick={() => setProvider("icloud")}
                >
                  iCloud
                </UiButton>
              )}
            </div>
          </Row>
        )}
        <Row
          id="cloud"
          group
          title={connected ? "Connected" : "Not connected"}
          hint={
            data.syncStatus.state === "error" && data.syncStatus.message ? (
              <span className="danger">{data.syncStatus.message}</span>
            ) : connected ? (
              lastBackup ? (
                t("Last backup {{date}}", {
                  date: formatDay(lastBackup, locale, {
                    hour: "2-digit",
                    minute: "2-digit",
                  }),
                })
              ) : (
                t("Never backed up")
              )
            ) : undefined
          }
        >
          <div className="actions">
            {connected ? (
              <>
                <UiButton
                  className="textButton"
                  disabled={busy}
                  onClick={disconnect}
                >
                  {t("Disconnect")}
                </UiButton>
                <UiButton
                  className="button small"
                  disabled={busy}
                  onClick={restore}
                >
                  {t("Restore")}
                </UiButton>
                <UiButton
                  className="button small primary"
                  disabled={busy}
                  onClick={backUp}
                >
                  {t("Back up now")}
                </UiButton>
              </>
            ) : (
              <UiButton
                className="button small primary"
                disabled={
                  (provider === "google-drive" && !clientId) ||
                  !cloudAvailable ||
                  busy
                }
                onClick={connect}
              >
                {t("Connect")}
              </UiButton>
            )}
          </div>
        </Row>
        {conflict && (
          <div className="alert" role="alert">
            <b>{t("The cloud backup is different from this device")}</b>
            <div className="actions">
              <UiButton
                className="button"
                disabled={busy}
                onClick={keepThisDevice}
              >
                {t("Keep this device")}
              </UiButton>
              <UiButton className="button" disabled={busy} onClick={useCloud}>
                {t("Use the cloud backup")}
              </UiButton>
              <UiButton
                className="button"
                onClick={() => void exportWorkspace()}
              >
                {t("Save a copy first")}
              </UiButton>
            </div>
          </div>
        )}
        {connected && provider === "google-drive" && (
          <>
            <Row
              id="credential-sync"
              group
              title={
                credentialSync
                  ? "Login credentials synced"
                  : credentialBackupExists
                    ? "Restore login credentials"
                    : "Create credential backup"
              }
              hint={
                credentialSync
                  ? "Enabled on this device"
                  : credentialBackupExists === null
                    ? "Checking Google Drive…"
                    : credentialBackupExists
                      ? "Enter the encryption password used on your other device"
                      : "Create a password to encrypt your ISCT portal accounts"
              }
            >
              <div className="actions">
                <UiInput
                  type="password"
                  value={encryptionPassword}
                  minLength={8}
                  autoComplete="new-password"
                  placeholder={
                    credentialSync
                      ? t("Enter a new 8–12 character password")
                      : credentialBackupExists
                        ? t("Encryption password")
                        : t("Create an 8–12 character password")
                  }
                  aria-label={t("Credential encryption password")}
                  disabled={busy}
                  onChange={(event) =>
                    setEncryptionPassword(event.currentTarget.value)
                  }
                />
                {(credentialSync || credentialBackupExists === false) && (
                  <UiInput
                    type="password"
                    value={confirmEncryptionPassword}
                    minLength={8}
                    autoComplete="new-password"
                    placeholder={t("Confirm password")}
                    aria-label={t("Confirm credential encryption password")}
                    disabled={busy}
                    onChange={(event) =>
                      setConfirmEncryptionPassword(event.currentTarget.value)
                    }
                  />
                )}
                <UiButton
                  className="button small"
                  disabled={
                    busy ||
                    credentialBackupExists === null ||
                    encryptionPasswordLength < 8 ||
                    encryptionPasswordLength > 12 ||
                    ((credentialSync || credentialBackupExists === false) &&
                      !passwordsMatch)
                  }
                  onClick={saveCredentialSync}
                >
                  {credentialSync
                    ? t("Change")
                    : credentialBackupExists
                      ? t("Restore")
                      : t("Create and sync")}
                </UiButton>
                {credentialSync && (
                  <UiButton
                    className="button small"
                    disabled={busy}
                    onClick={disableCredentialSync}
                  >
                    {t("Disable")}
                  </UiButton>
                )}
              </div>
            </Row>
            {(credentialSync || credentialBackupExists === false) &&
              confirmEncryptionPassword &&
              !passwordsMatch && (
                <div className="alert" role="alert">
                  {t("The two passwords do not match.")}
                </div>
              )}
            <div className="alert" role="note">
              {t(
                "If you sync ISCT login credentials, choose an 8–12 character encryption password. The encrypted credential file is stored in Google Drive. The encryption password stays only in this device's system credential vault and is never uploaded. Beyond this encryption, we cannot guarantee the security of your credentials. Enter the same password on another device to restore them.",
              )}
            </div>
          </>
        )}
        {connected && provider === "icloud" && (
          <div className="alert" role="note">
            {t(
              "ISCT login credentials sync separately through iCloud Keychain. Device-only settings such as window layout, display density, and local permissions stay on this device.",
            )}
          </div>
        )}
      </SettingsGroup>
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
