import { useEffect, useRef, useState } from "react";
import { Alert, Button, Modal, Progress, Stack, Text } from "@mantine/core";
import type { MatrixScanSnapshot } from "../generated/persistence";
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
  const [snapshot, setSnapshot] = useState<MatrixScanSnapshot>();
  const [error, setError] = useState<string>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    setError(undefined);
    setSnapshot(undefined);
    if (!video) return;
    return openMatrixCamera(
      video,
      (next) => {
        setSnapshot(next);
        if (next.complete) complete.current(next.cells);
      },
      (failure) => setError(failure.message),
    );
  }, [attempt, video]);
  const stable = snapshot?.stableCells ?? 0;
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
            "Keep the whole Matrix in view, including A–J and 1–7. Hold steady while the letters are read.",
          )}
        </Text>
        <CameraPreview ref={setVideo} className="matrixCameraPreview">
          <svg
            viewBox="0 0 1 1"
            preserveAspectRatio="none"
            aria-label={t("Matrix recognition overlay")}
          >
            {snapshot?.quads.map((quad, index) => {
              const cell = snapshot.cells[index];
              const center = quad.reduce(
                (sum, point) => ({
                  x: sum.x + point.x / 4,
                  y: sum.y + point.y / 4,
                }),
                { x: 0, y: 0 },
              );
              return (
                <g
                  key={index}
                  className={
                    cell?.stable ? "matrixCameraStable" : "matrixCameraPending"
                  }
                >
                  <polygon
                    points={quad
                      .map((point) => `${point.x},${point.y}`)
                      .join(" ")}
                  />
                  <text
                    x={center.x}
                    y={center.y}
                    fontSize={Math.max(
                      0.008,
                      Math.abs(quad[1].x - quad[0].x) * 0.42,
                    )}
                  >
                    {cell?.text}
                  </text>
                </g>
              );
            })}
          </svg>
        </CameraPreview>
        {error ? (
          <Alert color="red" role="alert">
            {error}
          </Alert>
        ) : (
          <>
            <Progress
              value={(stable / 70) * 100}
              aria-label={t("Stable Matrix cells")}
            />
            <Text size="sm" role="status">
              {snapshot?.quads.length
                ? t("{{count}}/70 cells stable. Keep holding steady.", {
                    count: stable,
                  })
                : t(
                    "Looking for the Matrix. Include the headers and avoid glare.",
                  )}
            </Text>
          </>
        )}
        {!error && snapshot?.cells.some((cell) => cell.text) && (
          <Button
            variant="light"
            onClick={() => complete.current(snapshot.cells)}
          >
            {t("Use current result and review")}
          </Button>
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
