import { lazy, Suspense, useState, useSyncExternalStore } from "react";
import { Modal } from "@mantine/core";
import { desktopBridge, hideNativeSchoolWeb } from "../../app/native";
import { notify } from "../../app/notify";
import { t } from "../../i18n";
import type { SchoolDownloadEvent } from "../../platform/desktop-bridge";
import { ImageView, Toolbar } from "../lms/FilePreview";
import { LmsIcon } from "../lms/LmsIcon";
import { localFileUrl } from "../lms/files";
import "../lms/lms.css";
const PdfDocument = lazy(() => import("../lms/PdfDocument").then(m => ({ default: m.PdfDocument })));

type Preview = { name: string; path: string; kind: "pdf" | "image" };

/**
 * The attachment a school view handed to the app's viewer (Core decides which
 * attachments; see `attachment_action`). One per app: whichever page owns the
 * native school view steps aside while it is shown, because the native view
 * sits above the DOM.
 */
let current: Preview | undefined;
const listeners = new Set<() => void>();
const publish = (next: Preview | undefined) => {
  current = next;
  listeners.forEach(listener => listener());
};
let unlisten: (() => void) | undefined;
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  unlisten ??= desktopBridge?.onSchoolDownload?.((event: SchoolDownloadEvent) => {
    if (event.state !== "done" || !event.preview || !event.path) return;
    const next = { name: event.name, path: event.path, kind: event.preview };
    void hideNativeSchoolWeb().catch(() => undefined).finally(() => publish(next));
  });
  return () => {
    listeners.delete(listener);
    if (!listeners.size) {
      unlisten?.();
      unlisten = undefined;
    }
  };
};

/** True while an attachment preview covers the school view. */
export const useSchoolFilePreviewOpen = () =>
  useSyncExternalStore(subscribe, () => Boolean(current));

export function SchoolFilePreview() {
  const preview = useSyncExternalStore(subscribe, () => current);
  const [error, setError] = useState("");
  const close = () => {
    setError("");
    publish(undefined);
  };
  const save = async () => {
    if (!preview) return;
    try {
      if (!(await desktopBridge?.saveSchoolPreview?.(preview.path)))
        notify.info("This file can no longer be saved.");
    } catch (error) {
      notify.error(error);
    }
  };
  const src = preview ? localFileUrl(preview.path) : "";
  const toolbar = preview && (
    <Toolbar title={preview.name} url={preview.name}>
      {desktopBridge?.saveSchoolPreview && (
        <button type="button" onClick={() => void save()}>
          <LmsIcon name="download" size={14} />
          {t("Save to Downloads")}
        </button>
      )}
      <button type="button" onClick={close}>{t("Close")}</button>
    </Toolbar>
  );
  return (
    <Modal opened={Boolean(preview)} onClose={close} fullScreen withCloseButton={false} padding={0}
      styles={{ content: { display: "flex", flexDirection: "column" }, body: { flex: 1, minHeight: 0, display: "flex", flexDirection: "column" } }}>
      {preview && (error ? (
        <div className="lmsAttachmentPreview">{toolbar}<p className="lmsQuiet">{error}</p></div>
      ) : preview.kind === "pdf" ? (
        <Suspense fallback={<p className="lmsQuiet lmsLoading">{t("Loading preview…")}</p>}>
          <PdfDocument key={preview.path} src={src} title={preview.name} fileToolbar={toolbar} />
        </Suspense>
      ) : (
        <div className="lmsAttachmentPreview">
          {toolbar}
          <ImageView key={preview.path} src={src} title={preview.name}
            onBroken={() => setError(t("This image can't be displayed here."))} />
        </div>
      ))}
    </Modal>
  );
}
