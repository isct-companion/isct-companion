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
  const [command, setCommand] = useState(
    "isct-companion-mcp --db /absolute/path/to/school.db",
  );
  useEffect(() => {
    if (!desktopBridge) return;
    void invoke<string>("agent_command")
      .then(setCommand)
      .catch(() => undefined);
  }, []);
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
          "Add this command as a stdio MCP server in your agent. The isct-companion-mcp binary must be on your PATH.",
        )}
      >
        <div className="commandLine">
          <code>{command}</code>
          <UiButton
            className="button small"
            onClick={() =>
              void navigator.clipboard
                .writeText(command)
                .then(() => notify(t("Copied")))
                .catch(() => notify(t("Copy failed")))
            }
          >
            {t("Copy")}
          </UiButton>
        </div>
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
