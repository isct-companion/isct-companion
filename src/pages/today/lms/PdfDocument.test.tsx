// @vitest-environment jsdom
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
const { load, destroy, cancel, render } = vi.hoisted(() => ({ load: vi.fn(), destroy: vi.fn(), cancel: vi.fn(), render: vi.fn() }));
vi.mock("pdfjs-dist/legacy/build/pdf.mjs", () => ({ GlobalWorkerOptions: {}, getDocument: load, TextLayer: class { render() { return Promise.resolve(); } cancel() {} } }));
vi.mock("pdfjs-dist/legacy/build/pdf.worker.min.mjs?url", () => ({ default: "worker.mjs" }));
import { PdfDocument } from "./PdfDocument";
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
it("releases the document worker and cancels its active page when the viewer leaves", async () => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const sheet = { getViewport: ({ scale }: { scale: number }) => ({width: 600 * scale, height: 800 * scale}), render, cleanup: vi.fn() };
  render.mockReturnValue({promise: new Promise(() => {}), cancel});
  load.mockReturnValue({promise: Promise.resolve({ numPages: 20, getPage: vi.fn().mockResolvedValue(sheet) }), destroy});
  await act(async () => root.render(<PdfDocument src="asset://localhost/a.pdf" title="Lecture" />));
  expect(host.querySelectorAll("canvas")).toHaveLength(1);
  expect(render).toHaveBeenCalledTimes(1);
  await act(async () => root.unmount());
  expect(cancel).toHaveBeenCalledOnce();
  expect(destroy).toHaveBeenCalledOnce();
  host.remove();
  vi.unstubAllGlobals();
});
it("finds text in later pages and changes zoom without mounting every canvas", async () => {
  vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
  const scroll = vi.fn();
  HTMLElement.prototype.scrollIntoView = scroll;
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  render.mockClear(); render.mockReturnValue({promise: Promise.resolve(), cancel});
  load.mockReturnValue({promise: Promise.resolve({ numPages: 3, getPage: vi.fn(async (number: number) => ({
    getViewport: ({scale}:{scale:number}) => ({width:600*scale,height:800*scale}), render, cleanup:vi.fn(),
    streamTextContent: () => { let done = false; return { getReader: () => ({ read: async () => done ? {done:true} : (done=true,{done:false,value:{items:[{str:number===3?"Java object inheritance":"Introduction"}]}}), releaseLock() {}, cancel:async()=>{} }) }; },
  })) }), destroy});
  await act(async()=>root.render(<PdfDocument src="asset://localhost/b.pdf" title="Lecture"/>));
  expect(host.querySelector("form")).toBeNull();
  await act(async()=>host.querySelector<HTMLButtonElement>('.lmsPdfFindToggle')!.click());
  const search = host.querySelector('input[aria-label="Find in document"]')!;
  await act(async()=>{ Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(search,"inheritance");search.dispatchEvent(new Event("input",{bubbles:true})); });
  await act(async()=>host.querySelector("form")!.dispatchEvent(new Event("submit",{bubbles:true,cancelable:true})));
  expect(scroll).toHaveBeenCalled(); expect(host.querySelector('input[aria-label="Page"]')).toHaveProperty("value","3");
  const zoom=host.querySelector("select")!;
  await act(async()=>{zoom.value="1.5";zoom.dispatchEvent(new Event("change",{bubbles:true}));});
  expect((host.querySelector(".lmsPdfPages") as HTMLElement).style.width).toBe("150%");
  expect(host.querySelectorAll("canvas")).toHaveLength(1);
  vi.useFakeTimers();
  const viewport = host.querySelector(".lmsPdfDocument")!;
  const gesture = new WheelEvent("wheel", {ctrlKey:true,deltaY:-20,bubbles:true,cancelable:true});
  await act(async()=>{viewport.dispatchEvent(gesture);});
  expect(gesture.defaultPrevented).toBe(true);
  expect((host.querySelector(".lmsPdfPages") as HTMLElement).style.transform).toContain("scale");
  await act(async()=>{vi.advanceTimersByTime(150);});
  expect(parseFloat((host.querySelector(".lmsPdfPages") as HTMLElement).style.width)).toBeGreaterThan(150);
  expect((host.querySelector(".lmsPdfPages") as HTMLElement).style.transform).toBe("");
  vi.useRealTimers();
  await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();
});
