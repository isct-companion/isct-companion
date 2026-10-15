import { useEffect, useState } from "react";
import { t } from "../../i18n";
import type { Workspace } from "../../lib/model";
import { desktopBridge } from "../../app/native";
import type { AgentConnection } from "../../platform/desktop-bridge";
import { UiButton } from "../../components/mantine";
import {
  Row,
  SettingsGroup,
  SettingsHeader,
  Toggle,
} from "../../components/forms";
import { formatDay } from "./subscription-status";

/** Local MCP server for AI agents (see docs/agent-access.md). */
export function AgentSettings({
  data,
  update,
  notify,
}: {
  data: Workspace;
  update: (f: (d: Workspace) => Workspace) => void;
  notify: (message: string) => void;
}) {
  const [connection, setConnection] = useState<AgentConnection>();
  const enabled = data.settings.agentAccess?.enabled === true;
  // Re-read after the setting changes: the native side starts or stops the
  // listener to match it before answering.
  useEffect(() => {
    if (!desktopBridge?.agentConnection) return;
    let active = true;
    setConnection(undefined);
    void desktopBridge
      .agentConnection()
      .then((value) => {
        if (active) setConnection(value);
      })
      .catch((reason: unknown) => {
        if (active) setConnection({ state: "failed", error: String(reason) });
      });
    return () => {
      active = false;
    };
  }, [enabled]);
  const copy = (value: string) =>
    void navigator.clipboard
      .writeText(value)
      .then(() => notify(t("Copied")))
      .catch(() => notify(t("Copy failed")));
  const access = data.settings.agentAccess;
  const writes = [...(access?.lastWrites ?? [])]
    .sort((a, b) => b.at.localeCompare(a.at))
    .slice(0, 8);
  return (
    <>
      <SettingsHeader
        title="AI agent access"
        description="Lets a local AI agent (for example an MCP client) read your timetable and plan courses."
      />
      <SettingsGroup
        footer={t(
          "Agents can read school data and change only plans, personal events and hidden assignments. They never see passwords or sessions.",
        )}
      >
        <Row id="mcp" title="Allow local agents">
          <Toggle
            checked={access?.enabled === true}
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
      </SettingsGroup>
      <SettingsGroup
        title="Connect an agent"
        footer={t(
          "Add this URL as a Streamable HTTP MCP server. Use the access token for Bearer authentication and keep IsctCompanion open.",
        )}
      >
        {connection?.state === "listening" ? (
          <>
            <div className="commandLine">
              <code>{connection.url}</code>
              <UiButton
                className="button small"
                onClick={() => copy(connection.url)}
              >
                {t("Copy URL")}
              </UiButton>
            </div>
            <Row title="Access token">
              <UiButton
                className="button small"
                onClick={() => copy(connection.token)}
              >
                {t("Copy token")}
              </UiButton>
            </Row>
          </>
        ) : (
          <p
            className="paneEmpty"
            role={connection?.state === "failed" ? "alert" : "status"}
          >
            {connection?.state === "failed"
              ? `${t("MCP server is unavailable.")} ${connection.error}`
              : connection?.state === "off"
                ? t(
                    "Turn on Allow local agents to show the connection details.",
                  )
                : t("Loading…")}
          </p>
        )}
      </SettingsGroup>
      <SettingsGroup title="Recent agent changes">
        {writes.length ? (
          <div className="logList">
            {writes.map((write, index) => (
              <div key={`${write.at}-${index}`}>
                <b>{write.tool}</b>
                <span>
                  {formatDay(write.at, data.settings.locale, {
                    hour: "2-digit",
                    minute: "2-digit",
                  })}
                </span>
              </div>
            ))}
          </div>
        ) : (
          <p className="paneEmpty">{t("No changes by agents yet.")}</p>
        )}
      </SettingsGroup>
    </>
  );
}
