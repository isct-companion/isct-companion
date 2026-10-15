import { useEffect, useRef } from "react";

/** Touches starting this close to the left edge can swipe back. */
const EDGE = 24;
const SETTLE_MS = 200;

/** The screen that moves with the finger: an open dialog, else the page. */
const surface = () =>
  document.querySelector<HTMLElement>(".lectureDesk.open .lectureContent") ??
  document.querySelector<HTMLElement>(".mobileMain");

/**
 * iOS-style back: drag from the left edge and the screen follows the finger;
 * releasing past a third of the width (or with a quick flick) goes back.
 * Android's system back gesture reaches the same history through the router.
 */
export function useEdgeSwipeBack(back: (() => void) | undefined) {
  const backRef = useRef(back);
  backRef.current = back;
  useEffect(() => {
    let start: { x: number; y: number; time: number } | null = null;
    let target: HTMLElement | null = null;
    let dx = 0;
    let dragging = false;
    const reset = (element: HTMLElement | null) => {
      if (!element) return;
      element.style.transition = "";
      element.style.transform = "";
    };
    const onStart = (event: TouchEvent) => {
      const touch = event.touches[0];
      if (
        !backRef.current ||
        event.touches.length !== 1 ||
        touch.clientX > EDGE
      )
        return;
      start = { x: touch.clientX, y: touch.clientY, time: event.timeStamp };
      target = surface();
      dx = 0;
      dragging = false;
    };
    const onMove = (event: TouchEvent) => {
      if (!start || !target) return;
      const touch = event.touches[0];
      const moveX = touch.clientX - start.x;
      const moveY = touch.clientY - start.y;
      if (!dragging) {
        // A vertical scroll wins; only a mostly horizontal drag takes over.
        if (Math.abs(moveY) > 10 && Math.abs(moveY) > moveX) {
          start = null;
          return;
        }
        if (moveX < 10) return;
        dragging = true;
      }
      event.preventDefault();
      dx = Math.max(0, moveX);
      target.style.transition = "none";
      target.style.transform = `translateX(${dx}px)`;
    };
    const onEnd = (event: TouchEvent) => {
      if (!start || !target) return;
      const element = target;
      const velocity = dx / Math.max(1, event.timeStamp - start.time);
      const leave = dragging && (dx > innerWidth / 3 || velocity > 0.6);
      start = null;
      target = null;
      if (!dragging) return;
      element.style.transition = `transform ${SETTLE_MS}ms ease-out`;
      element.style.transform = leave ? `translateX(${innerWidth}px)` : "";
      window.setTimeout(() => {
        if (leave) backRef.current?.();
        requestAnimationFrame(() => reset(element));
      }, SETTLE_MS);
    };
    document.addEventListener("touchstart", onStart, { passive: true });
    document.addEventListener("touchmove", onMove, { passive: false });
    document.addEventListener("touchend", onEnd);
    document.addEventListener("touchcancel", onEnd);
    return () => {
      document.removeEventListener("touchstart", onStart);
      document.removeEventListener("touchmove", onMove);
      document.removeEventListener("touchend", onEnd);
      document.removeEventListener("touchcancel", onEnd);
    };
  }, []);
}
