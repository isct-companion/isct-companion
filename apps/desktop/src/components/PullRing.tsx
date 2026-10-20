import { RingProgress } from "@mantine/core";
import { t } from "../i18n";
import type { PageProgress } from "../platform/sync";

/** A run's progress as a small determinate ring (0–100). */
export function PullRing({ value, size = 18 }: { value: number; size?: number }) {
  const clamped = Math.max(0, Math.min(100, Math.round(value)));
  return (
    <RingProgress
      size={size}
      thickness={Math.max(2, Math.round(size / 7))}
      roundCaps
      sections={[{ value: clamped, color: "blue" }]}
      rootColor="var(--line)"
      aria-label={`${clamped}%`}
      className="pullRing"
    />
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
