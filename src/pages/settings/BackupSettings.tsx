import { useEffect, useRef, useState } from "react";
import { t } from "../../i18n";
import type { Workspace } from "../../core/model";
import type { Platform } from "../../app/shared";
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
import { UiButton, UiSelect } from "../../components/mantine";
import {
  Row,
  SettingsGroup,
  SettingsHeader,
  Status,
} from "../../components/forms";
import { formatDay } from "./subscription-status";

const providerName = (provider: CloudProvider) =>
  provider === "google-drive" ? "Google Drive" : "OneDrive";

function bytesToBase64(bytes: Uint8Array) {
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 0x8000)
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  return btoa(binary);
}

export function BackupSettings({
  data,
  update,
  platform,
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
  const cloudAvailable =
    platform === "browser-preview" ||
    caps["cloud-sync"].support === "supported";
  const selected = cloud.provider === "none" ? null : cloud.provider;
  const connected = Boolean(
    selected &&
    (authorized.includes(selected) ||
      (!desktopBridge && data.syncStatus.state === "idle")),
  );
  const configured = (provider: CloudProvider) =>
    Boolean(resolveCloudClientId(provider, cloud.clientId));

  const adapter = async () => {
    if (
      detectPlatform() !== "browser-preview" &&
      caps["cloud-sync"].support !== "supported"
    )
      throw new Error(caps["cloud-sync"].reason);
    if (cloud.provider === "none")
      throw new Error(t("Choose Google Drive or OneDrive first."));
    const provider = cloud.provider;
    const clientId = resolveCloudClientId(provider, cloud.clientId);
    const bridge = desktopBridge;
    const desktopOAuth = bridge
      ? {
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
        }
      : {};
    return createCloudAdapter(provider, clientId, {
      redirectUri: desktopBridge
        ? cloud.redirectUri
        : `${location.origin}${location.pathname}`,
      readOnly: !desktopBridge,
      deviceId: data.deviceId,
      ...desktopOAuth,
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
      await (await adapter()).connect();
      const provider = cloud.provider as CloudProvider;
      setAuthorized([provider]);
      mark("idle", t("Connected. Nothing backed up yet."));
      notify(
        t("Connected to {{service}}.", { service: providerName(provider) }),
      );
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
                onClick={() => void exportWorkspace()}
              >
                {t("Save a copy first")}
              </UiButton>
            </div>
          </div>
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
              onClick={() => void exportWorkspace()}
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
