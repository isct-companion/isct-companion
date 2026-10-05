import { describe, expect, it } from "vitest";
import { COURSE_PALETTE, displayColor } from "./course-colors";

const course = (code: string, color = "#526e91", overrides?: { color?: string }) => ({ id: code, code, color, overrides });

describe("course colours", () => {
  it("keeps a chosen colour", () => {
    expect(displayColor(course("CSC.T262", "#b0713f"))).toBe("#b0713f");
    expect(displayColor(course("CSC.T262", "#526e91", { color: "#806a9d" }))).toBe("#806a9d");
  });

  it("gives imported courses a stable palette colour by code", () => {
    const first = displayColor(course("CSC.T262"));
    expect(COURSE_PALETTE).toContain(first);
    expect(displayColor(course("CSC.T262", "#556b8d"))).toBe(first);
    const colours = new Set(["CSC.T262", "CSC.T273", "CSC.T252", "LAH.T319", "LAW.W221-12"].map((code) => displayColor(course(code))));
    expect(colours.size).toBeGreaterThan(2);
  });
});
