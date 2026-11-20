import { desktopBridge } from "./native";
import { t } from "../i18n";
import { errorText } from "../lib/error-text";

/**
 * Saves an export through the system Save dialog (the webview cancels
 * `<a download>`) and reports the outcome. Cancelling the dialog stays quiet.
 */
export async function saveExportFile(
  name: string,
  contents: () => string | Promise<string>,
  notify: (message: string) => void,
) {
  try {
    if (!desktopBridge) throw new Error(t("app.savingFilesNeedsTheDesktop"));
    if (await desktopBridge.saveFile(name, await contents()))
      notify(t("app.savedName", { name }));
  } catch (error) {
    notify(
      t("settings.main.exportFailedReason", {
        reason: errorText(error) ?? t("common.unknownError"),
      }),
    );
  }
}
