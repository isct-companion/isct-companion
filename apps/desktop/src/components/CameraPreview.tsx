import { forwardRef, type ReactNode } from "react";
import { t } from "../i18n";
import "./camera-preview.css";

/** Shared preview surface; each reader owns its decoding and overlays. */
export const CameraPreview = forwardRef<
  HTMLVideoElement,
  { children?: ReactNode; className?: string }
>(function CameraPreview({ children, className = "" }, ref) {
  return (
    <div className={`cameraPreview ${className}`}>
      <video
        ref={ref}
        autoPlay
        playsInline
        muted
        aria-label={t("Camera preview")}
      />
      {children}
    </div>
  );
});
