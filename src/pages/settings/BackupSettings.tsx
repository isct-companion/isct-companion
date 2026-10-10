import { useEffect, useRef, useState } from "react";
import { t } from "../../i18n";
import type { Workspace } from "../../core/model";
import { errorText } from "../../app/shared";
import { desktopBridge, saveExportFile } from "../../app/native";
import { workspaceActions } from "../../store/workspace";
import { performAction } from "../../platform/actions";
import {
  detectPlatform,
  resolveCapabilities,
} from "../../platform/capabilities";
import {
  CloudTransportError,
  createCloudAdapter,
  resolveCloudClientId,
  type CloudAdapter,
  type CloudProvider,
} from "../../connectors/cloud";
import { UiButton } from "../../components/mantine";
import { Row, SettingsGroup, SettingsHeader } from "../../components/forms";
import { formatDay } from "./subscription-status";

function bytesToBase64(bytes: Uint8Array) {
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 0x8000)
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  return btoa(binary);
}

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
  const [conflict, setConflict] = useState<string | null>(null);
  const [authorized, setAuthorized] = useState<CloudProvider[]>([]);
  const [busy, setBusy] = useState(false);
  const pending = useRef<CloudAdapter | null>(null);
  useEffect(() => {
    if (!desktopBridge) return;
    let disposed = false;
    void desktopBridge
      .cloudAuthorizationStatus()
      .then((providers) => {
        if (!disposed) setAuthorized(providers);
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
  // Google Drive is the only cloud service offered.
  const provider: CloudProvider = "google-drive";
  const connected =
    cloud.provider === provider && authorized.includes(provider);

  const adapter = async () => {
    const bridge = desktopBridge;
    if (!bridge || !cloudAvailable) throw new Error(caps["cloud-sync"].reason);
    const clientId = resolveCloudClientId(provider, cloud.clientId);
    return createCloudAdapter(provider, clientId, {
      redirectUri: cloud.redirectUri,
      deviceId: data.deviceId,
      oauth: {
        connect: () =>
          bridge
            .connectCloud(provider, clientId, cloud.redirectUri!)
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
        const response = await bridge.cloudRequest(
          provider,
          url instanceof Request ? url.url : String(url),
          {
            method: init?.method,
            headers: Object.fromEntries(new Headers(init?.headers)),
            bodyText: typeof body === "string" ? body : undefined,
            bodyBase64: binary ? bytesToBase64(binary) : undefined,
          },
        );
        return new Response(
          Uint8Array.from(atob(response.body), (c) => c.charCodeAt(0)),
          {
            status: response.status,
            statusText: response.statusText,
            headers: response.headers,
          },
        );
      },
    });
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
      await (await adapter()).connect();
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
      const remote = await (await adapter()).pull();
      if (!confirm(t("Replace the data on this device with the cloud backup?")))
        return;
      await workspaceActions.importData(remote);
      mark("idle", t("Restored from the cloud backup."), true);
      notify(t("Restored from the cloud backup."));
    });
  const backUp = () =>
    run("Could not back up.", async () => {
      const cloudAdapter = await adapter();
      try {
        const remote = await cloudAdapter.pull();
        const current = await workspaceActions.exportData();
        if (
          JSON.stringify(JSON.parse(remote).tables) !==
          JSON.stringify(JSON.parse(current).tables)
        ) {
          pending.current = cloudAdapter;
          setConflict(remote);
          return;
        }
      } catch (error) {
        if (!(error instanceof CloudTransportError) || error.status !== 404)
          throw error;
      }
      await performAction("push-cloud", detectPlatform(), () =>
        workspaceActions
          .exportData()
          .then((document) => cloudAdapter.push(document)),
      );
      mark("idle", t("Backup is up to date."), true);
      notify(t("Backed up."));
    });
  const keepThisDevice = () =>
    run("Could not back up.", async () => {
      await pending.current?.push(await workspaceActions.exportData());
      setConflict(null);
      mark("idle", t("Backup is up to date."), true);
      notify(t("Kept this device and backed it up."));
    });
  const useCloud = () =>
    run("Could not restore.", async () => {
      await workspaceActions.importData(conflict!);
      setConflict(null);
      mark("idle", t("Restored from the cloud backup."), true);
      notify(t("Restored from the cloud backup."));
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
                disabled={!cloudAvailable || busy}
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
