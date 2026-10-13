import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { t } from "../../i18n";
import type { Workspace } from "../../core/model";
import { desktopBridge } from "../../app/native";
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
  const [connection, setConnection] = useState<{
    url: string;
    token: string;
  }>();
  const [error, setError] = useState<string>();
  useEffect(() => {
    if (!desktopBridge) return;
    let active = true;
    void invoke<{ url: string; token: string }>("agent_connection")
      .then((value) => {
        if (active) setConnection(value);
      })
      .catch((reason: unknown) => {
        if (active) setError(String(reason));
      });
    return () => {
      active = false;
    };
  }, []);
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
        {connection ? (
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
          <p className="paneEmpty" role={error ? "alert" : "status"}>
            {error
              ? `${t("MCP server is unavailable.")} ${error}`
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
