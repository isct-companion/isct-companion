import type { SchoolWebBounds } from "./desktop-bridge";

type Rect = Pick<DOMRect, "left" | "top" | "width" | "height">;
type Viewport = Pick<
  Window,
  "innerWidth" | "innerHeight" | "outerWidth" | "outerHeight"
>;

const chromeInset = (outer: number, inner: number, maximum: number) =>
  Math.min(Math.max(outer - inner, 0), maximum);

/**
 * DOM rectangles are relative to the main webview's content area. Tauri child
 * webviews are positioned relative to the native window, so native borders and
 * the title bar must be added back to keep the child below app-owned controls.
 */
export function schoolWebBounds(
  rect: Rect,
  viewport: Viewport,
): SchoolWebBounds {
  const sideInset =
    chromeInset(viewport.outerWidth, viewport.innerWidth, 64) / 2;
  const measuredTopInset = chromeInset(
    viewport.outerHeight,
    viewport.innerHeight,
    160,
  );
  // WKWebView reports equal inner/outer heights in a Tauri window even though
  // child views are positioned from the top of the native title bar.
  // This helper is used only for native embedded webviews; frameless windows
  // still report a measurable zero-height origin and tolerate the empty inset.
  const nativeTitleBarFallback = measuredTopInset === 0 ? 28 : 0;
  const topInset = Math.max(
    Math.max(measuredTopInset, nativeTitleBarFallback) - sideInset,
    0,
  );
  return {
    x: Math.round(rect.left + sideInset),
    y: Math.round(rect.top + topInset),
    width: Math.max(1, Math.round(rect.width)),
    height: Math.max(1, Math.round(rect.height)),
  };
}
