import { Activity, createContext, use, useState, type ReactNode } from "react";

const PageLayer = createContext<HTMLElement | null>(null);

/** All page overlays, including Radix portals, mount in this retained layer. */
export const usePageLayer = () => use(PageLayer) ?? document.body;

/**
 * One retained page. `<Activity>` hides a page by hiding its own DOM, so a
 * popover, menu or modal portaled to `document.body` would stay on screen
 * with its effects disconnected: it could not close or move until the page
 * came back. Each page therefore owns an overlay layer inside its Activity,
 * and every portal on the page targets it, so overlays hide and
 * return with their page. A pane that retains its own hidden views (the
 * course slots on Lectures) nests one per view for the same reason.
 */
export function KeptPage({
  visible,
  children,
}: {
  visible: boolean;
  children: ReactNode;
}) {
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
    <Activity mode={visible ? "visible" : "hidden"}>
      <PageLayer value={layer}>{children}</PageLayer>
      <div className="pageLayerHost" ref={attach} />
    </Activity>
  );
}
