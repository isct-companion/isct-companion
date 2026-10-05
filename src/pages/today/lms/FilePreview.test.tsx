// @vitest-environment jsdom
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({open:vi.fn(),save:vi.fn()}));
vi.mock("../../../app/native", () => ({desktopBridge:{}}));
vi.mock("./files", () => ({previewKind:()=>"text",openMaterial:mocks.open,downloadMaterial:mocks.save,fileSrc:()=>"asset://localhost/a.txt"}));
vi.mock("../../../components/ExternalLink", () => ({ExternalLink:({href,children}:{href:string;children:React.ReactNode})=><a href={href}>{children}</a>}));
vi.mock("./TextDocument", () => ({ TextDocument: ({value}:{value:string}) => <pre>{value}</pre> }));
import { FilePreview } from "./FilePreview";
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
it("uses temporary preview by default and delegates Download to native Save As", async () => {
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const context={term:"2026-3Q",course:"CSC.T273",section:"section-91"};const url="https://lms.s.isct.ac.jp/2025/pluginfile.php/1/a.txt";
  mocks.open.mockResolvedValueOnce(null).mockResolvedValueOnce({name:"a.txt",path:"/tmp/a.txt",text:"Lecture notes"});mocks.save.mockResolvedValue("/tmp/export.txt");
  await act(async()=>root.render(<FilePreview url={url} title="Notes" context={context}/>));
  expect(mocks.open).toHaveBeenLastCalledWith(context,url,true,true,false);
  expect(host.textContent).toContain("Preview ready");expect(host.querySelector("a")?.href).toBe(url);
  await act(async()=> [...host.querySelectorAll("button")].find(b=>b.textContent==="Download")!.click());
  expect(mocks.save).toHaveBeenCalledWith(context,url);expect(host.textContent).toContain("/tmp/export.txt");
  await act(async()=>root.unmount());host.remove();
});
