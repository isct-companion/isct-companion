import type { ComponentType } from "react";
import { t } from "../../i18n";
import type { Platform } from "../../app/shared";
import { CampusPrinter } from "./CampusPrinter";
import { CampusVpn } from "./CampusVpn";
import { CampusWifi } from "./CampusWifi";
import { availableTools } from "./registry";
import type { DesktopTool } from "../../platform/desktop-bridge";
import "./tools.css";

const cards: Record<
  DesktopTool,
  ComponentType<{ notify: (message: string) => void }>
> = {
  "campus-wifi": CampusWifi,
  "campus-vpn": CampusVpn,
  "campus-printer": CampusPrinter,
};

/**
 * What the desktop app does on the campus network, as opposed to how it is
 * set up: one small card per tool, each acting when pressed.
 */
export function Tools({
  platform,
  notify,
}: {
  platform: Platform;
  notify: (message: string) => void;
}) {
  return (
    <div className="page tools">
      <h2>{t("common.tools")}</h2>
      <div className="toolCards">
        {availableTools(platform).map((tool) => {
          const Card = cards[tool];
          return <Card key={tool} notify={notify} />;
        })}
      </div>
    </div>
  );
}
