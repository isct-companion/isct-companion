// @vitest-environment jsdom
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { SyllabusDetails } from "./syllabus-details";
import type { SyllabusCourse } from "../../core/syllabus";
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
it("resolves an official offering-ID locator and renders the English canonical syllabus", async () => {
  vi.useFakeTimers();
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const url = "https://syllabus.s.isct.ac.jp/courses/-/-/-/202600001?hl=en";
  const fetch = vi.fn(async () => ({url,content:{schemaVersion:1,sourceUrl:url,sourceLanguage:"en",fetchedAt:"2026-10-05",sourceUpdated:null,metadata:[],sections:[{key:"overview",heading:"Overview",blocks:[{kind:"paragraph",text:"English course objectives"}],links:[]}],lecturePlan:[],competencies:[]}} as unknown as SyllabusCourse));
  await act(async () => root.render(<SyllabusDetails url="https://syllabus.s.isct.ac.jp/courses/-/-/-/202600001" fetchCourse={fetch} />));
  await act(async () => vi.advanceTimersByTimeAsync(400));
  expect(fetch).toHaveBeenCalledOnce(); expect(host.textContent).toContain("English course objectives");
  await act(async () => root.unmount()); host.remove(); vi.useRealTimers();
});
