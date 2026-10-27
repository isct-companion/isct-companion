import { Channel } from "@tauri-apps/api/core";
import { invoke, nativeError } from "./native-error";
import type {
  MatrixCameraSnapshot,
  MatrixCaptureMetrics,
  MatrixFrameTimeline,
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
  return nativeError(
    error,
    t("The camera is unavailable. Close other apps using it and try again."),
  );
}

/** Frames kept in the capture timeline of one session (about 20 s at 30 fps). */
const TIMELINE_LIMIT = 600;
/** Long side of the planes the geometry loop analyses (`WORK_SIDE` in app-ocr). */
const ANALYSIS_SIDE = 960;
/** Long side of the keyframe luma the cells are cut from. */
const KEYFRAME_SIDE = 2200;

/** Rec.601 luma with the native rounding, and R-G + 128 (pink rows are positive). */
function analysisPlanes(pixels: Uint8ClampedArray, count: number): Uint8Array {
  const planes = new Uint8Array(count * 2);
  for (let i = 0, p = 0; i < count; i++, p += 4) {
    const r = pixels[p],
      g = pixels[p + 1];
    planes[i] = (19595 * r + 38470 * g + 7471 * pixels[p + 2] + 32768) >> 16;
    planes[count + i] = Math.min(255, Math.max(0, r - g + 128));
  }
  return planes;
}
function lumaPlane(pixels: Uint8ClampedArray, count: number): Uint8Array {
  const luma = new Uint8Array(count);
  for (let i = 0, p = 0; i < count; i++, p += 4)
    luma[i] =
      (19595 * pixels[p] +
        38470 * pixels[p + 1] +
        7471 * pixels[p + 2] +
        32768) >>
      16;
  return luma;
}

/** Owns camera resources and one in-flight frame. Consensus belongs to core,
 * geometry to the native tracker. Cleanup also handles permission/IPC promises
 * that resolve after unmount. Every analysed frame gets an id and is drawn on
 * `display` together with the outline fitted on that same frame, so the box
 * never lags the picture under it. The timeline pairs each frame's capture,
 * IPC reply and paint (see matrix_track_report). */
export function openMatrixCamera(
  video: HTMLVideoElement,
  display: HTMLCanvasElement,
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
    timeline: [],
  };
  let nextFrameId = 0;
  let lastSequence = -1;
  let keyframeInFlight = false;
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
    // One full-resolution picture per video frame: the source of the analysis
    // planes, of a requested keyframe, and of what is displayed.
    const full = document.createElement("canvas");
    const small = document.createElement("canvas");
    const fullContext = full.getContext("2d", { willReadFrequently: true });
    const smallContext = small.getContext("2d", { willReadFrequently: true });
    const displayContext = display.getContext("2d");
    if (!fullContext || !smallContext || !displayContext)
      throw new Error(t("Cannot read camera frames. Scan an image instead."));
    captureStarted = performance.now();
    let lastTime = -1;
    const hasNewFrame = () =>
      video.readyState >= 2 &&
      video.videoWidth > 0 &&
      video.currentTime !== lastTime;
    /** Draw the analysed picture with the outline fitted on it. */
    const paint = (
      snapshot: MatrixCameraSnapshot,
      entry: MatrixFrameTimeline,
    ) => {
      if (display.width !== full.width || display.height !== full.height) {
        display.width = full.width;
        display.height = full.height;
      }
      displayContext.drawImage(full, 0, 0);
      if (snapshot.bounds && snapshot.boundsFrameId === entry.frameId) {
        displayContext.beginPath();
        snapshot.bounds.forEach((point, index) => {
          const x = point.x * full.width,
            y = point.y * full.height;
          if (index === 0) displayContext.moveTo(x, y);
          else displayContext.lineTo(x, y);
        });
        displayContext.closePath();
        displayContext.lineWidth = Math.max(2, full.width / 400);
        displayContext.strokeStyle = "#ff4545";
        displayContext.stroke();
      }
      entry.paintedMs = performance.now();
      entry.paintedMediaMs = entry.mediaMs;
      entry.liveMediaMs = video.currentTime * 1000;
    };
    const receive = async (
      snapshot: MatrixCameraSnapshot,
      entry?: MatrixFrameTimeline,
    ) => {
      if (disposed) return false;
      if (snapshot.bounds) firstOutlineAt ??= performance.now();
      if (snapshot.phase === "capturing") {
        // An older reply must not replace an outline from a newer frame.
        if (snapshot.sequence < lastSequence) return true;
        lastSequence = snapshot.sequence;
        if (entry) paint(snapshot, entry);
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
      return false;
    };
    /** Core asked for this frame's cells: send its full-resolution luma. */
    const sendKeyframe = (frameId: number, capturedMs: number) => {
      if (keyframeInFlight) return;
      keyframeInFlight = true;
      const { data } = fullContext.getImageData(0, 0, full.width, full.height);
      const luma = lumaPlane(data, full.width * full.height);
      void invoke<MatrixCameraSnapshot>("capture_matrix_keyframe", luma, {
        headers: {
          "x-matrix-session": id,
          "x-matrix-frame": String(frameId),
          "x-matrix-width": String(full.width),
          "x-matrix-height": String(full.height),
          "x-matrix-captured": String(capturedMs),
        },
      })
        .then((snapshot) => receive(snapshot))
        .catch(fail)
        .finally(() => {
          keyframeInFlight = false;
        });
    };
    // One consumer starts with acquisition and independently reports consensus.
    const onProgress = new Channel<MatrixCameraSnapshot>();
    onProgress.onmessage = (snapshot) => {
      void receive(snapshot);
    };
    void invoke<MatrixCameraSnapshot>("process_matrix_camera", {
      sessionId: id,
      onProgress,
    })
      .then(receive)
      .catch(fail);
    const frame = async () => {
      if (disposed) return;
      const started = performance.now();
      try {
        let snapshot: MatrixCameraSnapshot;
        let entry: MatrixFrameTimeline | undefined;
        if (hasNewFrame()) {
          lastTime = video.currentTime;
          const scale = Math.min(
            1,
            KEYFRAME_SIDE / Math.max(video.videoWidth, video.videoHeight),
          );
          const width = Math.round(video.videoWidth * scale);
          const height = Math.round(video.videoHeight * scale);
          if (full.width !== width || full.height !== height) {
            full.width = width;
            full.height = height;
            const reduce = Math.min(1, ANALYSIS_SIDE / Math.max(width, height));
            small.width = Math.round(width * reduce);
            small.height = Math.round(height * reduce);
          }
          fullContext.drawImage(video, 0, 0, width, height);
          const frameId = nextFrameId++;
          const capturedMs = performance.now();
          smallContext.drawImage(full, 0, 0, small.width, small.height);
          const { data } = smallContext.getImageData(
            0,
            0,
            small.width,
            small.height,
          );
          const planes = analysisPlanes(data, small.width * small.height);
          const sentMs = performance.now();
          snapshot = await invoke<MatrixCameraSnapshot>(
            "track_matrix_camera_frame",
            planes,
            {
              headers: {
                "x-matrix-session": id,
                "x-matrix-frame": String(frameId),
                "x-matrix-width": String(small.width),
                "x-matrix-height": String(small.height),
                "x-matrix-captured": String(capturedMs),
              },
            },
          );
          if (disposed) return;
          entry = {
            frameId,
            mediaMs: lastTime * 1000,
            capturedMs,
            sentMs,
            respondedMs: performance.now(),
            sequence: snapshot.sequence,
            boundsFrameId: snapshot.boundsFrameId,
            paintedMs: null,
            paintedMediaMs: null,
            liveMediaMs: null,
          };
          if (capture.timeline.length < TIMELINE_LIMIT)
            capture.timeline.push(entry);
          capture.frames += 1;
          capture.frameMs.push(performance.now() - started);
          if (capture.frameMs.length > 128) capture.frameMs.shift();
          // The full canvas still holds this frame until the next one is drawn.
          if (snapshot.keyframeFrameId === frameId)
            sendKeyframe(frameId, capturedMs);
        } else {
          // Core's acquisition deadline still advances when video stalls.
          snapshot = await invoke<MatrixCameraSnapshot>("poll_matrix_camera", {
            sessionId: id,
          });
        }
        if (!(await receive(snapshot, entry))) return;
        if (hasNewFrame()) {
          timer = setTimeout(() => void frame(), 0);
        } else {
          // The timeout is a lifecycle wakeup, not a fixed capture FPS budget.
          let woken = false;
          const wake = () => {
            // WebKit and WebView2 can queue the video callback at the same time
            // as the fallback timer. Only one may start the next native frame;
            // two concurrent captures would submit out-of-order observations.
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
