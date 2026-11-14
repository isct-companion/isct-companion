import {
  nativeOs,
  type NativeOs,
  type SchoolWebBounds,
} from "./desktop-bridge";

type Rect = Pick<DOMRect, "left" | "top" | "width" | "height">;
type Viewport = Pick<
  Window,
  | "innerWidth"
  | "innerHeight"
  | "outerWidth"
  | "outerHeight"
  | "devicePixelRatio"
>;

const chromeInset = (outer: number, inner: number, maximum: number) =>
  Math.min(Math.max(outer - inner, 0), maximum);

/**
 * On Windows the bounds are physical pixels of the client area: the DOM
 * rectangle times the page's own `devicePixelRatio`. WebView2's CSS pixel is
 * not always the window's logical pixel (a 125% laptop panel whose page
 * renders at another scale, a window moved between monitors), and Tauri turns
 * logical bounds into physical ones with the window's scale factor, so a
 * logical rectangle lands shrunk or grown toward the window's top-left
 * (`school_web::apply_bounds` takes these as physical). Edges are rounded,
 * not the width, so the view meets the pane exactly.
 *
 * On macOS they stay logical points: WKWebView's child origin includes the
 * title-bar inset, which must be added back here.
 */
export function schoolWebBounds(
  rect: Rect,
  viewport: Viewport,
  os: NativeOs | undefined = nativeOs(),
): SchoolWebBounds {
  if (os !== "macos") {
    const scale =
      Number.isFinite(viewport.devicePixelRatio) && viewport.devicePixelRatio > 0
        ? viewport.devicePixelRatio
        : 1;
    const x = Math.round(rect.left * scale);
    const y = Math.round(rect.top * scale);
    return {
      x,
      y,
      width: Math.max(1, Math.round((rect.left + rect.width) * scale) - x),
      height: Math.max(1, Math.round((rect.top + rect.height) * scale) - y),
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
