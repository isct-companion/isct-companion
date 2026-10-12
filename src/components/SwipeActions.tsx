import { useRef, useState, type ReactNode } from "react";
import { UiButton } from "./mantine";
import { t } from "../i18n";
import "./swipe-actions.css";

type Action = { label: string; run: () => void };
/** Direction locking is needed here: useMove tracks absolute positions and
 * would consume vertical list scrolling. Pointer events also provide cancellation. */
export function SwipeActions({
  children,
  leading,
  trailing,
}: {
  children: ReactNode;
  leading?: Action;
  trailing: Action;
}) {
  const start = useRef<{
    x: number;
    y: number;
    id: number;
    axis?: "x" | "y";
  } | null>(null);
  const moved = useRef(false);
  const distance = useRef(0);
  const [offset, setOffset] = useState(0);
  const [dragging, setDragging] = useState(false);
  const close = () => {
    setOffset(0);
    distance.current = 0;
  };
  return (
    <div
      className={`swipeActions${dragging ? " dragging" : ""}`}
      onPointerDown={(event) => {
        if (!event.isPrimary) {
          start.current = null;
          setDragging(false);
          close();
          return;
        }
        if (
          event.pointerType === "mouse" ||
          event.clientX <= 24 ||
          (event.target as Element).closest("[data-swipe-action]")
        )
          return;
        moved.current = false;
        start.current = {
          x: event.clientX,
          y: event.clientY,
          id: event.pointerId,
        };
      }}
      onPointerMove={(event) => {
        const origin = start.current;
        if (!origin || origin.id !== event.pointerId || origin.axis === "y")
          return;
        const dx = event.clientX - origin.x,
          dy = event.clientY - origin.y;
        if (!origin.axis) {
          if (Math.max(Math.abs(dx), Math.abs(dy)) < 10) return;
          origin.axis = Math.abs(dx) > Math.abs(dy) * 1.3 ? "x" : "y";
          if (origin.axis === "y") return;
          event.currentTarget.setPointerCapture?.(event.pointerId);
        }
        moved.current = true;
        setDragging(true);
        distance.current = Math.max(-88, Math.min(leading ? 88 : 0, dx));
        setOffset(distance.current);
      }}
      onPointerUp={(event) => {
        if (start.current?.id !== event.pointerId) return;
        start.current = null;
        setDragging(false);
        const next =
          Math.abs(distance.current) >= 48
            ? Math.sign(distance.current) * 88
            : 0;
        setOffset(next);
        distance.current = next;
        if (event.currentTarget.hasPointerCapture?.(event.pointerId))
          event.currentTarget.releasePointerCapture(event.pointerId);
      }}
      onPointerCancel={() => {
        start.current = null;
        moved.current = true;
        setDragging(false);
        close();
      }}
      onClickCapture={(event) => {
        if ((event.target as Element).closest("[data-swipe-action]")) return;
        if (moved.current || offset !== 0) {
          event.preventDefault();
          event.stopPropagation();
          if (!moved.current) close();
          moved.current = false;
        }
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") close();
      }}
    >
      {leading && (
        <UiButton
          data-swipe-action
          className="swipeAction leading"
          tabIndex={offset > 0 ? 0 : -1}
          aria-hidden={offset <= 0}
          onClick={() => {
            leading.run();
            close();
          }}
        >
          {t(leading.label)}
        </UiButton>
      )}
      <UiButton
        data-swipe-action
        className="swipeAction trailing"
        tabIndex={offset < 0 ? 0 : -1}
        aria-hidden={offset >= 0}
        onClick={() => {
          trailing.run();
          close();
        }}
      >
        {t(trailing.label)}
      </UiButton>
      <div
        className="swipeContent"
        style={{ transform: `translateX(${offset}px)` }}
      >
        {children}
      </div>
    </div>
  );
}
