import { UiSelect } from "../../components/ui";
import { ReaderTools } from "./ReaderPane";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { desktopBridge } from "../../app/native";
import { nativeOs } from "../../platform/desktop-bridge";
import type {
  PdfPlan,
  PdfSearchHit,
  PdfTextRun,
  PdfViewport,
} from "../../platform/desktop-bridge";
import { t } from "../../i18n";
import { LmsIcon } from "./LmsIcon";

// The shell renders tiles with PDFium (slotledger aqchay, crates/pdf-view):
// this view asks for a plan per viewport and draws the tiles it names, so a
// 35-megapixel scan costs the same to scroll as a slide deck.
const tileOrigin =
  nativeOs() === "windows" ? "http://pdftile.localhost" : "pdftile://localhost";
const ZOOMS = [0.5, 0.75, 1, 1.25, 1.5, 2, 3];
const clampZoom = (value: number) => Math.max(0.5, Math.min(3, value));

/** Text runs of a page as transparent spans, stretched to their boxes so a
 * selection lines up with the printed words. */
function TextLayer({ runs, scale }: { runs: PdfTextRun[]; scale: number }) {
  const [measure] = useState(() =>
    document.createElement("canvas").getContext("2d"),
  );
  return (
    <div className="lmsPdfTextLayer">
      {runs.map((run, index) => {
        const size = Math.max(1, run.rect.height * scale);
        let stretch = 1;
        if (measure) {
          measure.font = `${size}px sans-serif`;
          const natural = measure.measureText(run.text).width;
          if (natural > 0) stretch = (run.rect.width * scale) / natural;
        }
        return (
          <span
            key={index}
            style={{
              left: run.rect.left * scale,
              top: run.rect.top * scale,
              fontSize: size,
              transform: `scaleX(${stretch})`,
            }}
          >
            {run.text}
          </span>
        );
      })}
    </div>
  );
}

/** Continuous reading over PDFium tiles; only pages near the viewport have
 * any, and the shell renders the ones nearest the middle first. */
export function PdfDocument({
  path,
  title,
  fileToolbar,
}: {
  /** A file the shell already lets this page read. */
  path: string;
  title: string;
  fileToolbar?: ReactNode;
}) {
  const pdf = desktopBridge?.pdf;
  const host = useRef<HTMLDivElement>(null);
  const pages = useRef<HTMLDivElement>(null);
  const [doc, setDoc] = useState<{ id: number; pageCount: number }>();
  const [plan, setPlan] = useState<PdfPlan>();
  const [error, setError] = useState("");
  const [zoom, setZoom] = useState(1);
  const zoomValue = useRef(1);
  zoomValue.current = zoom;
  const [pageInput, setPageInput] = useState("1");
  const [texts, setTexts] = useState<Record<number, PdfTextRun[]>>({});
  const asked = useRef(new Set<number>());
  const [searchOpen, setSearchOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [hits, setHits] = useState<{ query: string; items: PdfSearchHit[] }>();
  const [hitIndex, setHitIndex] = useState(0);
  const [finding, setFinding] = useState(false);
  // Where the middle of the viewport was, as a share of the content, so a
  // zoom keeps the same place in view.
  const anchor = useRef<number | undefined>(undefined);
  const planSeq = useRef(0);
  const frame = useRef(0);

  useEffect(() => {
    if (!pdf) return setError(t("lms.searchUnavailable"));
    let disposed = false;
    let opened: number | undefined;
    setDoc(undefined);
    setPlan(undefined);
    setTexts({});
    asked.current = new Set();
    setHits(undefined);
    pdf.open(path).then(
      (value) => {
        if (disposed) return void pdf.close(value.id);
        opened = value.id;
        setDoc(value);
      },
      (e) => !disposed && setError(String(e)),
    );
    return () => {
      // Also when a kept page hides the reader: its tiles go with it.
      disposed = true;
      setDoc(undefined);
      setPlan(undefined);
      if (opened !== undefined) void pdf.close(opened);
    };
  }, [pdf, path]);

  const viewport = useCallback((): PdfViewport | undefined => {
    const element = host.current;
    if (!element) return undefined;
    return {
      width: element.clientWidth,
      height: element.clientHeight,
      scrollTop: element.scrollTop,
      zoom: zoomValue.current,
      devicePixelRatio: window.devicePixelRatio || 1,
    };
  }, []);

  const replan = useCallback(() => {
    cancelAnimationFrame(frame.current);
    frame.current = requestAnimationFrame(() => {
      const current = viewport();
      if (!pdf || !doc || !current) return;
      const seq = ++planSeq.current;
      pdf.plan(doc.id, current).then(
        (next) => {
          if (seq !== planSeq.current) return;
          const element = host.current;
          if (element && anchor.current !== undefined) {
            // A zoom changed the content height: put the anchor back in the
            // middle, then plan again for where that lands.
            element.scrollTop =
              anchor.current * next.contentHeight - element.clientHeight / 2;
            anchor.current = undefined;
            return replan();
          }
          setPlan(next);
          setPageInput(String(next.currentPage + 1));
        },
        (e) => setError(String(e)),
      );
    });
  }, [pdf, doc, viewport]);

  useEffect(() => {
    replan();
    const element = host.current;
    if (!element) return;
    const observer = new ResizeObserver(() => replan());
    observer.observe(element);
    element.addEventListener("scroll", replan, { passive: true });
    return () => {
      cancelAnimationFrame(frame.current);
      observer.disconnect();
      element.removeEventListener("scroll", replan);
    };
  }, [replan]);

  const changeZoom = useCallback(
    (value: number) => {
      const element = host.current;
      const next = clampZoom(value);
      if (element && plan && next !== zoomValue.current)
        anchor.current =
          (element.scrollTop + element.clientHeight / 2) / plan.contentHeight;
      zoomValue.current = next;
      setZoom(next);
      replan();
    },
    [plan, replan],
  );

  // Text for the pages in the plan, fetched once each.
  useEffect(() => {
    if (!pdf || !doc || !plan) return;
    for (const page of plan.pages) {
      if (asked.current.has(page.index)) continue;
      asked.current.add(page.index);
      pdf.text(doc.id, page.index).then(
        (runs) => setTexts((all) => ({ ...all, [page.index]: runs })),
        () => undefined,
      );
    }
  }, [pdf, doc, plan]);

  // Pinch, Ctrl-wheel and two-finger zoom preview as a CSS scale around the
  // pointer, then commit one zoom when the gesture settles.
  useLayoutEffect(() => {
    const element = host.current;
    if (!element) return;
    let base = zoomValue.current,
      pending = base,
      gesturing = false,
      touchDistance = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const commit = () => {
      clearTimeout(timer);
      timer = undefined;
      const surface = pages.current;
      if (surface) {
        surface.style.transform = "";
        surface.style.transformOrigin = "";
      }
      base = pending;
      changeZoom(pending);
    };
    const preview = (value: number, x: number, y: number) => {
      pending = clampZoom(value);
      const surface = pages.current;
      if (surface) {
        if (!surface.style.transform) {
          const bounds = surface.getBoundingClientRect();
          surface.style.transformOrigin = `${x - bounds.left}px ${y - bounds.top}px`;
        }
        surface.style.transform = `scale(${pending / zoomValue.current})`;
      }
      clearTimeout(timer);
      timer = setTimeout(commit, 140);
    };
    const start = (event: Event) => {
      event.preventDefault();
      gesturing = true;
      base = zoomValue.current;
      clearTimeout(timer);
    };
    const change = (event: Event) => {
      event.preventDefault();
      const gesture = event as Event & {
        scale: number;
        clientX: number;
        clientY: number;
      };
      preview(base * gesture.scale, gesture.clientX, gesture.clientY);
      clearTimeout(timer);
    };
    const end = (event: Event) => {
      event.preventDefault();
      gesturing = false;
      commit();
    };
    const wheel = (event: WheelEvent) => {
      if (!event.ctrlKey || gesturing) return;
      event.preventDefault();
      // A new wheel gesture starts from the zoom shown now, which the zoom
      // menu may have changed since this effect ran.
      const from = timer ? pending : zoomValue.current;
      preview(
        from * Math.exp(-event.deltaY * 0.01),
        event.clientX,
        event.clientY,
      );
    };
    const distance = (touches: TouchList) =>
      Math.hypot(
        touches[0].clientX - touches[1].clientX,
        touches[0].clientY - touches[1].clientY,
      );
    const touchStart = (event: TouchEvent) => {
      if (event.touches.length === 2 && !gesturing) {
        base = zoomValue.current;
        touchDistance = distance(event.touches);
      }
    };
    const touchMove = (event: TouchEvent) => {
      if (event.touches.length !== 2 || !touchDistance || gesturing) return;
      event.preventDefault();
      preview(
        (base * distance(event.touches)) / touchDistance,
        (event.touches[0].clientX + event.touches[1].clientX) / 2,
        (event.touches[0].clientY + event.touches[1].clientY) / 2,
      );
      clearTimeout(timer);
    };
    const touchEnd = () => {
      if (touchDistance && !gesturing) {
        touchDistance = 0;
        commit();
      }
    };
    element.addEventListener("gesturestart", start, { passive: false });
    element.addEventListener("gesturechange", change, { passive: false });
    element.addEventListener("gestureend", end, { passive: false });
    element.addEventListener("wheel", wheel, { passive: false });
    element.addEventListener("touchstart", touchStart, { passive: true });
    element.addEventListener("touchmove", touchMove, { passive: false });
    element.addEventListener("touchend", touchEnd);
    element.addEventListener("touchcancel", touchEnd);
    return () => {
      clearTimeout(timer);
      element.removeEventListener("gesturestart", start);
      element.removeEventListener("gesturechange", change);
      element.removeEventListener("gestureend", end);
      element.removeEventListener("wheel", wheel);
      element.removeEventListener("touchstart", touchStart);
      element.removeEventListener("touchmove", touchMove);
      element.removeEventListener("touchend", touchEnd);
      element.removeEventListener("touchcancel", touchEnd);
    };
  }, [changeZoom]);

  const scrollToPage = async (page: number, offset = 0) => {
    const current = viewport();
    if (!pdf || !doc || !current || !host.current) return;
    const top = await pdf.pageTop(doc.id, current, page);
    host.current.scrollTop = top + offset;
  };
  const goToPage = () => {
    if (!doc) return;
    const number = Math.max(1, Math.min(doc.pageCount, Number(pageInput) || 1));
    setPageInput(String(number));
    void scrollToPage(number - 1);
  };
  const showHit = (items: PdfSearchHit[], index: number) => {
    const hit = items[index];
    if (!hit || !plan) return;
    setHitIndex(index);
    const first = hit.rects[0];
    // A little above the match, so it is not under the header.
    void scrollToPage(hit.page, first ? first.top * plan.scale - 80 : 0);
  };
  const findNext = async () => {
    const query = search.trim();
    if (!pdf || !doc || !query) return;
    if (hits?.query === query) {
      if (hits.items.length)
        showHit(hits.items, (hitIndex + 1) % hits.items.length);
      return;
    }
    setFinding(true);
    try {
      const items = await pdf.search(doc.id, query);
      setHits({ query, items });
      showHit(items, 0);
    } catch (e) {
      setError(String(e));
    } finally {
      setFinding(false);
    }
  };
  const searchStatus = !hits
    ? ""
    : hits.items.length
      ? `${hitIndex + 1} / ${hits.items.length}`
      : t("lms.noMatches");

  return (
    <div className="lmsPdfReader">
      <ReaderTools>
        <div className="lmsPdfHeader">
          <div
            className="lmsPdfControls"
            role="toolbar"
            aria-label={t("lms.pdfTools")}
          >
            <div className="lmsPdfPagination">
              <input
                aria-label={t("lms.page")}
                type="number"
                min="1"
                max={doc?.pageCount || 1}
                value={pageInput}
                onChange={(e) => setPageInput(e.target.value)}
                onBlur={goToPage}
                onKeyDown={(e) => {
                  if (e.key === "Enter") goToPage();
                }}
              />
              <span>/ {doc?.pageCount || "—"}</span>
            </div>
            <div className="lmsPdfZoom">
              <button
                aria-label={t("common.zoomOut")}
                title={t("common.zoomOut")}
                disabled={zoom <= 0.5}
                onClick={() => changeZoom(zoom - 0.25)}
              >
                <LmsIcon name="minus" size={14} />
              </button>
              <UiSelect
                aria-label={t("lms.zoom")}
                value={String(zoom)}
                onChange={(value) => changeZoom(Number(value))}
                data={[...new Set([...ZOOMS, zoom])]
                  .sort((a, b) => a - b)
                  .map((value) => ({
                    value: String(value),
                    label:
                      value === 1
                        ? t("lms.fitWidth")
                        : `${Math.round(value * 100)}%`,
                  }))}
              />
              <button
                aria-label={t("common.zoomIn")}
                title={t("common.zoomIn")}
                disabled={zoom >= 3}
                onClick={() => changeZoom(zoom + 0.25)}
              >
                <LmsIcon name="plus" size={14} />
              </button>
            </div>
            <button
              className="lmsPdfFindToggle"
              aria-label={t("lms.findInDocument")}
              title={t("lms.findInDocument")}
              aria-expanded={searchOpen}
              onClick={() => setSearchOpen((value) => !value)}
            >
              <LmsIcon name="search" size={15} />
            </button>
          </div>
          {fileToolbar}
        </div>
      </ReaderTools>
      {searchOpen && (
        <form
          className="lmsPdfSearch"
          role="search"
          onSubmit={(event) => {
            event.preventDefault();
            void findNext();
          }}
        >
          <input
            aria-label={t("lms.findInDocument")}
            placeholder={t("lms.findInDocument")}
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
          <span role="status">{searchStatus}</span>
          <button disabled={finding || !doc || !search.trim()}>
            {t(finding ? "lms.searching" : "lms.findNext")}
          </button>
          <button
            type="button"
            aria-label={t("lms.closeSearch")}
            onClick={() => {
              setSearchOpen(false);
              setHits(undefined);
            }}
          >
            ×
          </button>
        </form>
      )}
      <div className="lmsPdfDocument" ref={host} aria-label={title}>
        {error ? (
          <p role="alert">{error}</p>
        ) : !plan || !doc ? (
          <p role="status">{t("lms.loadingPDF")}</p>
        ) : (
          <div
            className="lmsPdfPages"
            ref={pages}
            style={{ width: plan.contentWidth, height: plan.contentHeight }}
          >
            {plan.pages.map((page) => (
              <div
                key={page.index}
                className="lmsPdfSheet"
                data-pdf-page={page.index + 1}
                style={{
                  left: page.rect.left,
                  top: page.rect.top,
                  width: page.rect.width,
                  height: page.rect.height,
                }}
              >
                {page.tiles.map((tile) => (
                  <img
                    key={`${tile.key.level}/${tile.key.col}/${tile.key.row}`}
                    className="lmsPdfTile"
                    data-preview={tile.preview || undefined}
                    alt=""
                    draggable={false}
                    decoding="async"
                    src={`${tileOrigin}/${doc.id}/${tile.key.page}/${tile.key.level}/${tile.key.col}/${tile.key.row}`}
                    onError={(event) => {
                      event.currentTarget.style.visibility = "hidden";
                    }}
                    style={{
                      left: tile.rect.left,
                      top: tile.rect.top,
                      width: tile.rect.width,
                      height: tile.rect.height,
                    }}
                  />
                ))}
                {hits?.items
                  .map((hit, index) => ({ hit, index }))
                  .filter(({ hit }) => hit.page === page.index)
                  .flatMap(({ hit, index }) =>
                    hit.rects.map((rect, part) => (
                      <i
                        key={`${index}/${part}`}
                        className="lmsPdfHit"
                        data-current={index === hitIndex || undefined}
                        style={{
                          left: rect.left * plan.scale,
                          top: rect.top * plan.scale,
                          width: rect.width * plan.scale,
                          height: rect.height * plan.scale,
                        }}
                      />
                    )),
                  )}
                {texts[page.index] && (
                  <TextLayer runs={texts[page.index]} scale={plan.scale} />
                )}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
