import { describe, expect, it } from "vitest";
import { pickLocalized, seedWorkspace } from "./model";

describe("v3 model helpers", () => {
  it("starts real workspaces empty", () => {
    const workspace = seedWorkspace();
    expect(workspace.schemaVersion).toBe(3);
    expect(workspace.courses).toEqual([]);
    expect(workspace.assignments).toEqual([]);
    expect(workspace.planItems).toEqual([]);
  });

  it("picks the locale, falls back, and treats zh as English-first", () => {
    const both = { ja: "日本語", en: "English" };
    expect(pickLocalized(both, "ja")).toBe("日本語");
    expect(pickLocalized(both, "en")).toBe("English");
    expect(pickLocalized(both, "zh")).toBe("English");
    expect(pickLocalized({ ja: "日本語" }, "en")).toBe("日本語");
    expect(pickLocalized({}, "en", "Fallback")).toBe("Fallback");
  });
});
