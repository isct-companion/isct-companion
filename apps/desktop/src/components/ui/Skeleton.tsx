import { useRef } from "react";
import { t } from "../../i18n";
import { cx } from "./layout";

/**
 * Grey placeholders in the shape of what is coming (slotledger
 * approach/loading-skeleton): rows for a list, blocks across five columns
 * for a week. Shown only while a view has nothing at all to draw; cached
 * content is drawn at once instead.
 */
export function Skeleton({
  shape = "row",
  count = 3,
  className,
}: {
  shape?: "row" | "week";
  count?: number;
  className?: string;
}) {
  return (
    <div
      className={cx("skeleton", className)}
      data-shape={shape}
      role="status"
      aria-label={t("Loading…")}
    >
      {shape === "week"
        ? Array.from({ length: 5 }, (_, column) => (
            <span key={column} className="skeletonColumn">
              {Array.from({ length: count }, (_, block) => (
                <i key={block} style={{ "--block": (block + column) % 3 } as React.CSSProperties} />
              ))}
            </span>
          ))
        : Array.from({ length: count }, (_, row) => <i key={row} />)}
    </div>
  );
}

/**
 * The class that fades content in when it replaces a skeleton; nothing when
 * the content was there from the start, so cached views never flash.
 */
export function useReveal(loading: boolean): string | undefined {
  const shown = useRef(false);
  if (loading) shown.current = true;
  return !loading && shown.current ? "uiReveal" : undefined;
}
