import { useState } from "react";
import { desktopBridge } from "../platform/native";
import { notify } from "./notifications/notify";
import { t, type MessageKey } from "../i18n";
import {
  nativeOs,
  type DownloadEvent,
  type SavedDownload,
} from "../platform/desktop-bridge";
import { UiButton } from "./ui";

const messages: Record<DownloadEvent["state"], MessageKey> = {
  started: "web.downloadingName",
  done: "web.savedNameToDownloads",
  failed: "web.downloadFailedName",
};
const revealLabel: MessageKey =
  nativeOs() === "macos" ? "web.showInFinder" : "web.showInFolder";

/** One result and action for downloads, in a native-webview notice or a toast. */
function feedback(
  download: DownloadEvent,
  onError: (error: unknown) => void,
  onMissing = () => onError(new Error(t("web.thatFileCanNoLonger"))),
) {
  const path = download.path;
  return {
    message: t(messages[download.state], { name: download.name }),
    action:
      download.state === "done" && path
        ? {
            label: t(revealLabel),
            run: () => {
              const reveal = async () => {
                if (!desktopBridge?.revealDownload)
                  throw new Error(t("error.unavailable"));
                if (!(await desktopBridge.revealDownload(path))) onMissing();
              };
              void reveal().catch(onError);
            },
          }
        : undefined,
  };
}

export function notifyDownload(saved: SavedDownload) {
  const notice = feedback({ ...saved, state: "done" }, notify.error);
  notify.info(notice.message, notice.action);
}

/** School pages need this above the native webview, which would cover a toast. */
export function DownloadNotice({
  download,
  onError,
}: {
  download: DownloadEvent;
  onError: (error: unknown) => void;
}) {
  const [unavailable, setUnavailable] = useState<DownloadEvent>();
  const missing = unavailable === download;
  const notice = feedback(download, onError, () => setUnavailable(download));
  return (
    <div
      className={`webNotice${download.state === "failed" ? " warning" : ""}`}
      role="status"
      aria-busy={download.state === "started"}
    >
      <span>{missing ? t("web.thatFileCanNoLonger") : notice.message}</span>
      {!missing && notice.action && (
        <UiButton type="button" onClick={notice.action.run}>
          {notice.action.label}
        </UiButton>
      )}
    </div>
  );
}
