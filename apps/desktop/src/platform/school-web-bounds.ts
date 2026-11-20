import {
  nativeOs,
  type NativeOs,
  type SchoolWebBounds,
} from "./desktop-bridge";

type Rect = Pick<DOMRect, "left" | "top" | "width" | "height">;
type Viewport = Pick<Window, "devicePixelRatio">;

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
 * On macOS they are logical points from the top of the content; the host
 * adds the window's own title bar (`school_web::placement`), which the page
 * cannot measure (WKWebView reports equal inner and outer heights).
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
  return {
    x: Math.round(rect.left),
    y: Math.round(rect.top),
    width: Math.max(1, Math.round(rect.width)),
    height: Math.max(1, Math.round(rect.height)),
  };
}
