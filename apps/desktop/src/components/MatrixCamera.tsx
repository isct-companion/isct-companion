import { useEffect, useRef, useState } from "react";
import { Alert, Button, Modal, Progress, Stack, Text } from "@mantine/core";
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
  const [processing, setProcessing] = useState(false);
  const [progress, setProgress] = useState(0);
  const [readyCells, setReadyCells] = useState(0);
  const [error, setError] = useState<string>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    setError(undefined);
    setProcessing(false);
    setProgress(0);
    setReadyCells(0);
    setOutline(undefined);
    if (!video) return;
    return openMatrixCamera(
      video,
      (next) => {
        setOutline(next.bounds ? { bounds: next.bounds } : undefined);
        setReadyCells(next.readyCells);
        setProcessing(next.phase !== "capturing");
        setProgress(
          next.totalSamples ? next.completedSamples / next.totalSamples : 0,
        );
        if (
          next.phase === "complete" &&
          next.cells.every((cell) => !cell.text)
        ) {
          setError(
            t(
              "No readable letters found. Check for blur or covered cells and try again.",
            ),
          );
          return;
        }
        if (next.phase === "complete")
          complete.current(
            next.cells.map((cell) => ({
              text: cell.text,
              confidence: cell.stable
                ? cell.confidence
                : Math.min(cell.confidence, 0.79),
            })),
          );
      },
      (failure) => setError(failure.message),
    );
  }, [attempt, video]);
  // Activity hiding detaches video refs and restarts the capture effect.
  // Keep the preview mounted while native recognition finishes.
  return (
    <>
      <Modal
        opened={!processing}
        keepMounted
        keepMountedMode="display-none"
        onClose={onClose}
        title={t("Scan Matrix with camera")}
        size="xl"
      >
        <Stack gap="sm">
          <Text size="sm">
            {t(
              "Bring your card closer until the Matrix is framed. Capture ends when every cell has enough clear images, or after eight seconds.",
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
                ? t("Capturing Matrix… {{count}}/70 cells ready.", {
                    count: readyCells,
                  })
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
      {processing && (
        <Stack gap="xs" className="matrixCameraProcessing">
          <Text size="sm" role="status">
            {t("Processing Matrix… You can put your card down.")}
          </Text>
          <Progress
            value={progress * 100}
            aria-label={t("Matrix processing progress")}
          />
          {error && (
            <>
              <Alert color="red" role="alert">
                {error}
              </Alert>
              <Button
                variant="default"
                size="compact-xs"
                onClick={() => setAttempt((value) => value + 1)}
              >
                {t("Try again")}
              </Button>
            </>
          )}
          <Button variant="default" size="compact-xs" onClick={onClose}>
            {t("Cancel")}
          </Button>
        </Stack>
      )}
    </>
  );
}
