import { Progress } from "radix-ui";
import { t } from "../i18n";
import type { PageProgress } from "../platform/sync";

/** A run's progress as a small determinate ring (0–100). */
export function PullRing({
  value,
  size = 18,
}: {
  value: number;
  size?: number;
}) {
  const clamped = Math.max(0, Math.min(100, Math.round(value)));
  const thickness = Math.max(2, Math.round(size / 7));
  const radius = (size - thickness) / 2;
  const circumference = 2 * Math.PI * radius;
  // Radix supplies progress semantics; SVG draws the shell's existing ring.
  return (
    <Progress.Root
      value={clamped}
      max={100}
      aria-label={`${clamped}%`}
      className="pullRing"
    >
      <svg
        width={size}
        height={size}
        viewBox={`0 0 ${size} ${size}`}
        aria-hidden="true"
      >
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke="var(--line)"
          strokeWidth={thickness}
        />
        <circle
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke="var(--accent)"
          strokeWidth={thickness}
          strokeLinecap="round"
          strokeDasharray={circumference}
          strokeDashoffset={circumference * (1 - clamped / 100)}
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
        />
      </svg>
    </Progress.Root>
  );
}

/** A page's sync progress: the ring only when Core knows the size, a small
 * spinner otherwise. */
export function SyncRing({
  progress,
  size = 18,
}: {
  progress: NonNullable<PageProgress>;
  size?: number;
}) {
  return progress.total !== null ? (
    <PullRing
      value={(100 * progress.done) / Math.max(progress.total, 1)}
      size={size}
    />
  ) : (
    <i className="schoolStatusSpinner" aria-hidden="true" />
  );
}

/** "4 / 9" for a run of known size, "Updating…" otherwise. */
export const progressLabel = (progress: NonNullable<PageProgress>) =>
  progress.total !== null
    ? `${progress.done} / ${progress.total}`
    : t("Updating…");
