import type { ReactNode, Ref, RefObject } from "react";
import { PanelLeft } from "lucide-react";
import {
  Group,
  Panel,
  Separator,
  useDefaultLayout,
  type PanelImperativeHandle,
} from "react-resizable-panels";
import { t } from "../i18n";
import "./resizable-workspace.css";

/** Layout stays mounted while resizing is off, preserving readers and editors. */
export function ResizableWorkspace({
  id,
  enabled,
  children,
  firstSize,
  secondSize,
  firstMin = "20%",
  secondMin = "30%",
  firstMax = "75%",
  secondMax,
  firstCollapsible = false,
  firstPanelRef,
  onFirstCollapse,
}: {
  id: string;
  enabled: boolean;
  children: [ReactNode, ReactNode];
  firstSize?: number | string;
  secondSize?: number | string;
  firstMin?: number | string;
  secondMin?: number | string;
  firstMax?: number | string;
  secondMax?: number | string;
  firstCollapsible?: boolean;
  firstPanelRef?: Ref<PanelImperativeHandle | null>;
  onFirstCollapse?: (collapsed: boolean) => void;
}) {
  const { defaultLayout, onLayoutChanged } = useDefaultLayout({
    id: `workspace:${id}`,
    onlySaveAfterUserInteractions: !firstCollapsible,
  });
  return (
    <Group
      id={`workspace-${id}`}
      className="splitWorkspace"
      data-resizable={enabled}
      disabled={!enabled}
      // Document-wide cursor overrides compete with native WKWebView cursors.
      disableCursor
      defaultLayout={defaultLayout}
      onLayoutChange={
        firstCollapsible
          ? (layout) => onFirstCollapse?.(layout[`${id}-first`] === 0)
          : undefined
      }
      onLayoutChanged={(layout, meta) => {
        if (enabled) onLayoutChanged(layout, meta);
      }}
    >
      <Panel
        id={`${id}-first`}
        className="splitPane"
        defaultSize={firstSize}
        groupResizeBehavior={
          typeof firstSize === "number"
            ? "preserve-pixel-size"
            : "preserve-relative-size"
        }
        collapsible={firstCollapsible}
        collapsedSize={0}
        collapsedThreshold={48}
        panelRef={firstPanelRef}
        minSize={firstMin}
        maxSize={firstMax}
      >
        {children[0]}
      </Panel>
      <Separator
        className="workspaceSeparator"
        aria-label={t("resizableWorkspace.resizeWorkspace")}
        title={t("resizableWorkspace.dragToResizeDoubleClick")}
      />
      <Panel
        id={`${id}-second`}
        className="splitPane"
        defaultSize={secondSize}
        groupResizeBehavior={
          typeof secondSize === "number"
            ? "preserve-pixel-size"
            : "preserve-relative-size"
        }
        minSize={secondMin}
        maxSize={secondMax ?? (firstCollapsible ? "100%" : "80%")}
      >
        {children[1]}
      </Panel>
    </Group>
  );
}

/** The button that folds a collapsible first panel away and back, as a
 * Mac app's sidebar button does. */
export function WorkspaceSidebarToggle({
  panel,
  collapsed,
  controls,
  onToggle,
}: {
  panel: RefObject<PanelImperativeHandle | null>;
  collapsed: boolean;
  /** The id of the first panel's content. */
  controls: string;
  onToggle?: () => void;
}) {
  return (
    <button
      type="button"
      className="workspaceSidebarToggle"
      aria-label={t("common.toggleSidebar")}
      aria-expanded={!collapsed}
      aria-controls={controls}
      title={t("common.toggleSidebar")}
      onClick={() => {
        onToggle?.();
        if (panel.current?.isCollapsed()) panel.current.expand();
        else panel.current?.collapse();
      }}
    >
      <PanelLeft size={18} strokeWidth={1.75} aria-hidden />
    </button>
  );
}
