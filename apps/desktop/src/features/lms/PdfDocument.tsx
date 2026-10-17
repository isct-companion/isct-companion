import { UiSelect } from "../../components/mantine";
import { ReaderTools } from "./ReaderPane";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { getDocument, GlobalWorkerOptions, TextLayer, type PDFDocumentProxy } from "pdfjs-dist/legacy/build/pdf.mjs";
import workerUrl from "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url";
import "pdfjs-dist/web/pdf_viewer.css";
import { t } from "../../i18n";
import { LmsIcon } from "./LmsIcon";
GlobalWorkerOptions.workerSrc = workerUrl;
// Canvas budget per page: sharp on Retina without unbounded memory.
const MAX_DPR = 2;
const MAX_PIXELS = 4_000_000;
function PdfPage({ pdf, number, width, ratio }: { pdf: PDFDocumentProxy; number: number; width: number; ratio: number }) {
  const host = useRef<HTMLDivElement>(null);
  const paper = useRef<HTMLDivElement>(null);
  const text = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(number === 1);
  const [error, setError] = useState("");
  useEffect(() => {
    if (typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(entries => setVisible(entries[0].isIntersecting), { rootMargin: "200px" });
    if (host.current) observer.observe(host.current);
    return () => observer.disconnect();
  }, []);
  // Leaving the viewport releases the decoded bitmap.
  useEffect(() => {
    if (visible) return;
    paper.current?.querySelectorAll("canvas").forEach(canvas => { canvas.width = 0; canvas.height = 0; });
    paper.current?.replaceChildren();
    text.current?.replaceChildren();
  }, [visible]);
  useEffect(() => {
    if (!visible) return;
    let disposed = false;
    let render: ReturnType<Awaited<ReturnType<PDFDocumentProxy["getPage"]>>["render"]> | undefined;
    let layer: TextLayer | undefined;
    // Draw off-screen and swap in when done, so resizing never flashes blank.
    const canvas = document.createElement("canvas");
    canvas.setAttribute("role", "img");
    canvas.setAttribute("aria-label", `Page ${number}`);
    void pdf.getPage(number).then(async sheet => {
      if (disposed) return;
      const initial = sheet.getViewport({ scale: 1 });
      const scale = Math.min(width * Math.min(window.devicePixelRatio || 1, MAX_DPR) / initial.width, Math.sqrt(MAX_PIXELS / (initial.width * initial.height)));
      canvas.width = Math.floor(initial.width * scale); canvas.height = Math.floor(initial.height * scale);
      render = sheet.render({ canvas, viewport: sheet.getViewport({ scale }) });
      try {
        await render.promise;
        if (disposed || !paper.current) return;
        paper.current.querySelectorAll("canvas").forEach(old => { old.width = 0; old.height = 0; });
        paper.current.replaceChildren(canvas);
        if (text.current) {
          const textScale = width / initial.width;
          const next = document.createElement("div");
          text.current.style.setProperty("--total-scale-factor", String(textScale));
          layer = new TextLayer({ textContentSource: sheet.streamTextContent(), container: next, viewport: sheet.getViewport({ scale: textScale }) });
          await layer.render();
          if (!disposed) text.current.replaceChildren(...next.childNodes);
        }
      } finally { sheet.cleanup(); }
    }).catch(e => { if (!disposed && e?.name !== "RenderingCancelledException") setError(String(e)); });
    return () => {
      disposed = true; render?.cancel(); layer?.cancel();
      if (!canvas.isConnected) { canvas.width = 0; canvas.height = 0; }
    };
  }, [pdf, number, width, visible]);
  return <div className="lmsPdfPage" ref={host} style={{ aspectRatio: `${1 / ratio}` }} data-pdf-page={number}>
    {error ? <p role="alert">{error}</p> : <><div ref={paper} className="lmsPdfPaper" /><div ref={text} className="textLayer" /></>}
  </div>;
}
/** Continuous reading; only pages near the viewport own decoded canvases. */
export function PdfDocument({ src, title, fileToolbar }: { src: string; title: string; fileToolbar?: ReactNode }) {
  const host = useRef<HTMLDivElement>(null);
  const [pdf, setPdf] = useState<PDFDocumentProxy>();
  const [width, setWidth] = useState(600);
  // Height/width of each page; mixed portrait/landscape decks are common.
  const [ratios, setRatios] = useState<number[]>([]);
  const [error, setError] = useState("");
  const [zoom, setZoom] = useState(1);
  const [page, setPage] = useState(1);
  const [pageInput, setPageInput] = useState("1");
  const [search, setSearch] = useState("");
  const [searchOpen, setSearchOpen] = useState(false);
  const zoomValue = useRef(1);
  zoomValue.current = zoom;
  const [finding, setFinding] = useState(false);
  const [searchStatus, setSearchStatus] = useState("");
  const searchTask = useRef(0);
  useEffect(() => () => { searchTask.current++; }, [src]);
  useEffect(() => {
    let disposed = false;
    const task = getDocument({ url: src, disableAutoFetch: true });
    void task.promise.then(async doc => {
      const ratioOf = async (number: number) => { const size = (await doc.getPage(number)).getViewport({ scale: 1 }); return size.height / size.width; };
      const first = await ratioOf(1);
      if (disposed) return;
      const known = Array<number>(doc.numPages).fill(first);
      setRatios(known); setPdf(doc);
      // Page sizes only need each page's dictionary, not its content.
      for (let number = 2; number <= doc.numPages && !disposed; number++) {
        known[number - 1] = await ratioOf(number);
        if (number % 20 === 0 || number === doc.numPages) if (!disposed) setRatios([...known]);
      }
    }).catch(e => { if (!disposed) setError(String(e)); });
    return () => { disposed = true; void task.destroy(); };
  }, [src]);
  useEffect(() => {
    // First size applies at once; later ones settle before pages re-render.
    let timer: ReturnType<typeof setTimeout> | undefined;
    let measured = false;
    const observer = new ResizeObserver(entries => {
      const next = Math.max(200, Math.round(entries[0].contentRect.width));
      clearTimeout(timer);
      if (!measured) { measured = true; setWidth(next); } else timer = setTimeout(() => setWidth(next), 150);
    });
    if (host.current) observer.observe(host.current);
    return () => { clearTimeout(timer); observer.disconnect(); };
  }, []);
  useEffect(() => {
    if (!pdf || !host.current || typeof IntersectionObserver === "undefined") return;
    const shown = new Map<number, number>();
    const observer = new IntersectionObserver(entries => {
      for (const entry of entries) {
        const number = Number((entry.target as HTMLElement).dataset.pdfPage);
        if (entry.isIntersecting) shown.set(number, entry.intersectionRatio); else shown.delete(number);
      }
      const best = [...shown].sort((a, b) => b[1] - a[1])[0];
      if (best) { setPage(best[0]); setPageInput(String(best[0])); }
    }, { threshold: [0, .25, .5, .75, 1] });
    host.current.querySelectorAll("[data-pdf-page]").forEach(element => observer.observe(element));
    return () => observer.disconnect();
  }, [pdf]);
  useEffect(() => {
    const viewport = host.current;
    if (!viewport) return;
    let base = zoomValue.current, pending = base, gesturing = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let touchDistance = 0;
    const pages = () => viewport.querySelector<HTMLElement>(".lmsPdfPages");
    const commit = () => {
      clearTimeout(timer);
      const surface = pages();
      if (surface) { surface.style.transform = ""; surface.style.transformOrigin = ""; }
      zoomValue.current = pending;
      setZoom(pending);
      base = pending;
    };
    const preview = (value: number, x: number, y: number) => {
      pending = Math.max(.5, Math.min(3, value));
      const surface = pages();
      if (surface) {
        if (!surface.style.transform) {
          const bounds = surface.getBoundingClientRect();
          surface.style.transformOrigin = `${x - bounds.left}px ${y - bounds.top}px`;
        }
        surface.style.transform = `scale(${pending / base})`;
      }
      clearTimeout(timer); timer = setTimeout(commit, 140);
    };
    const start = (event: Event) => { event.preventDefault(); gesturing = true; base = zoomValue.current; clearTimeout(timer); };
    const change = (event: Event) => {
      event.preventDefault();
      const gesture = event as Event & {scale:number;clientX:number;clientY:number};
      preview(base * gesture.scale, gesture.clientX, gesture.clientY);
      // Keep the original gesture scale until its end; no canvas redraw per frame.
      clearTimeout(timer);
    };
    const end = (event: Event) => { event.preventDefault(); gesturing = false; commit(); };
    const wheel = (event: WheelEvent) => {
      if (!event.ctrlKey || gesturing) return;
      event.preventDefault();
      preview(pending * Math.exp(-event.deltaY * .01), event.clientX, event.clientY);
    };
    const distance = (touches: TouchList) => Math.hypot(touches[0].clientX - touches[1].clientX, touches[0].clientY - touches[1].clientY);
    const touchStart = (event: TouchEvent) => { if (event.touches.length === 2 && !gesturing) { base = zoomValue.current; touchDistance = distance(event.touches); } };
    const touchMove = (event: TouchEvent) => {
      if (event.touches.length !== 2 || !touchDistance || gesturing) return;
      event.preventDefault();
      preview(base * distance(event.touches) / touchDistance, (event.touches[0].clientX + event.touches[1].clientX) / 2, (event.touches[0].clientY + event.touches[1].clientY) / 2);
      clearTimeout(timer);
    };
    const touchEnd = () => { if (touchDistance && !gesturing) { touchDistance = 0; commit(); } };
    viewport.addEventListener("gesturestart", start, {passive:false});
    viewport.addEventListener("gesturechange", change, {passive:false});
    viewport.addEventListener("gestureend", end, {passive:false});
    viewport.addEventListener("wheel", wheel, {passive:false});
    viewport.addEventListener("touchstart", touchStart, {passive:true});
    viewport.addEventListener("touchmove", touchMove, {passive:false});
    viewport.addEventListener("touchend", touchEnd);
    viewport.addEventListener("touchcancel", touchEnd);
    return () => {
      clearTimeout(timer);
      viewport.removeEventListener("gesturestart", start); viewport.removeEventListener("gesturechange", change); viewport.removeEventListener("gestureend", end);
      viewport.removeEventListener("wheel", wheel); viewport.removeEventListener("touchstart", touchStart); viewport.removeEventListener("touchmove", touchMove); viewport.removeEventListener("touchend", touchEnd); viewport.removeEventListener("touchcancel", touchEnd);
    };
  }, [zoom]);
  function goToPage() {
    const number = Math.max(1, Math.min(pdf?.numPages || 1, Number(pageInput) || page));
    host.current?.querySelector(`[data-pdf-page="${number}"]`)?.scrollIntoView({ block: "start" });
    setPage(number); setPageInput(String(number));
  }
  async function findNext() {
    const term = search.trim().toLocaleLowerCase();
    if (!term || !pdf) return;
    const task = ++searchTask.current;
    setFinding(true); setSearchStatus("");
    try {
      for (let offset = 0; offset < pdf.numPages; offset++) {
        const number = ((page - 1 + offset + (searchStatus ? 1 : 0)) % pdf.numPages) + 1;
        const sheet = await pdf.getPage(number);
        // WKWebView supports stream readers but not ReadableStream async iteration.
        const reader = sheet.streamTextContent().getReader();
        const words: string[] = [];
        try {
          for (;;) {
            const chunk = await reader.read();
            if (task !== searchTask.current) { await reader.cancel(); return; }
            if (chunk.done) break;
            words.push(...chunk.value.items.map((item: {str?: string}) => item.str || ""));
          }
        } finally { reader.releaseLock(); }
        const text = words.join(" ").toLocaleLowerCase();
        if (text.includes(term)) {
          host.current?.querySelector(`[data-pdf-page="${number}"]`)?.scrollIntoView({ block: "start" });
          setPage(number); setPageInput(String(number)); setSearchStatus(`${t("Page")} ${number}`);
          return;
        }
      }
      setSearchStatus(t("No matches"));
    } catch (error) { if (task === searchTask.current) setSearchStatus(`${t("Search unavailable")} · ${String(error)}`); }
    finally { if (task === searchTask.current) setFinding(false); }
  }
  return <div className="lmsPdfReader">
    <ReaderTools><div className="lmsPdfHeader">
    <div className="lmsPdfControls" role="toolbar" aria-label={t("PDF tools")}>
      <div className="lmsPdfPagination"><input aria-label={t("Page")} type="number" min="1" max={pdf?.numPages || 1} value={pageInput} onChange={e => setPageInput(e.target.value)} onBlur={goToPage} onKeyDown={e => { if (e.key === "Enter") goToPage(); }} /><span>/ {pdf?.numPages || "—"}</span></div>
      <div className="lmsPdfZoom"><button aria-label={t("Zoom out")} title={t("Zoom out")} disabled={zoom <= .5} onClick={() => setZoom(value => Math.max(.5, value - .25))}><LmsIcon name="minus" size={14} /></button><UiSelect aria-label={t("Zoom")} value={zoom} onChange={value => setZoom(Number(value))} data={[...new Set([.5,.75,1,1.25,1.5,2,3,zoom])].sort((a,b)=>a-b).map(value => ({value:String(value),label:value === 1 ? t("Fit width") : `${Math.round(value * 100)}%`}))} /><button aria-label={t("Zoom in")} title={t("Zoom in")} disabled={zoom >= 3} onClick={() => setZoom(value => Math.min(3, value + .25))}><LmsIcon name="plus" size={14} /></button></div>
      <button className="lmsPdfFindToggle" aria-label={t("Find in document")} title={t("Find in document")} aria-expanded={searchOpen} onClick={() => setSearchOpen(value => !value)}><LmsIcon name="search" size={15} /></button>
    </div>
    {fileToolbar}
    </div>
    </ReaderTools>
    {searchOpen && <form className="lmsPdfSearch" role="search" onSubmit={event => { event.preventDefault(); void findNext(); }}><input aria-label={t("Find in document")} placeholder={t("Find in document")} value={search} onChange={e => { searchTask.current++; setFinding(false); setSearch(e.target.value); setSearchStatus(""); }} /><span role="status">{searchStatus}</span><button disabled={finding || !pdf || !search.trim()}>{t(finding ? "Searching…" : "Find next")}</button><button type="button" aria-label={t("Close search")} onClick={() => setSearchOpen(false)}>×</button></form>}
    <div className="lmsPdfDocument" ref={host} aria-label={title}>
      <div className="lmsPdfPages" style={{ width: `${zoom * 100}%` }}>
        {error ? <p role="alert">{error}</p> : pdf ? Array.from({length: pdf.numPages}, (_, i) => <PdfPage key={i} pdf={pdf} number={i + 1} width={width * zoom} ratio={ratios[i] ?? ratios[0]} />) : <p role="status">{t("Loading PDF…")}</p>}
      </div>
    </div>
  </div>;
}
