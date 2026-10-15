import type { AnchorHTMLAttributes, MouseEvent } from "react";
import { getDesktopBridge } from "../platform/desktop-bridge";

/**
 * Opens a URL in the system browser. Inside the desktop app a plain
 * `target="_blank"` link does nothing, so this goes through the native opener.
 */
export function openExternalUrl(url: string) {
  const bridge = getDesktopBridge();
  if (bridge) return bridge.openExternal(url);
  window.open(url, "_blank", "noopener,noreferrer");
  return Promise.resolve();
}

/** An `<a>` that opens its `href` in the system browser. */
export function ExternalLink({
  href,
  onClick,
  ...props
}: AnchorHTMLAttributes<HTMLAnchorElement> & { href: string }) {
  return (
    <a
      {...props}
      href={href}
      rel="noreferrer"
      onClick={(event: MouseEvent<HTMLAnchorElement>) => {
        onClick?.(event);
        if (event.defaultPrevented) return;
        event.preventDefault();
        void openExternalUrl(href).catch(() => undefined);
      }}
    />
  );
}
