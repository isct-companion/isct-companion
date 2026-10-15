// @vitest-environment jsdom
import React, {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import {YearQuarterPicker} from "./YearQuarterPicker";
import type {TermId} from "../../lib/model";
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
it("keeps one year open without changing the timetable until a quarter is selected", async () => {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); const select = vi.fn();
  const terms = ["2025-1Q","2025-2Q","2025-3Q","2025-4Q","2026-1Q","2026-2Q","2026-3Q","2026-4Q"] as TermId[];
  await act(async()=>root.render(<YearQuarterPicker terms={terms} selected="2026-3Q" current="2026-3Q" onSelect={select}/>));
  expect(host.querySelectorAll(".yearQuarterGroup.expanded")).toHaveLength(1);
  expect(host.querySelector(".yearQuarterGroup.expanded .yearQuarterYear")?.textContent).toBe("2026");
  const past = host.querySelector(".yearQuarterGroup")!;
  const hover = new Event("pointerover", {bubbles:true}); Object.defineProperty(hover,"pointerType",{value:"mouse"});
  await act(async()=>past.dispatchEvent(hover));
  expect(host.querySelectorAll(".yearQuarterGroup.expanded")).toHaveLength(1);
  expect(past.classList.contains("expanded")).toBe(true);
  expect(select).not.toHaveBeenCalled();
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="2025 2Q"]')!.click());
  expect(select).toHaveBeenCalledWith("2025-2Q");
  await act(async()=>root.render(<YearQuarterPicker terms={terms} selected="2026-1Q" current="2026-3Q" onSelect={select}/>));
  expect(host.querySelector(".yearQuarterGroup.expanded .yearQuarterYear")?.textContent).toBe("2026");
  await act(async()=>root.unmount()); host.remove();
});
