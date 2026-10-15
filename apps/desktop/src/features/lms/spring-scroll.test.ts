import { describe, expect, it } from "vitest";
import { springStep, stiffnessFor } from "./spring-scroll";

describe("spring scroll", () => {
  it("arrives within about a third of a second without overshooting visibly", () => {
    let state = { position: 0, velocity: 0 };
    let arrivedAt: number | undefined;
    let furthest = 0;
    for (let frame = 1; frame <= 120; frame++) {
      state = springStep(state.position, state.velocity, 400, 1 / 60, stiffnessFor(400));
      furthest = Math.max(furthest, state.position);
      if (arrivedAt == null && Math.abs(state.position - 400) < 0.5 && Math.abs(state.velocity) < 8)
        arrivedAt = frame / 60;
    }
    expect(arrivedAt).toBeGreaterThan(0.2);
    expect(arrivedAt).toBeLessThan(0.6);
    // At most a hair past the target.
    expect(furthest - 400).toBeLessThan(2);
  });

  it("is softer for long jumps", () => {
    expect(stiffnessFor(3000)).toBeLessThan(stiffnessFor(300));
    expect(stiffnessFor(100)).toBe(14);
  });
});
