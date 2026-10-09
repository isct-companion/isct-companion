import { useEffect, useState } from "react";
import { t } from "../../i18n";
import type { Workspace } from "../../core/model";
import type { Platform } from "../../app/shared";
import { desktopBridge } from "../../app/native";
import { workspaceActions } from "../../store/workspace";
import { resolveCapabilities } from "../../platform/capabilities";
import type {
  CloudProvider,
  CloudSyncResult,
} from "../../platform/desktop-bridge";
import { UiButton, UiInput, UiSelect } from "../../components/mantine";
import {
  Row,
  SettingsGroup,
  SettingsHeader,
  Status,
} from "../../components/forms";
import { formatDay } from "./subscription-status";

const providerName = (provider: CloudProvider) =>
  provider === "google-drive" ? "Google Drive" : "OneDrive";

const GOOGLE_DRIVE_CLIENT_ID =
  import.meta.env.VITE_GOOGLE_DRIVE_CLIENT_ID?.trim() ?? "";
const resolveCloudClientId = (provider: CloudProvider, override: string) =>
  override.trim() ||
  (provider === "google-drive" ? GOOGLE_DRIVE_CLIENT_ID : "");

async function exportWorkspaceFile() {
  const blob = new Blob([await workspaceActions.exportData()], {
    type: "application/json",
  });
  const anchor = document.createElement("a");
  anchor.href = URL.createObjectURL(blob);
  anchor.download = "IsctCompanion.json";
  anchor.click();
  URL.revokeObjectURL(anchor.href);
}

export function BackupSettings({
  data,
  update,
  caps,
  notify,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
  platform: Platform;
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

  const cloud = data.settings.cloud;
  const cloudAvailable = caps["cloud-sync"].support === "supported";
  const selected = cloud.provider === "none" ? null : cloud.provider;
  const connected = Boolean(
    selected &&
    (authorized.includes(selected) ||
      (!desktopBridge && data.syncStatus.state === "idle")),
  );
  const configured = (provider: CloudProvider) =>
    Boolean(resolveCloudClientId(provider, cloud.clientId));

  const requireCloud = () => {
    if (!cloudAvailable || !desktopBridge)
      throw new Error(caps["cloud-sync"].reason);
    if (cloud.provider === "none")
      throw new Error(t("Choose Google Drive or OneDrive first."));
    return { bridge: desktopBridge, provider: cloud.provider };
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
      const message = error instanceof Error ? error.message : t(failure);
      mark("error", message);
      notify(message);
    } finally {
      setBusy(false);
    }
  };

  const chooseProvider = async (
    next: Workspace["settings"]["cloud"]["provider"],
  ) => {
    if (next === cloud.provider) return;
    const disconnect = authorized.filter((provider) => provider !== next);
    if (
      disconnect.length > 0 &&
      !confirm(
        t(
          "Switch cloud service? This signs out of {{services}} on this device.",
          {
            services: disconnect.map(providerName).join(" / "),
          },
        ),
      )
    )
      return;
    await run("Could not switch cloud service.", async () => {
      const bridge = desktopBridge;
      if (bridge)
        await Promise.all(
          disconnect.map((provider) => bridge.disconnectCloud(provider)),
        );
      const remaining = authorized.filter(
        (provider) => !disconnect.includes(provider),
      );
      setAuthorized(remaining);
      update((draft) => {
        draft.settings.cloud.provider = next;
        draft.syncStatus.state =
          next !== "none" && remaining.includes(next)
            ? "idle"
            : "needs-configuration";
        draft.syncStatus.lastSyncedAt = undefined;
        draft.syncStatus.message = undefined;
        return draft;
      });
    });
  };
  const connect = () =>
    run("Could not connect.", async () => {
      const { bridge, provider } = requireCloud();
      await bridge.connectCloud(
        provider,
        resolveCloudClientId(provider, cloud.clientId),
        cloud.redirectUri!,
      );
      setAuthorized([provider]);
      mark("idle", t("Connected. Nothing backed up yet."));
      notify(
        t("Connected to {{service}}.", { service: providerName(provider) }),
      );
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
      notify(error instanceof Error ? error.message : t("Import failed."));
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
      <SettingsHeader
        title="Backup & sync"
        description="Save your plans, calendar changes and preferences, and bring them to another device. School data is downloaded again from the school."
      />
      <SettingsGroup
        title="Cloud backup"
        footer={t(
          "Saved as a readable file in a private app folder of your cloud account. The file is not encrypted by this app; your cloud account protects it.",
        )}
      >
        <Status
          id="cloud"
          tone={
            data.syncStatus.state === "error"
              ? "error"
              : connected
                ? "ok"
                : "idle"
          }
        >
          {!selected
            ? t("Cloud backup is off.")
            : data.syncStatus.state === "error"
              ? data.syncStatus.message
              : connected
                ? t("Connected to {{service}}", {
                    service: providerName(selected),
                  })
                : t("Connect {{service}} to start backing up.", {
                    service: providerName(selected),
                  })}
          {selected && (
            <small>
              {" · "}
              {lastBackup
                ? t("Last backup {{date}}", {
                    date: formatDay(lastBackup, locale, {
                      hour: "2-digit",
                      minute: "2-digit",
                    }),
                  })
                : t("Never backed up")}
            </small>
          )}
        </Status>
        <Row title="Cloud service">
          <UiSelect
            value={cloud.provider}
            disabled={busy || !cloudAvailable}
            onChange={(event) =>
              void chooseProvider(
                event.target
                  .value as Workspace["settings"]["cloud"]["provider"],
              )
            }
          >
            <option value="none">{t("Off")}</option>
            {(["google-drive", "onedrive"] as const).map((provider) => (
              <option
                key={provider}
                value={provider}
                disabled={!configured(provider) && cloud.provider !== provider}
              >
                {configured(provider)
                  ? providerName(provider)
                  : `${providerName(provider)} (${t("not set up in this build")})`}
              </option>
            ))}
          </UiSelect>
        </Row>
        {selected && (
          <div className="actions end">
            <UiButton
              className={`button${connected ? "" : " primary"}`}
              disabled={!cloudAvailable || busy}
              onClick={connect}
            >
              {connected ? t("Reconnect") : t("Connect")}
            </UiButton>
            <UiButton
              className="button"
              disabled={!cloudAvailable || !connected || busy}
              onClick={restore}
            >
              {t("Restore from cloud")}
            </UiButton>
            {desktopBridge && cloudAvailable && (
              <UiButton
                className="button primary"
                disabled={!connected || busy}
                onClick={backUp}
              >
                {t("Back up now")}
              </UiButton>
            )}
          </div>
        )}
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
                onClick={() => {
                  void exportWorkspaceFile();
                  notify(t("Saved a copy of this device first."));
                }}
              >
                {t("Save a copy first")}
              </UiButton>
            </div>
          </div>
        )}
        {selected && (
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
                  disabled={!connected || busy}
                  onChange={(event) =>
                    setEncryptionPassword(event.currentTarget.value)
                  }
                />
                <UiButton
                  className="button small"
                  disabled={
                    !connected ||
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
                "If you sync ISCT login credentials, choose an 8–12 character encryption password. The encrypted credential file is stored in Google Drive or OneDrive. The encryption password stays only in this device's system credential vault and is never uploaded. Beyond this encryption, we cannot guarantee the security of your credentials. Enter the same password on another device to restore them.",
              )}
            </div>
          </>
        )}
      </SettingsGroup>
      <SettingsGroup title="Backup file">
        <Row
          id="file"
          group
          title="Back up to a file"
          hint="A JSON file you can keep anywhere"
        >
          <div className="actions">
            <UiButton
              className="button small"
              onClick={() => void exportWorkspaceFile()}
            >
              {t("Export…")}
            </UiButton>
          </div>
        </Row>
        <Row
          group
          title="Restore from a file"
          hint="Replaces the data on this device"
        >
          <div className="actions">
            <label className="button">
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
      <SettingsGroup title="Reset" tone="danger">
        <Row
          id="reset"
          group
          title="Erase data on this device"
          hint="Removes plans, calendar changes and preferences, then starts empty"
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
