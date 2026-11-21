import { useEffect, useRef } from "react";

/**
 * WebKit keeps a detached element's decoded image or media player until its
 * garbage collector runs, which can be long after the viewer closed. Dropping
 * the source on unmount releases them at once, like the PDF canvases.
 * React can run the cleanup while the element stays mounted (StrictMode's
 * extra effect pass in development, or a new `src`), so each run puts the
 * source back; otherwise the element would stay empty.
 */
export function useReleaseSource<
  T extends HTMLImageElement | HTMLMediaElement,
>(src: string) {
  const element = useRef<T>(null);
  useEffect(() => {
    const current = element.current;
    if (current && current.getAttribute("src") !== src) {
      current.setAttribute("src", src);
      if (current instanceof HTMLMediaElement) current.load();
    }
    return () => {
      if (!current) return;
      if (current instanceof HTMLMediaElement) current.pause();
      current.removeAttribute("src");
      if (current instanceof HTMLMediaElement) current.load();
    };
  }, [src]);
  return element;
}
