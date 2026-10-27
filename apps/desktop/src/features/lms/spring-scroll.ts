/**
 * Scrolls an element to `target` along a damped spring instead of the
 * browser's fixed smooth-scroll curve: it moves off quickly and eases in
 * without a visible bounce. Any input from the person (wheel, touch, keys,
 * pointer) stops it at once.
 */
export type SpringScroll = { cancel: () => void };

/** One spring step of `dt` seconds; exposed for tests. */
export function springStep(
  position: number,
  velocity: number,
  target: number,
  dt: number,
  stiffness: number,
  damping = 0.92,
) {
  // Semi-implicit Euler in small substeps stays stable at any frame rate.
  const steps = Math.max(1, Math.ceil(dt / 0.004));
  const h = dt / steps;
  for (let i = 0; i < steps; i++) {
    const acceleration =
      -2 * damping * stiffness * velocity - stiffness * stiffness * (position - target);
    velocity += acceleration * h;
    position += velocity * h;
  }
  return { position, velocity };
}

/** A softer spring for longer distances, so a long jump never looks frantic. */
export function stiffnessFor(distance: number) {
  return 14 * Math.min(1, Math.max(0.55, Math.sqrt(520 / Math.max(1, distance))));
}

export function springScroll(
  host: HTMLElement,
  target: number,
  { onDone }: { onDone?: () => void } = {},
): SpringScroll {
  const max = Math.max(0, host.scrollHeight - host.clientHeight);
  const end = Math.max(0, Math.min(target, max));
  const reduced =
    typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
  let frame = 0;
  let safety = 0;
  let done = false;
  const finish = (snap: boolean) => {
    if (done) return;
    done = true;
    cancelAnimationFrame(frame);
    clearTimeout(safety);
    for (const type of STOPPERS) host.removeEventListener(type, stop);
    if (snap) host.scrollTop = end;
    onDone?.();
  };
  const stop = () => finish(false);
  if (reduced || typeof requestAnimationFrame !== "function") {
    host.scrollTop = end;
    onDone?.();
    return { cancel: () => undefined };
  }
  for (const type of STOPPERS) host.addEventListener(type, stop, { passive: true, once: true });
  const stiffness = stiffnessFor(Math.abs(end - host.scrollTop));
  let position = host.scrollTop;
  let speed = 0;
  let last = performance.now();
  const tick = (now: number) => {
    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;
    ({ position, velocity: speed } = springStep(position, speed, end, dt, stiffness));
    host.scrollTop = position;
    if (Math.abs(position - end) < 0.5 && Math.abs(speed) < 8) return finish(true);
    frame = requestAnimationFrame(tick);
  };
  frame = requestAnimationFrame(tick);
  // Panes that are not painted run no frames: arrive anyway.
  safety = window.setTimeout(() => finish(true), 1600);
  return { cancel: stop };
}

const STOPPERS = ["wheel", "touchstart", "keydown", "pointerdown"] as const;

/** Where sections pull: bands at the outline's edges, as fractions of its height. */
export const GRAVITY = { nearTop: 0.2, beyondTop: 0.15, nearBottom: 0.2 };

/**
 * Where a resting outline settles, given the tops of the sections after the
 * first (`boundaries`). Inside a section nothing pulls. With a boundary near
 * the top (or just above it) the new section brings its start to the top;
 * with one near the bottom the old section brings its end to the bottom. A
 * start near the top wins over any end near the bottom, so a section shorter
 * than the outline shows from its beginning; otherwise the nearer pull wins.
 * `undefined` when nothing pulls.
 */
export function gravityTarget(
  top: number,
  height: number,
  boundaries: number[],
  scrollHeight = Infinity,
): number | undefined {
  const nearest = (pairs: [number, number][]) =>
    pairs.sort((left, right) => left[0] - right[0])[0]?.[1];
  const starts: [number, number][] = [];
  const ends: [number, number][] = [];
  for (const boundary of boundaries) {
    const at = boundary - top;
    if (at >= -GRAVITY.beyondTop * height && at <= GRAVITY.nearTop * height)
      starts.push([Math.abs(at), boundary]);
    else if (at >= (1 - GRAVITY.nearBottom) * height && at <= height)
      ends.push([height - at, boundary - height]);
  }
  let target = nearest(starts) ?? nearest(ends);
  if (target == null) return undefined;
  target = Math.max(0, Math.min(target, scrollHeight - height));
  return Math.abs(target - top) < 2 ? undefined : target;
}
