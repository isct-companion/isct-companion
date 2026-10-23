import { MantineThemeProvider } from "@mantine/core";
import {
  Activity,
  createContext,
  use,
  useMemo,
  useState,
  type ReactNode,
} from "react";

const PageLayer = createContext<HTMLElement | null>(null);

/** Where a hand-written portal on this page mounts; Mantine's already do. */
export const usePageLayer = () => use(PageLayer) ?? document.body;

/**
 * One retained page. `<Activity>` hides a page by hiding its own DOM, so a
 * popover, menu or modal portaled to `document.body` would stay on screen
 * with its effects disconnected: it could not close or move until the page
 * came back. Each page therefore owns an overlay layer inside its Activity,
 * and every Mantine Portal on the page targets it, so overlays hide and
 * return with their page.
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
  const theme = useMemo(
    () => ({ components: { Portal: { defaultProps: { target: layer } } } }),
    [layer],
  );
  return (
    <Activity mode={visible ? "visible" : "hidden"}>
      <MantineThemeProvider theme={theme}>
        <PageLayer value={layer}>{children}</PageLayer>
      </MantineThemeProvider>
      <div className="pageLayerHost" ref={attach} />
    </Activity>
  );
}
