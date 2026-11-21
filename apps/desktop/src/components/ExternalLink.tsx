import type { AnchorHTMLAttributes, MouseEvent } from "react";
import { openExternalUrl } from "../platform/external-url";

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
