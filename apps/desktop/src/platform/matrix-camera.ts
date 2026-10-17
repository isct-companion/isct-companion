import { invoke } from "@tauri-apps/api/core";
import type { MatrixScanSnapshot } from "../generated/persistence";
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
  onSnapshot: (snapshot: MatrixScanSnapshot) => void,
  onError: (error: Error) => void,
): () => void {
  let disposed = false;
  let stream: MediaStream | undefined;
  let sessionId: string | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const stopSession = (id: string) => {
    void invoke("stop_matrix_camera", { sessionId: id }).catch(() => {
      // A shutting-down native host may already have dropped the session.
    });
  };
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    clearTimeout(timer);
    document.removeEventListener("visibilitychange", visibility);
    stream?.getTracks().forEach((track) => track.stop());
    video.srcObject = null;
    if (sessionId) stopSession(sessionId);
  };
  const fail = (error: unknown) => {
    if (disposed) return;
    dispose();
    onError(cameraError(error));
  };
  const visibility = () => {
    if (document.hidden)
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
      video: { width: { ideal: 1920 }, height: { ideal: 1080 } },
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
    let lastTime = -1;
    const frame = async () => {
      if (disposed) return;
      try {
        if (
          video.readyState >= 2 &&
          video.videoWidth &&
          video.currentTime !== lastTime
        ) {
          lastTime = video.currentTime;
          const scale = Math.min(
            1,
            2200 / Math.max(video.videoWidth, video.videoHeight),
          );
          canvas.width = Math.round(video.videoWidth * scale);
          canvas.height = Math.round(video.videoHeight * scale);
          context.drawImage(video, 0, 0, canvas.width, canvas.height);
          const image = canvas.toDataURL("image/jpeg", 0.92).split(",")[1];
          const snapshot = await invoke<MatrixScanSnapshot>(
            "scan_matrix_camera_frame",
            { sessionId: id, image },
          );
          if (disposed) return;
          if (snapshot.complete) dispose();
          onSnapshot(snapshot);
          if (snapshot.complete) return;
        }
        // Capture the next fresh frame only after the previous worker finishes.
        timer = setTimeout(() => void frame(), 250);
      } catch (error) {
        fail(error);
      }
    };
    void frame();
  })().catch(fail);
  return dispose;
}
