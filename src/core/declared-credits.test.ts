import { describe, expect, it } from "vitest";
import { declaredTermCredits } from "./declared-credits";

const row = (offeringId: string, quarters: number[], effectiveCredits: number) => ({
  kind: "declared",
  offeringId,
  quarters,
  effectiveCredits,
});

describe("declared credits per quarter", () => {
  const rows = [
    row("202636840", [3], 2),
    row("202608712", [3], 1),
    row("202637648", [3, 4], 2),
    row("202600702", [3], 0.5),
    row("202600722", [4], 0.5),
    row("202602431", [3], 2),
    { kind: "declared", offeringId: "202602427", quarters: [3], effectiveCredits: 2, deletedAt: "x" },
  ];

  it("sums what 教務Web counts, sharing multi-quarter courses", () => {
    expect(declaredTermCredits(rows, "2026-3Q")).toBe(6.5);
    expect(declaredTermCredits(rows, "2026-4Q")).toBe(1.5);
  });

  it("has no answer for a year 教務Web did not list", () => {
    expect(declaredTermCredits(rows, "2025-3Q")).toBeUndefined();
    expect(declaredTermCredits([{ kind: "registration", offeringId: "202602431" }], "2026-3Q")).toBeUndefined();
  });
});
