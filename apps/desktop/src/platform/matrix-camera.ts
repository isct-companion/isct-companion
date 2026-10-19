import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  MatrixCameraSnapshot,
  MatrixCaptureMetrics,
} from "../generated/persistence";
import { t } from "../i18n";

function cameraError(error: unknown): Error {
  if (error instanceof DOMException) {
    if (error.name === "NotAllowedError")
      return new Error(
        t(
          "Camera access was denied. Allow camera access in system settings, then try again.",
        ),
      );
    if (error.name === "NotFoundError")
      return new Error(
        t("No camera found. Connect a camera or scan an image instead."),
      );
    if (error.name === "NotReadableError")
      return new Error(
        t(
          "The camera is unavailable. Close other apps using it and try again.",
        ),
      );
  }
  if (error instanceof Error) return error;
  if (error && typeof error === "object" && "detail" in error)
    return new Error(String(error.detail));
  return new Error(String(error));
}

/** Owns camera resources and one in-flight frame. Consensus belongs to core.
 * Cleanup also handles permission/IPC promises that resolve after unmount. */
export function openMatrixCamera(
  video: HTMLVideoElement,
  onSnapshot: (snapshot: MatrixCameraSnapshot) => void,
  onError: (error: Error) => void,
): () => void {
  let disposed = false;
  let processing = false;
  let cameraReleased = false;
  let stream: MediaStream | undefined;
  let sessionId: string | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let videoFrame: number | undefined;
  let captureStarted: number | undefined;
  let firstOutlineAt: number | undefined;
  const capture: MatrixCaptureMetrics = {
    elapsedMs: 0,
    trackedMs: null,
    frames: 0,
    frameMs: [],
  };
  const stopSession = (id: string) => {
    void invoke("stop_matrix_camera", { sessionId: id, capture }).catch(() => {
      // A shutting-down native host may already have dropped the session.
    });
  };
  const releaseCamera = () => {
    if (cameraReleased) return;
    cameraReleased = true;
    clearTimeout(timer);
    if (videoFrame !== undefined) video.cancelVideoFrameCallback(videoFrame);
    stream?.getTracks().forEach((track) => track.stop());
    video.srcObject = null;
    capture.elapsedMs =
      captureStarted === undefined ? 0 : performance.now() - captureStarted;
    capture.trackedMs =
      firstOutlineAt === undefined ? null : performance.now() - firstOutlineAt;
  };
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    releaseCamera();
    document.removeEventListener("visibilitychange", visibility);
    if (sessionId) stopSession(sessionId);
  };
  const fail = (error: unknown) => {
    if (disposed) return;
    dispose();
    onError(cameraError(error));
  };
  const visibility = () => {
    if (document.hidden && !processing)
      fail(
        new Error(
          t(
            "Camera scan stopped while the app was in the background. Open it again to continue.",
          ),
        ),
      );
  };
  document.addEventListener("visibilitychange", visibility);

  void (async () => {
    if (!navigator.mediaDevices?.getUserMedia)
      throw new Error(
        t("Camera access is unavailable here. Scan an image instead."),
      );
    const id = await invoke<string>("begin_matrix_camera");
    if (disposed) {
      stopSession(id);
      return;
    }
    sessionId = id;
    const acquired = await navigator.mediaDevices.getUserMedia({
      audio: false,
      video: {
        width: { ideal: 1920 },
        height: { ideal: 1080 },
        frameRate: { ideal: 30, max: 30 },
      },
    });
    if (disposed) {
      acquired.getTracks().forEach((track) => track.stop());
      return;
    }
    stream = acquired;
    for (const track of stream.getVideoTracks())
      track.addEventListener(
        "ended",
        () =>
          fail(
            new Error(t("The camera disconnected. Open it again to continue.")),
          ),
        { once: true },
      );
    video.srcObject = stream;
    await video.play();
    if (disposed) return;
    const canvas = document.createElement("canvas");
    const context = canvas.getContext("2d");
    if (!context)
      throw new Error(t("Cannot read camera frames. Scan an image instead."));
    captureStarted = performance.now();
    let lastTime = -1;
    const hasNewFrame = () =>
      video.readyState >= 2 &&
      video.videoWidth > 0 &&
      video.currentTime !== lastTime;
    const receive = async (snapshot: MatrixCameraSnapshot) => {
      if (disposed) return false;
      if (snapshot.bounds) firstOutlineAt ??= performance.now();
      if (snapshot.phase === "capturing") {
        onSnapshot(snapshot);
        return true;
      }
      processing = true;
      releaseCamera();
      if (snapshot.phase === "complete") {
        dispose();
        onSnapshot(snapshot);
        return false;
      }
      onSnapshot(snapshot);
      const onProgress = new Channel<MatrixCameraSnapshot>();
      onProgress.onmessage = (progress) => {
        // Complete through the command result, after its worker releases state.
        if (!disposed && progress.phase === "processing") onSnapshot(progress);
      };
      const result = await invoke<MatrixCameraSnapshot>(
        "process_matrix_camera",
        {
          sessionId: id,
          onProgress,
        },
      );
      if (!disposed) {
        dispose();
        onSnapshot(result);
      }
      return false;
    };
    const frame = async () => {
      if (disposed) return;
      const started = performance.now();
      try {
        let snapshot: MatrixCameraSnapshot;
        if (hasNewFrame()) {
          lastTime = video.currentTime;
          const scale = Math.min(
            1,
            2200 / Math.max(video.videoWidth, video.videoHeight),
          );
          canvas.width = Math.round(video.videoWidth * scale);
          canvas.height = Math.round(video.videoHeight * scale);
          context.drawImage(video, 0, 0, canvas.width, canvas.height);
          const image = canvas.toDataURL("image/jpeg", 0.92).split(",")[1];
          snapshot = await invoke<MatrixCameraSnapshot>(
            "capture_matrix_camera_frame",
            {
              sessionId: id,
              image,
            },
          );
          if (disposed) return;
          capture.frames += 1;
          capture.frameMs.push(performance.now() - started);
          if (capture.frameMs.length > 128) capture.frameMs.shift();
        } else {
          // Core's acquisition deadline still advances when video stalls.
          snapshot = await invoke<MatrixCameraSnapshot>("poll_matrix_camera", {
            sessionId: id,
          });
        }
        if (!(await receive(snapshot))) return;
        if (hasNewFrame()) {
          timer = setTimeout(() => void frame(), 0);
        } else {
          // The timeout is a lifecycle wakeup, not a fixed capture FPS budget.
          let woken = false;
          const wake = () => {
            // WebKit and WebView2 can queue the video callback at the same time
            // as the fallback timer. Only one may start the next native frame;
            // two concurrent captures can both seal the session and start two
            // processing loops for the same Matrix.
            if (woken || disposed) return;
            woken = true;
            clearTimeout(timer);
            if (videoFrame !== undefined)
              video.cancelVideoFrameCallback(videoFrame);
            videoFrame = undefined;
            void frame();
          };
          timer = setTimeout(wake, 100);
          if (typeof video.requestVideoFrameCallback === "function")
            videoFrame = video.requestVideoFrameCallback(wake);
        }
      } catch (error) {
        fail(error);
      }
    };
    void frame();
  })().catch(fail);
  return dispose;
}
