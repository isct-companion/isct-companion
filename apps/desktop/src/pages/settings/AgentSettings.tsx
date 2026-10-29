import { History, Server, ServerOff } from "lucide-react";
import { useEffect, useState } from "react";
import { t, type MessageKey } from "../../i18n";
import type { Workspace } from "../../lib/model";
import { desktopBridge } from "../../app/native";
import type { AgentConnection } from "../../platform/desktop-bridge";
import { SegmentedControl, EmptyState } from "../../components/ui";
import { UiButton } from "../../components/ui";
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
      .then(() => notify(t("settings.credentialImport.copied")))
      .catch(() => notify(t("settings.otp.copyFailed")));
  const access = data.settings.agentAccess;
  const writes = [...(access?.lastWrites ?? [])]
    .sort((a, b) => b.at.localeCompare(a.at))
    .slice(0, 8);
  return (
    <>
      <SettingsHeader title="settings.agent.aiAgentAccess" />
      <SettingsGroup>
        <Row
          id="mcp"
          title="settings.agent.allowLocalAgents"
          hint={t("settings.agent.agentsCanReadSchoolData")}
        >
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
      {/* While access is off there is nothing to connect to; the switch says it all. */}
      {access?.enabled && (
        <SettingsGroup
          title="settings.agent.connectAnAgent"
          footer={
            connection?.state === "listening"
              ? t("settings.agent.pickYourAssistantBelowAnd")
              : undefined
          }
        >
          {connection?.state === "listening" ? (
            <>
              <div className="commandLine">
                <code>{connection.url}</code>
                <UiButton
                  className="button small"
                  onClick={() => copy(connection.url)}
                >
                  {t("settings.agent.copyURL")}
                </UiButton>
              </div>
              <Row title="settings.agent.accessToken">
                <UiButton
                  className="button small"
                  onClick={() => copy(connection.token)}
                >
                  {t("settings.agent.copyToken")}
                </UiButton>
              </Row>
              <AgentSetup
                url={connection.url}
                token={connection.token}
                copy={copy}
              />
            </>
          ) : (
            <EmptyState
              compact
              icon={connection?.state === "failed" ? ServerOff : Server}
              role={connection?.state === "failed" ? "alert" : "status"}
            >
              {connection?.state === "failed"
                ? `${t("settings.agent.mcpServerIsUnavailable")} ${connection.error}`
                : t("common.loading")}
            </EmptyState>
          )}
        </SettingsGroup>
      )}
      {(access?.enabled || writes.length > 0) && (
        <SettingsGroup title="settings.agent.recentAgentChanges">
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
            <EmptyState compact icon={History}>
              {t("settings.agent.noChangesByAgentsYet")}
            </EmptyState>
          )}
        </SettingsGroup>
      )}
    </>
  );
}

type AgentClient = "claude-code" | "codex" | "claude-desktop" | "cursor";

const SERVER = "isct-companion";

/**
 * What to paste into each common MCP client. Each client spells a Streamable
 * HTTP server with a Bearer token differently; Claude Desktop has no HTTP
 * transport for local servers, so it goes through `mcp-remote`.
 */
export function agentSetup(
  client: AgentClient,
  url: string,
  token: string,
): { where: MessageKey; text: string } {
  const bearer = `Bearer ${token}`;
  switch (client) {
    case "claude-code":
      return {
        where: "settings.agent.runInATerminal",
        text: `claude mcp add --transport http ${SERVER} ${url} --header "Authorization: ${bearer}"`,
      };
    case "codex":
      return {
        where: "settings.agent.addToCodexConfigToml",
        text: `[mcp_servers.${SERVER}]\nurl = "${url}"\nhttp_headers = { Authorization = "${bearer}" }`,
      };
    case "claude-desktop":
      return {
        where: "settings.agent.addToClaudeDesktopConfig",
        text: JSON.stringify(
          {
            mcpServers: {
              [SERVER]: {
                command: "npx",
                args: [
                  "-y",
                  "mcp-remote",
                  url,
                  "--header",
                  "Authorization:${AUTH}",
                ],
                env: { AUTH: bearer },
              },
            },
          },
          null,
          2,
        ),
      };
    case "cursor":
      return {
        where: "settings.agent.addToCursorMcpJson",
        text: JSON.stringify(
          {
            mcpServers: {
              [SERVER]: { url, headers: { Authorization: bearer } },
            },
          },
          null,
          2,
        ),
      };
  }
}

/** Ready-to-paste setup per client; the token shows masked but copies whole. */
function AgentSetup({
  url,
  token,
  copy,
}: {
  url: string;
  token: string;
  copy: (value: string) => void;
}) {
  const [client, setClient] = useState<AgentClient>("claude-code");
  const setup = agentSetup(client, url, token);
  const shown = agentSetup(client, url, "•".repeat(8));
  return (
    <div className="agentSetup">
      <SegmentedControl
        size="xs"
        value={client}
        onChange={(value) => setClient(value as AgentClient)}
        data={[
          { value: "claude-code", label: "Claude Code" },
          { value: "codex", label: "Codex" },
          { value: "claude-desktop", label: "Claude Desktop" },
          { value: "cursor", label: "Cursor" },
        ]}
      />
      <p>{t(setup.where)}</p>
      <div className="agentSnippet">
        <pre>{shown.text}</pre>
        <UiButton className="button small" onClick={() => copy(setup.text)}>
          {t("settings.schoolAccounts.copy")}
        </UiButton>
      </div>
    </div>
  );
}
