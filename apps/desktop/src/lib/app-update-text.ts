import { t } from "../i18n";
import type { AppLastInstall } from "../platform/desktop-bridge";

export function lastInstallText(last: AppLastInstall) {
  switch (last.outcome) {
    case "installed":
      return t("app.updatedToVersion", { version: last.version });
    case "incomplete":
      return t("app.theUpdateToVersionDid", { version: last.version });
    case "failed":
      return t("app.installingVersionFailedMessage", {
        version: last.version,
        message: last.message,
      });
  }
}
