import { useEffect, useRef, useState } from "react";
import { ErrorBanner, Modal, Stack, Text } from "./ui";
import { CameraPreview } from "./CameraPreview";
import { t } from "../i18n";

export function QrCamera({
  onClose,
  onRead,
}: {
  onClose: () => void;
  onRead: (raw: string) => void;
}) {
  const [video, setVideo] = useState<HTMLVideoElement | null>(null);
  const [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);
  const read = useRef(onRead);
  read.current = onRead;
  useEffect(() => {
    if (!video) return;
    let disposed = false;
    let controls: { stop: () => void } | undefined;
    const stop = () => {
      disposed = true;
      controls?.stop();
    };
    const visibility = () => {
      if (document.hidden) {
        stop();
        setError(t("settings.matrixCamera.cameraScanStoppedWhileThe"));
      }
    };
    document.addEventListener("visibilitychange", visibility);
    setError("");
    void import("@zxing/browser")
      .then(async ({ BrowserQRCodeReader }) => {
        if (disposed) return;
        const active = await new BrowserQRCodeReader().decodeFromVideoDevice(
          undefined,
          video,
          (result, _error, active) => {
            if (disposed || !result) return;
            active.stop();
            disposed = true;
            read.current(result.getText());
          },
        );
        controls = active;
        if (disposed) active.stop();
      })
      .catch((reason: unknown) => {
        if (disposed) return;
        stop();
        setError(
          reason instanceof DOMException && reason.name === "NotAllowedError"
            ? t("settings.matrixCamera.cameraAccessWasDeniedAllow")
            : t("qrCamera.theCameraIsUnavailableSelect"),
        );
      });
    return () => {
      stop();
      document.removeEventListener("visibilitychange", visibility);
    };
  }, [video, attempt]);
  return (
    <Modal
      opened
      onClose={onClose}
      title={t("qrCamera.scanAuthenticatorQRCode")}
      size="lg"
    >
      <Stack gap="sm">
        <Text size="sm">{t("qrCamera.showTheWholeQRCode")}</Text>
        <CameraPreview ref={setVideo} />
        {error && (
          <ErrorBanner onRetry={() => setAttempt((n) => n + 1)}>
            {error}
          </ErrorBanner>
        )}
      </Stack>
    </Modal>
  );
}
