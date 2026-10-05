import { RingProgress } from "@mantine/core";

/** A pull's progress as a small determinate ring (0–100). */
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
