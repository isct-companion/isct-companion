import { useEffect, useRef, useState } from "react";
import { Alert, Button, Modal, Stack, Text } from "@mantine/core";
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
        setError(
          t(
            "Camera scan stopped while the app was in the background. Open it again to continue.",
          ),
        );
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
            ? t(
                "Camera access was denied. Allow camera access in system settings, then try again.",
              )
            : t("The camera is unavailable. Select a QR code image instead."),
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
      title={t("Scan authenticator QR code")}
      size="lg"
    >
      <Stack gap="sm">
        <Text size="sm">
          {t(
            "Show the whole QR code to the camera. It is read only on this device.",
          )}
        </Text>
        <CameraPreview ref={setVideo} />
        {error && (
          <Alert color="red" role="alert">
            {error}
          </Alert>
        )}
        {error && (
          <Button onClick={() => setAttempt((n) => n + 1)}>
            {t("Try again")}
          </Button>
        )}
      </Stack>
    </Modal>
  );
}
