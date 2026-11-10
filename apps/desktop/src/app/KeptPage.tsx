import { Activity, createContext, use, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

const PageLayer = createContext<HTMLElement | null>(null);
const TopPageLayer = createContext<HTMLElement | null>(null);

/** All page overlays, including Radix portals, mount in this retained layer. */
export const usePageLayer = () => use(PageLayer) ?? document.body;

/** The outermost page's layer, for overlays that cover the whole window. */
export const useTopPageLayer = () => use(TopPageLayer) ?? document.body;

/**
 * One retained page. `<Activity>` hides a page by hiding its own DOM, so a
 * popover, menu or modal portaled to `document.body` would stay on screen
 * with its effects disconnected: it could not close or move until the page
 * came back. Each page therefore owns an overlay layer inside its Activity,
 * and every portal on the page targets it, so overlays hide and
 * return with their page. A pane that retains its own hidden views (the
 * course slots on Lectures) nests one per view for the same reason.
 *
 * A nested layer is hosted inside the enclosing page's layer, not beside its
 * view: the view may sit in a pane with `contain` or a transform, which would
 * confine and clip anything positioned against the viewport. It is hidden
 * with its view instead.
 */
export function KeptPage({
  visible,
  children,
}: {
  visible: boolean;
  children: ReactNode;
}) {
  const parent = use(PageLayer);
  const top = use(TopPageLayer);
  // A node that exists from the first render, so no overlay ever mounts
  // into body first and has to move.
  const [layer] = useState(() => {
    const node = document.createElement("div");
    node.className = "pageLayer";
    return node;
  });
  const [attach] = useState(
    () => (host: HTMLDivElement | null) => host?.append(layer),
  );
  return (
    <>
      <Activity mode={visible ? "visible" : "hidden"}>
        <PageLayer value={layer}>
          <TopPageLayer value={top ?? layer}>{children}</TopPageLayer>
        </PageLayer>
        {!parent && <div className="pageLayerHost" ref={attach} />}
      </Activity>
      {parent &&
        createPortal(
          <div className="pageLayerHost" hidden={!visible} ref={attach} />,
          parent,
        )}
    </>
  );
}
