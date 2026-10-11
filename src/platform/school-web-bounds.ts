import type { SchoolWebBounds } from "./desktop-bridge";

type Rect = Pick<DOMRect, "left" | "top" | "width" | "height">;
type Viewport = Pick<
  Window,
  "innerWidth" | "innerHeight" | "outerWidth" | "outerHeight" | "navigator"
>;

const chromeInset = (outer: number, inner: number, maximum: number) =>
  Math.min(Math.max(outer - inner, 0), maximum);

/**
 * DOM rectangles and Tauri child-webview bounds share client-area coordinates
 * on Windows and Linux. WKWebView is the exception: its child origin includes
 * the macOS title-bar inset, which must be added back here.
 */
export function schoolWebBounds(
  rect: Rect,
  viewport: Viewport,
): SchoolWebBounds {
  const platform = `${viewport.navigator.platform} ${viewport.navigator.userAgent}`;
  if (!/Mac/i.test(platform)) {
    return {
      x: Math.round(rect.left),
      y: Math.round(rect.top),
      width: Math.max(1, Math.round(rect.width)),
      height: Math.max(1, Math.round(rect.height)),
    };
  }

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
