import { useEffect, useState } from "react";
import { t } from "../../i18n";
import type { Workspace } from "../../core/model";
import { errorText } from "../../app/shared";
import { desktopBridge, saveExportFile } from "../../app/native";
import { workspaceActions } from "../../store/workspace";
import { resolveCapabilities } from "../../platform/capabilities";
import type {
  CloudProvider,
  CloudSyncResult,
} from "../../platform/desktop-bridge";
import { UiButton, UiInput } from "../../components/mantine";
import { Row, SettingsGroup, SettingsHeader } from "../../components/forms";
import { formatDay } from "./subscription-status";

// Google Drive is the only cloud service offered for now.
const PROVIDER: CloudProvider = "google-drive";

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
  const [encryptionPassword, setEncryptionPassword] = useState("");
  const encryptionPasswordLength = Array.from(encryptionPassword).length;
  const [busy, setBusy] = useState(false);
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
    cloud.provider === PROVIDER && authorized.includes(PROVIDER);

  const requireCloud = () => {
    if (!cloudAvailable || !desktopBridge)
      throw new Error(caps["cloud-sync"].reason);
    return { bridge: desktopBridge, provider: PROVIDER };
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
      await bridge.connectCloud(provider, clientId, cloud.redirectUri!);
      setAuthorized([provider]);
      update((draft) => {
        draft.settings.cloud.provider = provider;
        draft.syncStatus.state = "idle";
        draft.syncStatus.message = undefined;
        return draft;
      });
      notify(t("Connected to {{service}}.", { service: "Google Drive" }));
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
      const { bridge } = requireCloud();
      await bridge.setCloudCredentialSyncPassword(encryptionPassword);
      setEncryptionPassword("");
      setCredentialSync(true);
      notify(t("Login credential sync is enabled on this device."));
    });
  const disableCredentialSync = () =>
    run("Could not disable credential sync.", async () => {
      const { bridge } = requireCloud();
      await bridge.clearCloudCredentialSyncPassword();
      setEncryptionPassword("");
      setCredentialSync(false);
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
        title="Google Drive"
        footer={t("Stored unencrypted in a private app folder in your Drive.")}
      >
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
                disabled={!clientId || !cloudAvailable || busy}
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
        {connected && (
          <>
            <Row
              id="credential-sync"
              group
              title="Sync login credentials"
              hint={
                credentialSync
                  ? "Enabled on this device"
                  : "Optional encrypted sync for ISCT portal accounts"
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
                      : t("8–12 character password")
                  }
                  aria-label={t("Credential encryption password")}
                  disabled={busy}
                  onChange={(event) =>
                    setEncryptionPassword(event.currentTarget.value)
                  }
                />
                <UiButton
                  className="button small"
                  disabled={
                    busy ||
                    encryptionPasswordLength < 8 ||
                    encryptionPasswordLength > 12
                  }
                  onClick={saveCredentialSync}
                >
                  {credentialSync ? t("Change") : t("Enable")}
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
            <div className="alert" role="note">
              {t(
                "If you sync ISCT login credentials, choose an 8–12 character encryption password. The encrypted credential file is stored in Google Drive. The encryption password stays only in this device's system credential vault and is never uploaded. Beyond this encryption, we cannot guarantee the security of your credentials. Enter the same password on another device to restore them.",
              )}
            </div>
          </>
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
