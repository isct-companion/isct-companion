import { useEffect, useRef, useState } from "react";
import { Button, ErrorBanner, Modal, Progress, Stack, Text } from "./ui";
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
  const [display, setDisplay] = useState<HTMLCanvasElement | null>(null);
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
    if (!video || !display) return;
    return openMatrixCamera(
      video,
      display,
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
          setError(t("matrixCamera.noReadableLettersFoundCheck"));
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
  }, [attempt, video, display]);
  // Activity hiding detaches video refs and restarts the capture effect.
  // Keep the preview mounted while native recognition finishes.
  return (
    <>
      <Modal
        opened={!processing}
        keepMounted
        keepMountedMode="display-none"
        onClose={onClose}
        title={t("matrixCamera.scanMatrixWithCamera")}
        size="xl"
      >
        <Stack gap="sm">
          <Text size="sm">
            {t("settings.matrixCamera.keepTheMatrixFramedAnd")}
          </Text>
          <CameraPreview ref={setVideo} className="matrixCameraPreview">
            {/* Each analysed frame is drawn here with the outline fitted on
                it; the live video shows through until the first one. */}
            <canvas
              ref={setDisplay}
              aria-label={t("matrixCamera.matrixRecognitionOverlay")}
            />
          </CameraPreview>
          {error ? (
            <ErrorBanner>{error}</ErrorBanner>
          ) : (
            <Text size="sm" role="status">
              {outline
                ? t("settings.matrixCamera.recognizingMatrixCount70Cells", {
                    count: readyCells,
                  })
                : t("matrixCamera.pointTheCameraAtThe")}
            </Text>
          )}
          {error && (
            <Button
              variant="default"
              onClick={() => setAttempt((value) => value + 1)}
            >
              {t("common.tryAgain")}
            </Button>
          )}
          <Button variant="default" onClick={onClose}>
            {t("common.cancel")}
          </Button>
        </Stack>
      </Modal>
      {processing && (
        <Stack gap="xs" className="matrixCameraProcessing">
          <Text size="sm" role="status">
            {t("matrixCamera.processingMatrixYouCanPut")}
          </Text>
          <Progress
            value={progress * 100}
            aria-label={t("matrixCamera.matrixProcessingProgress")}
          />
          {error && (
            <ErrorBanner onRetry={() => setAttempt((value) => value + 1)}>
              {error}
            </ErrorBanner>
          )}
          <Button variant="default" size="compact-xs" onClick={onClose}>
            {t("common.cancel")}
          </Button>
        </Stack>
      )}
    </>
  );
}
