import { t } from "../../i18n";
import type { Platform } from "../../app/shared";
import { CampusPrinter } from "./CampusPrinter";
import { CampusVpn } from "./CampusVpn";
import { CampusWifi } from "./CampusWifi";
import { availableTools } from "./registry";
import "./tools.css";

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
        {availableTools(platform).map((tool) =>
          tool === "wifi" ? (
            <CampusWifi key={tool} notify={notify} />
          ) : tool === "vpn" ? (
            <CampusVpn key={tool} notify={notify} />
          ) : (
            <CampusPrinter key={tool} notify={notify} />
          ),
        )}
      </div>
    </div>
  );
}
