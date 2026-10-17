import { useEffect, useRef, useState } from "react";
import { Alert, Button, Modal, Stack, Text } from "@mantine/core";
import type { MatrixScanOutline } from "../generated/persistence";
import type { Recognition } from "../lib/matrix-ocr";
import { openMatrixCamera } from "../platform/matrix-camera";
import { t } from "../i18n";
import "./matrix-camera.css";
import { CameraPreview } from "./CameraPreview";

export function MatrixCamera({
  onClose,
  onComplete,
}: {
  onClose: () => void;
  onComplete: (cells: Recognition[]) => void;
}) {
  const [video, setVideo] = useState<HTMLVideoElement | null>(null);
  const complete = useRef(onComplete);
  complete.current = onComplete;
  const [outline, setOutline] = useState<MatrixScanOutline>();
  const [error, setError] = useState<string>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    setError(undefined);
    setOutline(undefined);
    if (!video) return;
    return openMatrixCamera(
      video,
      (next) => {
        setOutline(next.bounds ? { bounds: next.bounds } : undefined);
        if (next.complete) complete.current(next.cells);
      },
      (failure) => setError(failure.message),
      setOutline,
    );
  }, [attempt, video]);
  return (
    <Modal
      opened
      onClose={onClose}
      title={t("Scan Matrix with camera")}
      size="xl"
    >
      <Stack gap="sm">
        <Text size="sm">
          {t(
            "Bring your card closer until the Matrix is framed, then hold steady. The scanner closes automatically when ready.",
          )}
        </Text>
        <CameraPreview ref={setVideo} className="matrixCameraPreview">
          <svg
            viewBox="0 0 1 1"
            preserveAspectRatio="none"
            aria-label={t("Matrix recognition overlay")}
          >
            {outline && (
              <polygon
                points={outline.bounds
                  .map((point) => `${point.x},${point.y}`)
                  .join(" ")}
              />
            )}
          </svg>
        </CameraPreview>
        {error ? (
          <Alert color="red" role="alert">
            {error}
          </Alert>
        ) : (
          <Text size="sm" role="status">
            {outline
              ? t("Reading Matrix. Hold steady…")
              : t("Point the camera at the Matrix on your card.")}
          </Text>
        )}
        {error && (
          <Button
            variant="default"
            onClick={() => setAttempt((value) => value + 1)}
          >
            {t("Try again")}
          </Button>
        )}
        <Button variant="default" onClick={onClose}>
          {t("Cancel")}
        </Button>
      </Stack>
    </Modal>
  );
}
