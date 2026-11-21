import { Minus, Plus } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { t } from "../../i18n";
import { useReleaseSource } from "./useReleaseSource";
import "./reader.css";

const MIN = 0.25;
const MAX = 8;
const clamp = (value: number) => Math.min(MAX, Math.max(MIN, value));

/**
 * One image reader for course files and past exams: fits the width, scrolls
 * natively like the PDF reader, zooms with pinch, Ctrl-wheel, double-click
 * or the corner buttons around the pointer, and pans by dragging. Written
 * here rather than with a pan-zoom library because those turn the wheel into
 * zoom, and a tall phone scan has to scroll like a page.
 */
export function ImageView({
  src,
  title,
  onBroken,
}: {
  src: string;
  title: string;
  onBroken?: () => void;
}) {
  const stage = useRef<HTMLDivElement>(null);
  const image = useReleaseSource<HTMLImageElement>(src);
  const [natural, setNatural] = useState<{ width: number; height: number }>();
  const [room, setRoom] = useState(0);
  const [zoom, setZoom] = useState(1);
  const zoomRef = useRef(1);
  // Where the pointer was over the image when a zoom started, as a fraction,
  // so the same spot stays under it after the size changes.
  const anchor = useRef<
    { fx: number; fy: number; x: number; y: number } | undefined
  >(undefined);

  useEffect(() => {
    const element = stage.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setRoom(element.clientWidth));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const zoomAt = (value: number, x?: number, y?: number) => {
    const element = stage.current;
    const picture = image.current;
    const next = clamp(value);
    if (!element || !picture || next === zoomRef.current) return;
    const box = element.getBoundingClientRect();
    const bounds = picture.getBoundingClientRect();
    const px = x ?? box.left + box.width / 2;
    const py = y ?? box.top + box.height / 2;
    anchor.current = {
      fx: (px - bounds.left) / bounds.width,
      fy: (py - bounds.top) / bounds.height,
      x: px - box.left,
      y: py - box.top,
    };
    zoomRef.current = next;
    setZoom(next);
  };

  useLayoutEffect(() => {
    const element = stage.current;
    const picture = image.current;
    const point = anchor.current;
    if (!element || !picture || !point) return;
    anchor.current = undefined;
    const box = element.getBoundingClientRect();
    const bounds = picture.getBoundingClientRect();
    element.scrollLeft +=
      bounds.left - box.left + point.fx * bounds.width - point.x;
    element.scrollTop +=
      bounds.top - box.top + point.fy * bounds.height - point.y;
  }, [zoom]);

  // Ctrl-wheel (and trackpad pinch, which Chromium reports that way) and
  // WebKit's gesture events; a plain wheel keeps scrolling.
  useEffect(() => {
    const element = stage.current;
    if (!element) return;
    let base = 1;
    const wheel = (event: WheelEvent) => {
      if (!event.ctrlKey) return;
      event.preventDefault();
      zoomAt(
        zoomRef.current * Math.exp(-event.deltaY * 0.01),
        event.clientX,
        event.clientY,
      );
    };
    const start = (event: Event) => {
      event.preventDefault();
      base = zoomRef.current;
    };
    const change = (event: Event) => {
      event.preventDefault();
      const gesture = event as Event & {
        scale: number;
        clientX: number;
        clientY: number;
      };
      zoomAt(base * gesture.scale, gesture.clientX, gesture.clientY);
    };
    element.addEventListener("wheel", wheel, { passive: false });
    element.addEventListener("gesturestart", start, { passive: false });
    element.addEventListener("gesturechange", change, { passive: false });
    return () => {
      element.removeEventListener("wheel", wheel);
      element.removeEventListener("gesturestart", start);
      element.removeEventListener("gesturechange", change);
    };
  }, []);

  // Dragging pans whatever overflows.
  const drag = useRef<
    { x: number; y: number; left: number; top: number } | undefined
  >(undefined);
  const [dragging, setDragging] = useState(false);

  const fit =
    natural && room ? Math.min(1, Math.max(room - 24, 1) / natural.width) : 1;
  const width = natural ? natural.width * fit * zoom : undefined;

  return (
    <div className="lmsImageViewer">
      <div
        ref={stage}
        className="lmsImageStage"
        data-dragging={dragging || undefined}
        onPointerDown={(event) => {
          const element = stage.current;
          if (event.button !== 0 || !element) return;
          drag.current = {
            x: event.clientX,
            y: event.clientY,
            left: element.scrollLeft,
            top: element.scrollTop,
          };
          element.setPointerCapture(event.pointerId);
          setDragging(true);
        }}
        onPointerMove={(event) => {
          const element = stage.current;
          const from = drag.current;
          if (!element || !from) return;
          element.scrollLeft = from.left - (event.clientX - from.x);
          element.scrollTop = from.top - (event.clientY - from.y);
        }}
        onPointerUp={() => {
          drag.current = undefined;
          setDragging(false);
        }}
        onPointerCancel={() => {
          drag.current = undefined;
          setDragging(false);
        }}
        onDoubleClick={(event) =>
          zoomAt(zoomRef.current > 1 ? 1 : 2, event.clientX, event.clientY)
        }
      >
        <img
          ref={image}
          className="lmsImagePreview"
          src={src}
          alt={title}
          draggable={false}
          style={width ? { width } : undefined}
          onLoad={(event) =>
            setNatural({
              width: event.currentTarget.naturalWidth,
              height: event.currentTarget.naturalHeight,
            })
          }
          onError={onBroken}
        />
      </div>
      <div className="lmsImageZoom">
        <button
          type="button"
          aria-label={t("common.zoomOut")}
          title={t("common.zoomOut")}
          disabled={zoom <= MIN}
          onClick={() => zoomAt(zoomRef.current / 1.25)}
        >
          <Minus size={14} aria-hidden />
        </button>
        <button
          type="button"
          title={t("lms.fitWidth")}
          onClick={() => zoomAt(1)}
        >
          {Math.round(zoom * 100)}%
        </button>
        <button
          type="button"
          aria-label={t("common.zoomIn")}
          title={t("common.zoomIn")}
          disabled={zoom >= MAX}
          onClick={() => zoomAt(zoomRef.current * 1.25)}
        >
          <Plus size={14} aria-hidden />
        </button>
      </div>
    </div>
  );
}
